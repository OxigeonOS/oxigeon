-- mudlib/daemons/cache_d.lua — Game state, tiered by how much you'd mind losing it.
--
-- Evennia writes every attribute change straight to the database, and that is
-- where its performance goes. A document write here costs ~101 microseconds
-- against ~2.7 for an in-memory one, synchronously on the game thread, so the
-- answer is not a faster store — it is writing less often. This daemon
-- collapses N changes into one write per scope per interval.
--
--   memory         never written. Combat state, aggro, sub-minute cooldowns.
--   write_behind   flushed on a ticker, on disconnect, and on shutdown.
--   write_through  written immediately. Daily gates, admin actions.
--
-- BETWEEN FLUSHES, MEMORY IS THE AUTHORITY. That is the one thing to know
-- before touching this file. Despite the name, dropping a dirty scope loses
-- data — which is why there is no `clear_all`, and why `evict` flushes before
-- it drops. The three destructive calls are `flush` (write), `evict` (write
-- then forget) and `drop` (forget, explicitly discarding).
--
-- Durability contract: you may lose up to a namespace's `flush_seconds` on a
-- hard crash. On a clean shutdown you lose nothing.
--
-- Storage is (namespace, scope, key), which maps onto one document collection,
-- one document id, and one top-level field of that document.
--
-- Exposes:
--   DAEMON.cache.define(ns, spec)
--   DAEMON.cache.get/set/delete/incr/has(ns, scope, key[, ...])
--   DAEMON.cache.get_scope/copy_scope/edit/set_scope/merge_scope/clear_scope/keys/exists
--   DAEMON.cache.flush/flush_namespace/flush_owner/flush_all/tick/verify
--   DAEMON.cache.evict/evict_owner/drop/write_offline/preload
--   DAEMON.cache.stats/inspect/namespaces/spec
--
-- See docs/src/lua-api/state-cache.md.

local jsonsafe = require('lib.jsonsafe')
local persist  = require('lib.persist')

local M = {}

-- ─── Logging ─────────────────────────────────────────────────────────────────

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.error, message)
    end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.warn, message)
    end
end

-- ─── Tunables ────────────────────────────────────────────────────────────────

local function cfg(key, default)
    local ok, v = pcall(config, key)
    if ok and type(v) == "number" then return v end
    return default
end

-- Ceilings from the driver's [documents] block. Kept in step with
-- DocumentLimits::default(); the byte figure only has to be conservative.
local MAX_DOC_BYTES  = 65536
local SOFT_DOC_BYTES = 49152
local MAX_ID_LEN     = 128

local TIERS = { memory = true, write_behind = true, write_through = true }

-- ─── State that survives a hot reload ────────────────────────────────────────
--
-- A table declared at module level dies when this file is reloaded, taking
-- every dirty scope with it. `_persistent_store` is a VM global, and hot
-- reload only replaces `package.loaded`, so state parked there survives.
-- Registered namespaces live in it too — otherwise reloading this daemon would
-- forget every namespace its tenants had declared, and their `define` calls
-- would not re-run.

local ROOT_KEY = "cache_d"
local S = nil

local function root()
    if S then return S end
    S = persist.root(ROOT_KEY, 1, function()
        return {
            specs = {},
            data  = {},   -- [ns][scope] = { key = value }
            meta  = {},   -- [ns][scope] = bookkeeping, see new_meta
            seq   = 0,    -- monotonic, orders dirty scopes without needing a clock
            clock = 0,    -- last known monotonic time
            stats = {
                db_gets = 0, db_puts = 0, db_deletes = 0,
                rejected_writes = 0, oversize_warnings = 0,
                flush_failures = 0, poisoned = 0, loads_failed = 0,
            },
        }
    end)
    return S
end

--- Monotonic seconds. Wall clock (`os_time`) can step backwards under NTP, and
--- scheduling on it would make flushes early or late for no reason; anything a
--- *player* perceives as a deadline uses `os_time` instead.
---
--- `server_info()` builds a table, so this is far too expensive to call on
--- every `set` — which is the write this daemon exists to make cheap. It is
--- called only where a fresh reading matters, and cached in `S.clock` for
--- everything else. One tick of resolution is exactly what scheduling and
--- idle eviction need.
local function mono()
    local ok, info = pcall(server_info)
    if ok and type(info) == "table" and type(info.uptime_secs) == "number" then
        return info.uptime_secs
    end
    return os_time()
end

local function refresh_clock()
    local s = root()
    s.clock = mono()
    return s.clock
end

