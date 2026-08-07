-- mudlib/daemons/cooldown_d.lua — "not yet" gates, per character.
--
-- Stores the moment a thing becomes available again, never the moment it was
-- last used. Storing expiry means the check does not have to know the duration,
-- so changing a cooldown from 24 hours to 12 does not retroactively rewrite
-- everyone's remaining time into something wrong.
--
-- Two homes, chosen by how long the gate lasts:
--
--   >= game.cooldown_durable_seconds (60)   written immediately; survives a restart
--   <  game.cooldown_durable_seconds        memory only; forgotten on restart
--
-- The threshold rather than a mandatory flag, because the two mistakes are not
-- equally loud. Forget the flag on a 2-second ability cooldown and you write to
-- disk on every use by every player — a slow leak that surfaces months later as
-- "the game feels sluggish" with nothing obvious to blame. Forget it on a daily
-- reward and the gate resets on the next restart, which a player reports the
-- same day. The default belongs on the side of the loud failure.
--
--   Under a minute it is a game mechanic. Over a minute it is a promise to
--   the player.
--
-- Pass `{ durable = true }` or `{ durable = false }` for the cases that rule
-- gets wrong — a 10-second gate on a rare reward, a 5-minute one nobody minds
-- losing.
--
-- ─── Who a gate belongs to ───────────────────────────────────────────────────
--
-- `char_id` was a scope with one hardcoded shape, and then mobs wanted abilities
-- too. Every function here now also takes an *entity* — a player, or a creature.
-- A creature's gates go to a third namespace and are **memory-only by
-- construction, not by threshold**:
--
--   a mob instance id is `mob:<seq>` from a sequence that restarts with the
--   process, so a durable mob cooldown would come back after a reboot attached
--   to a different creature.
--
-- Widened here rather than reimplemented in `ability_d`, because a private
-- cooldown store would mean `cooldown list` shows a player's gates and silently
-- omits a mob's, and the durable/fast rule would exist in two places and drift.
--
-- Exposes:
--   DAEMON.cooldown.mark(who, what, seconds, opts) -> expires_at | false
--   DAEMON.cooldown.remaining(who, what)           -> seconds (0 = ready)
--   DAEMON.cooldown.ready(who, what)               -> boolean
--   DAEMON.cooldown.expires_at(who, what)          -> unix seconds | nil
--   DAEMON.cooldown.clear(who, what)               -> boolean
--   DAEMON.cooldown.clear_all(who)                 -> count
--   DAEMON.cooldown.list(who)                      -> array
--   DAEMON.cooldown.scope(who)                     -> scope, is_object
--
-- `who` is a char_id (number or string), or an entity table. See
-- docs/src/lua-api/state-cache.md.

local M = {}

local DURABLE = "cooldowns"
local FAST    = "cooldowns_fast"
local OBJECT  = "cooldowns_obj"

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.warn, message)
    end
end

--- The stored value *is* the expiry, so the cache can prune dead gates on its
--- own without a reaper of any kind.
local function expiry_of(_, value)
    return type(value) == "number" and value or nil
end

do
    if DAEMON and DAEMON.cache then
        DAEMON.cache.define(DURABLE, {
            tier              = "write_through",
            scope_prefix      = "char:",
            owner             = "char",
            preload           = true,
            delete_when_empty = true,
            expiry_of         = expiry_of,
        })
        DAEMON.cache.define(FAST, {
            tier         = "memory",
            scope_prefix = "char:",
            owner        = "char",
            expiry_of    = expiry_of,
        })
        -- `owner = "none"`, not `"char"`: `cache.evict_owner` runs on disconnect
        -- and a creature never disconnects, so a mob scope in a char-owned
        -- namespace is one nothing ever evicts. `mob_d.despawn` drops it
        -- explicitly instead, beside the effect and trait detaches.
        DAEMON.cache.define(OBJECT, {
            tier         = "memory",
            scope_prefix = "obj:",
            owner        = "none",
            expiry_of    = expiry_of,
        })
    else
        log("error", "COOLDOWN_D: cache_d is not loaded — cooldowns will not work")
    end
end

local function threshold()
    local ok, v = pcall(config, "game.cooldown_durable_seconds")
    if ok and type(v) == "number" and v > 0 then return v end
    return 60
end

--- Whose gates these are.
---
--- A character id, or an entity. A player carries `char_id`; a creature carries
--- only `id`, and gets an object scope with its own memory-only namespace.
--- @param who number|string|table
--- @return string|number|nil scope, boolean is_object
function M.scope(who)
    if type(who) == "number" or type(who) == "string" then return who, false end
    if type(who) ~= "table" then return nil, false end

    if who.char_id ~= nil then return who.char_id, false end
    if type(who.id) == "string" and #who.id > 0 then return who.id, true end
    return nil, false
end