--- The cached monotonic reading: a table lookup, not a call into Rust.
local function clock()
    return root().clock
end

local function now_wall()
    return os_time()
end

-- ─── Validation ──────────────────────────────────────────────────────────────

local function valid_ns_name(name)
    return type(name) == "string" and #name >= 1 and #name <= 64
        and name:match("^[a-z][a-z0-9_]*$") ~= nil
end

--- Scopes become document ids, so they are limited to what an id may contain.
--- Numbers are accepted for the common `char_id` case.
local function to_scope(v)
    if type(v) == "number" then
        if v ~= math.floor(v) or v < 0 then return nil end
        return tostring(math.floor(v))
    end
    if type(v) ~= "string" or #v == 0 then return nil end
    if v:match("^[A-Za-z0-9._:%-]+$") == nil then return nil end
    return v
end

local function spec_of(ns, who)
    local s = root().specs[ns]
    if not s then
        log_warn("CACHE_D." .. who .. ": unknown namespace '" .. tostring(ns)
            .. "' — call DAEMON.cache.define first")
    end
    return s
end

-- ─── Scope bookkeeping ───────────────────────────────────────────────────────

local function new_meta()
    return {
        dirty = false, dirty_seq = 0, dirty_at = 0,
        last_flush = 0, last_touch = 0,
        loaded = false, missing = false,
        load_failed = false, next_load = 0,
        expiry = {}, ephemeral = {},
        bytes = 2,
        fails = 0, next_attempt = 0, poisoned = false,
    }
end

local function meta_of(ns, scope)
    local s = root()
    s.meta[ns] = s.meta[ns] or {}
    local m = s.meta[ns][scope]
    if not m then
        m = new_meta()
        s.meta[ns][scope] = m
    end
    return m
end

local function data_of(ns, scope)
    local s = root()
    s.data[ns] = s.data[ns] or {}
    local d = s.data[ns][scope]
    if not d then
        d = {}
        s.data[ns][scope] = d
    end
    return d
end

local function mark_dirty(spec, ns, scope, m)
    if spec.tier == "memory" then return end
    if not m.dirty then
        local s = root()
        s.seq = s.seq + 1
        m.dirty = true
        m.dirty_seq = s.seq
        m.dirty_at = clock()
    end
end

local function recount_bytes(ns, scope, m)
    m.bytes = jsonsafe.estimate_bytes(data_of(ns, scope))
end

-- ─── Loading ─────────────────────────────────────────────────────────────────

local function doc_id(spec, scope)
    return (spec.scope_prefix or "") .. scope
end

--- Would this scope produce a legal document id? Checked when a namespace is
--- defined and again when a scope is first written, because the prefix is
--- fixed but the scope is not.
local function id_too_long(spec, scope)
    return #doc_id(spec, scope) > MAX_ID_LEN
end

--- Bring a scope into memory, if it is not already.
---
--- Three outcomes, and the difference between the last two is the most
--- dangerous distinction in this file:
---   * the document exists      -> adopt it
---   * the document is absent   -> remember that, so repeated reads are free
---   * the read FAILED          -> do NOT remember anything. Treating a failed
---     read as "absent" would let the next flush `db_put` an empty document
---     over the player's real data.
local function ensure_loaded(spec, ns, scope)
    local m = meta_of(ns, scope)
    if m.loaded and not m.load_failed then return m end

    if spec.tier == "memory" then
        data_of(ns, scope)
        m.loaded = true
        return m
    end

    local t = refresh_clock()
    if m.load_failed and t < m.next_load then return m end

    local s = root()
    s.stats.db_gets = s.stats.db_gets + 1
    local ok, rec = pcall(db_get, spec.collection, doc_id(spec, scope))

    if not ok then
        s.stats.loads_failed = s.stats.loads_failed + 1
        m.load_failed = true
        m.next_load = t + 5
        log_error("CACHE_D: could not read " .. ns .. "/" .. scope .. ": " .. tostring(rec)
            .. " — this scope will not be flushed until it loads, so nothing is overwritten")
        return m
    end

    local stored = (type(rec) == "table" and type(rec.data) == "table") and rec.data or nil
    local mem = data_of(ns, scope)

    if stored then
        -- Anything already in memory was written while the load was failing
        -- and is newer, so it wins.
        for k, v in pairs(stored) do
            if mem[k] == nil then mem[k] = v end
        end
        m.missing = false
    else
        m.missing = next(mem) == nil
    end

    m.loaded = true
    m.load_failed = false

    -- Expiry is derived from the values rather than stored beside them, so the
    -- document stays exactly what the tenant put there and is still readable
    -- by hand. Re-deriving on load is safe because an expiry hint can only
    -- ever cause a deletion, never resurrect anything.
    local pruned = false
    if spec.expiry_of then
        local now = now_wall()
        for k, v in pairs(mem) do
            local okx, exp = pcall(spec.expiry_of, k, v)
            if okx and type(exp) == "number" then
                if exp <= now then
                    mem[k] = nil
                    pruned = true
                else
                    m.expiry[k] = exp
                    if spec.min_lifetime > 0 and (exp - now) < spec.min_lifetime then
                        m.ephemeral[k] = true
                    end
                end
            end
        end
    end

    if spec.on_load then
        local okl, err = pcall(spec.on_load, scope, mem)
        if not okl then
            log_error("CACHE_D: on_load for " .. ns .. "/" .. scope .. " failed: " .. tostring(err))
        end
    end

    recount_bytes(ns, scope, m)
    m.last_touch = t
    -- Writing the cleaned version back is worth one flush; otherwise dead keys
    -- would be re-read on every restart forever.
    if pruned then mark_dirty(spec, ns, scope, m) end
    return m