--- Which namespaces hold this scope's gates, most durable first.
local function spaces(is_object)
    if is_object then return { OBJECT } end
    return { DURABLE, FAST }
end

local function valid(scope, what, who)
    if type(scope) ~= "number" and type(scope) ~= "string" then
        log_warn("COOLDOWN_D." .. who .. ": expected a char_id or an entity, got " .. type(scope))
        return false
    end
    if type(what) ~= "string" or #what == 0 then
        log_warn("COOLDOWN_D." .. who .. ": the cooldown name must be a non-empty string")
        return false
    end
    return true
end

--- Which tier is this gate in? Answered by looking, not by recomputing the
--- threshold — so changing the config, or an explicit `durable` override, can
--- never strand a gate in a namespace nobody reads any more.
local function read_both(scope, what, is_object)
    if not (DAEMON and DAEMON.cache) then return nil end
    local best = nil
    for _, ns in ipairs(spaces(is_object)) do
        local v = DAEMON.cache.get(ns, scope, what)
        if type(v) == "number" then best = best and math.max(best, v) or v end
    end
    return best
end

--- Start (or extend) a cooldown.
--- @param seconds number  how long from now
--- @param opts table|nil  { durable = true|false } to override the threshold
--- @return number|false   the expiry timestamp
function M.mark(who, what, seconds, opts)
    local scope, is_object = M.scope(who)
    if not valid(scope, what, "mark") then return false end
    if type(seconds) ~= "number" or seconds <= 0 then
        log_warn("COOLDOWN_D.mark('" .. what .. "'): seconds must be a positive number, got "
            .. tostring(seconds))
        return false
    end
    if not (DAEMON and DAEMON.cache) then return false end

    local expires = os_time() + seconds

    -- A creature's gates are memory-only whatever the duration says, because a
    -- durable one would outlive the creature and land on whichever mob got the
    -- same sequence number after a restart.
    if is_object then
        if not DAEMON.cache.set(OBJECT, scope, what, expires, { expires_at = expires }) then
            return false
        end
        return expires
    end

    local durable = opts and opts.durable
    if durable == nil then durable = seconds >= threshold() end

    local ns = durable and DURABLE or FAST

    -- Moving between tiers (an explicit override, or a duration that crossed
    -- the threshold) must not leave the old copy behind to be found by
    -- `remaining`.
    local other = durable and FAST or DURABLE
    if DAEMON.cache.get(other, scope, what) ~= nil then
        DAEMON.cache.delete(other, scope, what)
    end

    if not DAEMON.cache.set(ns, scope, what, expires, { expires_at = expires }) then
        return false
    end
    return expires
end

--- Seconds until this is available again. 0 means ready now.
function M.remaining(who, what)
    local scope, is_object = M.scope(who)
    if not valid(scope, what, "remaining") then return 0 end
    local expires = read_both(scope, what, is_object)
    if not expires then return 0 end
    local left = expires - os_time()
    if left <= 0 then return 0 end
    return left
end

function M.ready(who, what)
    return M.remaining(who, what) <= 0
end

--- The raw expiry timestamp, for a caller that wants to format a date rather
--- than a countdown.
function M.expires_at(who, what)
    local scope, is_object = M.scope(who)
    if not valid(scope, what, "expires_at") then return nil end
    return read_both(scope, what, is_object)
end

function M.clear(who, what)
    local scope, is_object = M.scope(who)
    if not valid(scope, what, "clear") then return false end
    if not (DAEMON and DAEMON.cache) then return false end
    local gone = false
    for _, ns in ipairs(spaces(is_object)) do
        if DAEMON.cache.delete(ns, scope, what) then gone = true end
    end
    return gone
end

function M.clear_all(who)
    local scope, is_object = M.scope(who)
    if scope == nil or not (DAEMON and DAEMON.cache) then return 0 end
    local n = 0
    for _, ns in ipairs(spaces(is_object)) do
        for _, key in ipairs(DAEMON.cache.keys(ns, scope)) do
            if DAEMON.cache.delete(ns, scope, key) then n = n + 1 end
        end
    end
    return n
end

--- Everything currently gating this character or creature, ready ones dropped.
--- @return table  array of { what, remaining, expires_at, durable }
function M.list(who)
    local out = {}
    local scope, is_object = M.scope(who)
    if scope == nil or not (DAEMON and DAEMON.cache) then return out end

    local now = os_time()
    for _, ns in ipairs(spaces(is_object)) do
        for _, key in ipairs(DAEMON.cache.keys(ns, scope)) do
            local expires = DAEMON.cache.get(ns, scope, key)
            if type(expires) == "number" and expires > now then
                out[#out + 1] = {
                    what = key,
                    remaining = expires - now,
                    expires_at = expires,
                    durable = ns == DURABLE,
                }
            end
        end
    end
    table.sort(out, function(a, b) return a.remaining < b.remaining end)
    return out
end

log("info", "cooldown_d daemon loaded")

return M