end

--- Has this key passed its expiry? Removes it if so.
local function expired(spec, ns, scope, key, m)
    local exp = m.expiry[key]
    if not exp or exp > now_wall() then return false end
    local d = data_of(ns, scope)
    if d[key] ~= nil then
        d[key] = nil
        m.expiry[key] = nil
        m.ephemeral[key] = nil
        recount_bytes(ns, scope, m)
        mark_dirty(spec, ns, scope, m)
    end
    return true
end

-- ─── Registration ────────────────────────────────────────────────────────────

--- Declare a namespace and its durability policy.
--- @param ns string    lowercase letters, digits, underscores
--- @param spec table   see docs/src/lua-api/state-cache.md
--- @return boolean
function M.define(ns, spec)
    if not valid_ns_name(ns) then
        log_warn("CACHE_D.define: invalid namespace name '" .. tostring(ns)
            .. "' — must match ^[a-z][a-z0-9_]*$")
        return false
    end
    if type(spec) ~= "table" then
        log_warn("CACHE_D.define('" .. ns .. "'): spec must be a table")
        return false
    end

    local tier = spec.tier or "write_behind"
    if not TIERS[tier] then
        log_warn("CACHE_D.define('" .. ns .. "'): unknown tier '" .. tostring(tier)
            .. "' — expected memory, write_behind or write_through")
        return false
    end

    local prefix = spec.scope_prefix or ""
    if type(prefix) ~= "string" or #prefix > 32 then
        log_warn("CACHE_D.define('" .. ns .. "'): scope_prefix must be a string of at most 32 characters")
        return false
    end

    local built = {
        name              = ns,
        tier              = tier,
        collection        = spec.collection or ns,
        scope_prefix      = prefix,
        owner             = spec.owner or "none",
        flush_seconds     = spec.flush_seconds or 30,
        min_lifetime      = spec.min_lifetime or 0,
        evict_after       = spec.evict_after or 0,
        max_scopes        = spec.max_scopes or 4096,
        preload           = spec.preload or false,
        delete_when_empty = spec.delete_when_empty ~= false,
        expiry_of         = spec.expiry_of,
        on_load           = spec.on_load,
    }

    if not valid_ns_name(built.collection) then
        log_warn("CACHE_D.define('" .. ns .. "'): invalid collection name '"
            .. tostring(built.collection) .. "'")
        return false
    end

    local s = root()
    local prev = s.specs[ns]
    if prev then
        -- Re-defining is a legitimate workflow (edit flush_seconds, reload) but
        -- it silently changes durability, so say what moved.
        local changed = {}
        for _, field in ipairs({ "tier", "collection", "scope_prefix", "owner",
                                 "flush_seconds", "min_lifetime", "evict_after" }) do
            if prev[field] ~= built[field] then
                changed[#changed + 1] = field .. " " .. tostring(prev[field])
                    .. " -> " .. tostring(built[field])
            end
        end
        if #changed > 0 then
            log_warn("CACHE_D: namespace '" .. ns .. "' redefined: " .. table.concat(changed, ", "))
        end
    end

    s.specs[ns] = built
    s.data[ns]  = s.data[ns] or {}
    s.meta[ns]  = s.meta[ns] or {}
    return true
end

function M.spec(ns)
    return root().specs[ns]
end

function M.namespaces()
    local out = {}
    for name in pairs(root().specs) do out[#out + 1] = name end
    table.sort(out)
    return out
end

-- ─── Key access ──────────────────────────────────────────────────────────────

--- @return any|nil  the stored value, or nil if absent or expired
function M.get(ns, scope, key)
    local spec = spec_of(ns, "get"); if not spec then return nil end
    local sc = to_scope(scope);      if not sc then return nil end
    if type(key) ~= "string" then
        log_warn("CACHE_D.get('" .. ns .. "'): key must be a string, got " .. type(key))
        return nil
    end

    local m = ensure_loaded(spec, ns, sc)
    m.last_touch = clock()
    if expired(spec, ns, sc, key, m) then return nil end
    return data_of(ns, sc)[key]
end

--- Store a value. Marks the scope dirty; nothing reaches the database until a
--- flush, unless the namespace is write-through.
--- @param opts table|nil  { expires_at = unix_seconds }
--- @return boolean
function M.set(ns, scope, key, value, opts)
    local spec = spec_of(ns, "set"); if not spec then return false end
    local sc = to_scope(scope)
    if not sc then
        log_warn("CACHE_D.set('" .. ns .. "'): invalid scope " .. tostring(scope))
        return false
    end
    if type(key) ~= "string" then
        log_warn("CACHE_D.set('" .. ns .. "'): key must be a string, got " .. type(key)
            .. " — JSON object keys are strings, and a number key would change shape on the way back")
        return false
    end
    if value == nil then
        return M.delete(ns, scope, key)
    end

    local s = root()

    -- Everything that will be written is checked here rather than at flush
    -- time. Discovering an unserializable value inside on_shutdown is the
    -- worst possible moment to discover it.
    if spec.tier ~= "memory" then
        local ok, why = jsonsafe.check(value)
        if not ok then
            s.stats.rejected_writes = s.stats.rejected_writes + 1
            log_error("CACHE_D.set('" .. ns .. "', '" .. sc .. "', '" .. key
                .. "'): refused — " .. tostring(why))
            return false
        end
    end

    if spec.tier ~= "memory" and id_too_long(spec, sc) then
        log_warn("CACHE_D.set('" .. ns .. "'): scope '" .. sc .. "' makes a document id longer than "
            .. MAX_ID_LEN .. " characters")
        return false
    end

    local m  = ensure_loaded(spec, ns, sc)
    local d  = data_of(ns, sc)

    if spec.tier ~= "memory" then
        -- Subtract the whole of what this key cost before, key included.
        -- Counting `#key` on every write instead of only on the first made
        -- `bytes` creep up by a few bytes per write forever; a scope holding
        -- one counter that was updated often would eventually refuse every
        -- write with a size complaint that was not true. `benches/writes.rs`
        -- caught it, because its control counts writes rather than timing them.
        local existing = d[key]
        local was = existing ~= nil and (jsonsafe.estimate_bytes(existing) + #key + 4) or 0
        local est = m.bytes - was + jsonsafe.estimate_bytes(value) + #key + 4
        if est > MAX_DOC_BYTES then
            s.stats.rejected_writes = s.stats.rejected_writes + 1
            log_error("CACHE_D.set('" .. ns .. "', '" .. sc .. "', '" .. key
                .. "'): refused — the scope would be about " .. math.floor(est)
                .. " bytes, over the " .. MAX_DOC_BYTES .. " byte document ceiling")
            return false
        end
        if est > SOFT_DOC_BYTES and m.bytes <= SOFT_DOC_BYTES then
            s.stats.oversize_warnings = s.stats.oversize_warnings + 1
            log_warn("CACHE_D: scope " .. ns .. "/" .. sc .. " is about "
                .. math.floor(est) .. " bytes, approaching the " .. MAX_DOC_BYTES .. " byte ceiling")
        end
        m.bytes = est
    end

    d[key] = value
    m.missing = false
    m.last_touch = clock()

    -- Expiry, and with it the question of whether this entry is worth writing
    -- at all.
    local exp = opts and opts.expires_at
    if not exp and spec.expiry_of then
        local okx, derived = pcall(spec.expiry_of, key, value)
        if okx and type(derived) == "number" then exp = derived end
    end
    m.expiry[key] = exp

    local ephemeral = (opts and opts.ephemeral) or false
    if not ephemeral and exp and spec.min_lifetime > 0 then
        ephemeral = (exp - now_wall()) < spec.min_lifetime
    end
    m.ephemeral[key] = ephemeral or nil

    -- An ephemeral entry will never be written, so it must not make the scope
    -- look like it needs writing.
    if not ephemeral then
        mark_dirty(spec, ns, sc, m)
        if spec.tier == "write_through" then M.flush(ns, sc) end
    end
    return true
end

--- @return boolean  true if the key was there
function M.delete(ns, scope, key)
    local spec = spec_of(ns, "delete"); if not spec then return false end
    local sc = to_scope(scope);         if not sc then return false end
    if type(key) ~= "string" then return false end

    local m = ensure_loaded(spec, ns, sc)
    local d = data_of(ns, sc)
    if d[key] == nil then return false end

    local was_ephemeral = m.ephemeral[key]
    d[key] = nil
    m.expiry[key] = nil
    m.ephemeral[key] = nil
    recount_bytes(ns, sc, m)
    m.last_touch = clock()

    if not was_ephemeral then
        mark_dirty(spec, ns, sc, m)
        if spec.tier == "write_through" then M.flush(ns, sc) end
    end
    return true
end

function M.has(ns, scope, key)
    return M.get(ns, scope, key) ~= nil
end

--- Add to a stored number, in memory. Unlike `db_incr` this never touches the
--- database until the scope is flushed — the point of the write-behind tier.
function M.incr(ns, scope, key, delta)
    local current = M.get(ns, scope, key)
    if current ~= nil and type(current) ~= "number" then
        log_warn("CACHE_D.incr('" .. tostring(ns) .. "', '" .. tostring(key)
            .. "'): holds a " .. type(current) .. ", not a number")
        return nil
    end
    local value = (current or 0) + (delta or 1)
    if not M.set(ns, scope, key, value) then return nil end
    return value
end

-- ─── Scope access ────────────────────────────────────────────────────────────

--- The live table. Read-only by contract: mutating it will not mark the scope
--- dirty, so the change may never be written. Use `edit` to change a scope.
function M.get_scope(ns, scope)
    local spec = spec_of(ns, "get_scope"); if not spec then return nil end
    local sc = to_scope(scope);            if not sc then return nil end
    local m = ensure_loaded(spec, ns, sc)
    m.last_touch = clock()
    return data_of(ns, sc)
end

function M.copy_scope(ns, scope)
    local live = M.get_scope(ns, scope)
    if not live then return nil end
    local tables = require('lib.tables')
    return tables.deepcopy(live)
end

--- Hand the live scope table to `fn` and mark it dirty afterwards, whatever
--- `fn` did. This is the blessed way to change several keys at once — ticking
--- twelve effect durations is one dirty mark and one document write, not
--- twelve of each.
--- @return any  whatever fn returned
function M.edit(ns, scope, fn)
    local spec = spec_of(ns, "edit"); if not spec then return nil end
    local sc = to_scope(scope);       if not sc then return nil end
    if type(fn) ~= "function" then
        log_warn("CACHE_D.edit('" .. ns .. "'): expected a function")
        return nil
    end

    local m = ensure_loaded(spec, ns, sc)
    local ok, result = pcall(fn, data_of(ns, sc))
    if not ok then
        log_error("CACHE_D.edit('" .. ns .. "', '" .. sc .. "') failed: " .. tostring(result))
        return nil
    end

    m.missing = false
    m.last_touch = clock()
    recount_bytes(ns, sc, m)
    mark_dirty(spec, ns, sc, m)
    if spec.tier == "write_through" then M.flush(ns, sc) end
    -- `true` rather than nil when fn returned nothing, so a caller can tell
    -- "it worked and returned nothing" from "it failed".
    if result == nil then return true end
    return result
end

function M.set_scope(ns, scope, tbl)
    if type(tbl) ~= "table" then return false end
    return M.edit(ns, scope, function(live)
        for k in pairs(live) do live[k] = nil end
        for k, v in pairs(tbl) do live[k] = v end
    end) ~= nil
end

function M.merge_scope(ns, scope, tbl)
    if type(tbl) ~= "table" then return false end
    return M.edit(ns, scope, function(live)
        for k, v in pairs(tbl) do live[k] = v end
    end) ~= nil
end

function M.clear_scope(ns, scope)
    local spec = spec_of(ns, "clear_scope"); if not spec then return false end
    local sc = to_scope(scope);              if not sc then return false end
    local m = ensure_loaded(spec, ns, sc)
    local d = data_of(ns, sc)
    for k in pairs(d) do d[k] = nil end
    m.expiry = {}
    m.ephemeral = {}
    recount_bytes(ns, sc, m)
    mark_dirty(spec, ns, sc, m)
    if spec.tier == "write_through" then M.flush(ns, sc) end
    return true
end

function M.keys(ns, scope)
    local live = M.get_scope(ns, scope)
    local out = {}
    if not live then return out end
    for k in pairs(live) do out[#out + 1] = k end
    table.sort(out)
    return out
end

function M.exists(ns, scope)
    local live = M.get_scope(ns, scope)
    return live ~= nil and next(live) ~= nil
end

--- Load every namespace that belongs to this owner, before anything needs it.
--- Lazy loading otherwise puts the first read wherever it happens to fall,
--- often mid-combat; login is a better moment.
function M.preload(scope)
    local sc = to_scope(scope); if not sc then return 0 end
    local n = 0
    for _, ns in ipairs(M.namespaces()) do
        local spec = root().specs[ns]
        if spec.preload and spec.tier ~= "memory" then
            ensure_loaded(spec, ns, sc)
            n = n + 1
        end
    end
    return n
end

-- ─── Flushing ────────────────────────────────────────────────────────────────

--- What writing this scope would do, or nil if it needs no write.
---
--- Also prunes expired keys, because the walk is already happening.
local function build_intent(ns, scope)
    local spec = root().specs[ns]
    if not spec or spec.tier == "memory" then return nil end

    local m = root().meta[ns] and root().meta[ns][scope]
    if not m or not m.dirty then return nil end
    -- A scope whose load failed holds only part of the truth. Writing it would
    -- destroy the rest.
    if m.load_failed then return nil end

    local d = data_of(ns, scope)
    local now = now_wall()
    local doc, count = {}, 0
    for k, v in pairs(d) do
        local exp = m.expiry[k]
        if exp and exp <= now then
            d[k] = nil
            m.expiry[k] = nil
            m.ephemeral[k] = nil
        elseif not m.ephemeral[k] then
            doc[k] = v
            count = count + 1
        end
    end

    if count == 0 and spec.delete_when_empty then
        return { ns = ns, scope = scope, collection = spec.collection,
                 id = doc_id(spec, scope), action = "delete" }
    end
    return { ns = ns, scope = scope, collection = spec.collection,
             id = doc_id(spec, scope), action = "put", doc = doc }
end

local function hash(s)
    local h = 5381
    for i = 1, #s do
        h = (h * 33 + s:byte(i)) % 2147483647
    end
    return h
end

--- Spread a namespace's scopes across its interval, so a correlated event —
--- a server start, an area reset, a raid boss dying — does not put every write
--- on the same tick. Deterministic rather than random so tests and reloads
--- see the same schedule.
local function due_at(spec, scope, m)
    local window = math.floor(spec.flush_seconds / 2)
    local jitter = window > 0 and (hash(scope) % window) or 0
    return m.dirty_at + spec.flush_seconds + jitter
end

--- Everything that should be written now, oldest first, up to `budget`.
---
--- Does no I/O, so a test can assert exactly what *would* be written without
--- stubbing a single database function.
--- @param budget number|nil  max intents; nil for no limit
--- @param opts table|nil     { all = true } ignores the schedule
--- @return table  array of write intents
function M._plan_flush(budget, opts)
    local s = root()
    local t = refresh_clock()
    local all = opts and opts.all
    local candidates = {}

    for ns, spec in pairs(s.specs) do
        if spec.tier ~= "memory" then
            for scope, m in pairs(s.meta[ns] or {}) do
                if m.dirty and not m.poisoned and not m.load_failed
                    and t >= (m.next_attempt or 0)
                    and (all or t >= due_at(spec, scope, m))
                then
                    candidates[#candidates + 1] = { ns = ns, scope = scope, seq = m.dirty_seq }
                end
            end
        end
    end

    -- Oldest dirty first, so worst-case staleness is bounded rather than
    -- being whatever `pairs` felt like.
    table.sort(candidates, function(a, b)
        if a.seq ~= b.seq then return a.seq < b.seq end
        if a.ns ~= b.ns then return a.ns < b.ns end
        return a.scope < b.scope
    end)

    local plan = {}
    for _, c in ipairs(candidates) do
        if budget and #plan >= budget then break end
        local intent = build_intent(c.ns, c.scope)
        if intent then plan[#plan + 1] = intent end
    end
    return plan
end

--- Carry out a plan. The only function in this file that writes.
--- @return table  { written, deleted, failed }
function M._apply(plan)
    local s = root()
    local result = { written = 0, deleted = 0, failed = 0 }
    if #plan > 0 then refresh_clock() end

    for _, intent in ipairs(plan) do
        local m = meta_of(intent.ns, intent.scope)
        local ok, err
        if intent.action == "delete" then
            ok, err = pcall(db_delete, intent.collection, intent.id)
        else
            ok, err = pcall(db_put, intent.collection, intent.id, intent.doc)
        end

        if ok then
            m.dirty = false
            m.fails = 0
            m.next_attempt = 0
            m.last_flush = clock()
            if intent.action == "delete" then
                s.stats.db_deletes = s.stats.db_deletes + 1
                result.deleted = result.deleted + 1
                m.missing = true
            else
                s.stats.db_puts = s.stats.db_puts + 1
                result.written = result.written + 1
            end
        else
            result.failed = result.failed + 1
            s.stats.flush_failures = s.stats.flush_failures + 1
            m.fails = (m.fails or 0) + 1
            m.next_attempt = clock() + math.min(2 ^ m.fails, 60)

            -- Validation already passed at `set` time, so a failure here is
            -- presumed transient and retried. Three failures means it is not,
            -- and retrying forever would eat the shutdown budget. Quarantine
            -- keeps the data in memory so play continues, and makes the
            -- problem visible instead of noisy.
            if m.fails >= 3 then
                m.poisoned = true
                s.stats.poisoned = s.stats.poisoned + 1
                log_error("CACHE_D: quarantined " .. intent.ns .. "/" .. intent.scope
                    .. " after 3 failed writes: " .. tostring(err)
                    .. " — it stays in memory but will not be written again")
            elseif m.fails == 1 or (m.fails % 10) == 0 then
                log_error("CACHE_D: could not write " .. intent.ns .. "/" .. intent.scope
                    .. ": " .. tostring(err) .. " (attempt " .. m.fails .. ", will retry)")
            end
        end
    end
    return result
end

--- Write one scope now, whatever the schedule says.
--- @return boolean|nil  nil if there was nothing to write
function M.flush(ns, scope)
    if not spec_of(ns, "flush") then return false end
    local sc = to_scope(scope);        if not sc then return false end
    local intent = build_intent(ns, sc)
    if not intent then return nil end
    local m = meta_of(ns, sc)
    if m.poisoned then return false end
    return M._apply({ intent }).failed == 0
end

function M.flush_namespace(ns)
    if not spec_of(ns, "flush_namespace") then return 0 end
    local plan = {}
    for scope in pairs(root().meta[ns] or {}) do
        local intent = build_intent(ns, scope)
        if intent then plan[#plan + 1] = intent end
    end
    local r = M._apply(plan)
    return r.written + r.deleted
end

--- Write everything belonging to one character or session.
function M.flush_owner(owner)
    local sc = to_scope(owner); if not sc then return 0 end
    local plan = {}
    for ns, spec in pairs(root().specs) do
        if spec.owner ~= "none" and spec.tier ~= "memory" then
            local intent = build_intent(ns, sc)
            if intent then plan[#plan + 1] = intent end
        end
    end
    local r = M._apply(plan)
    return r.written + r.deleted
end

--- Write everything that is dirty, ignoring the schedule.
---
--- `opts.deadline` bounds it in monotonic seconds: shutdown gives the mudlib a
--- fixed budget, and writing 90% of the scopes and saying so beats being
--- killed part-way through with no idea how far it got.
--- @return number  scopes written
function M.flush_all(opts)
    opts = opts or {}
    local deadline = opts.deadline
    local total, rounds = 0, 0

    while true do
        rounds = rounds + 1
        if rounds > 64 then break end       -- a backstop, not a limit
        if deadline and refresh_clock() >= deadline then
            local left = M.stats().dirty_scopes
            log_error("CACHE_D: ran out of time flushing (" .. tostring(opts.reason or "flush_all")
                .. ") with " .. left .. " scope(s) still dirty — that data is lost")
            break
        end
        local plan = M._plan_flush(nil, { all = true })
        if #plan == 0 then break end
        local r = M._apply(plan)
        total = total + r.written + r.deleted
        if r.written + r.deleted == 0 then break end   -- everything failed; stop retrying
    end
    return total
end

--- What the flush ticker calls.
function M.tick()
    local s = root()
    local budget = cfg("game.cache_flush_budget", 32)
    local r = M._apply(M._plan_flush(budget))

    -- Idle eviction, for scopes nobody's disconnect will ever clean up (rooms,
    -- the world). Clean scopes only, so this can never race a pending write.
    local t = clock()
    for ns, spec in pairs(s.specs) do
        if spec.evict_after and spec.evict_after > 0 then
            for scope, m in pairs(s.meta[ns] or {}) do
                if not m.dirty and (t - m.last_touch) > spec.evict_after then
                    M.drop(ns, scope)
                end
            end
        end
    end
    return r.written + r.deleted
end

--- Check that every value in a scope could actually be written. Expensive and
--- explicit — for an admin command, or a test.
function M.verify(ns, scope)
    local live = M.get_scope(ns, scope)
    if not live then return false, "no such scope" end
    for k, v in pairs(live) do
        local ok, why = jsonsafe.check(v)
        if not ok then return false, k .. ": " .. tostring(why) end
    end
    return true
end

-- ─── Memory ──────────────────────────────────────────────────────────────────

--- Forget a scope without writing it. The data is gone.
function M.drop(ns, scope)
    local sc = to_scope(scope); if not sc then return false end
    local s = root()
    if s.data[ns] then s.data[ns][sc] = nil end
    if s.meta[ns] then s.meta[ns][sc] = nil end
    return true
end

--- Write a scope, then forget it.
function M.evict(ns, scope, opts)
    if not (opts and opts.discard) then
        M.flush(ns, scope)
    end
    return M.drop(ns, scope)
end

--- The disconnect path: write and forget everything this character owned.
function M.evict_owner(owner)
    local sc = to_scope(owner); if not sc then return 0 end
    M.flush_owner(sc)
    local n = 0
    for ns, spec in pairs(root().specs) do
        if spec.owner ~= "none" then
            if root().data[ns] and root().data[ns][sc] then n = n + 1 end
            M.drop(ns, sc)
        end
    end
    return n
end

--- Touch an offline player's state without pinning it in memory forever.
function M.write_offline(ns, scope, fn)
    M.edit(ns, scope, fn)
    M.flush(ns, scope)
    return M.drop(ns, scope)
end

-- ─── Introspection ───────────────────────────────────────────────────────────

function M.stats()
    local s = root()
    local out = {
        db_gets = s.stats.db_gets, db_puts = s.stats.db_puts,
        db_deletes = s.stats.db_deletes,
        rejected_writes = s.stats.rejected_writes,
        oversize_warnings = s.stats.oversize_warnings,
        flush_failures = s.stats.flush_failures,
        loads_failed = s.stats.loads_failed,
        poisoned = s.stats.poisoned,
        dirty_scopes = 0, loaded_scopes = 0, bytes = 0,
        namespaces = 0,
    }
    for ns in pairs(s.specs) do
        out.namespaces = out.namespaces + 1
        for _, m in pairs(s.meta[ns] or {}) do
            out.loaded_scopes = out.loaded_scopes + 1
            out.bytes = out.bytes + (m.bytes or 0)
            if m.dirty then out.dirty_scopes = out.dirty_scopes + 1 end
        end
    end
    return out
end

--- Every scope of a namespace currently in memory, in a stable order.
---
--- Only what is loaded — this is not a query over the database, and it must
--- never become one. A write-behind namespace's documents are up to
--- `flush_seconds` out of date, so asking the store "who has an effect right
--- now" would give an answer that was true a moment ago.
function M.scopes(ns)
    local out = {}
    for scope in pairs(root().meta[ns] or {}) do out[#out + 1] = scope end
    table.sort(out)
    return out
end

function M.inspect(ns, scope)
    local sc = to_scope(scope); if not sc then return nil end
    local m = root().meta[ns] and root().meta[ns][sc]
    if not m then return nil end
    local n = 0
    for _ in pairs(data_of(ns, sc)) do n = n + 1 end
    return {
        namespace = ns, scope = sc, keys = n,
        dirty = m.dirty, dirty_seq = m.dirty_seq,
        loaded = m.loaded, missing = m.missing, load_failed = m.load_failed,
        poisoned = m.poisoned, fails = m.fails, bytes = m.bytes,
        last_flush = m.last_flush, last_touch = m.last_touch,
    }
end

-- ─── Ticker ──────────────────────────────────────────────────────────────────
-- The closure deliberately captures nothing from this module, so a stale one
-- left over from a hot reload still calls the new code through DAEMON.

do
    local interval = cfg("game.cache_flush_seconds", 5)
    if interval > 0 and DAEMON and DAEMON.ticker then
        DAEMON.ticker.every(interval, "cache.flush", function()
            if DAEMON and DAEMON.cache then
                local ok, err = pcall(DAEMON.cache.tick)
                if not ok then
                    log("error", "CACHE_D: flush tick failed: " .. tostring(err))
                end
            end
        end)
    end
end

log("info", "cache_d daemon loaded")

return M
