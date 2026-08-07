-- mudlib/daemons/queue_d.lua — What you have committed to doing next.
--
-- A track is a named lane of intent on one entity: a bounded queue, a roundtime
-- gate, a policy for what an empty queue does, and a short history of what
-- finished. Combat is the first one. Crafting and gathering are meant to feel
-- like the same mini-game later, so a track is registered rather than hardcoded
-- and a second one needs no edit to this file.
--
-- ─── Roundtime never gates a command ─────────────────────────────────────────
--
-- The rule this daemon exists to keep. Roundtime says "this **track** may not
-- act again for N seconds" — it does not say "you are busy". `look`, `say` and
-- `who` work throughout, and not because of an exemption list: nothing in
-- command dispatch reads a track, so they never enter the code path at all.
--
-- That is the deliberate rejection of the single global "action" cooldown other
-- engines use, where being mid-swing stops you looking at the room.
--
-- ─── Recovery, not occupation ────────────────────────────────────────────────
--
-- `ability_d`'s `cast_time` and `channel` already own "you are in the middle of
-- something". Roundtime is what you owe *after* finishing. Because they are
-- different things they need no arbitration — the only interaction is that a
-- tick skips an entity that is casting, which is one line reading one existing
-- public function.
--
-- ─── Where roundtime lives ───────────────────────────────────────────────────
--
-- `cooldown_d`, under `rt.<track>`. Not a private store: every roundtime is
-- under a minute, so cooldown_d's threshold rule already puts it in memory and
-- forgets it on restart, which is exactly right and free. It already handles
-- creatures as well as players, `evict_owner` already cleans it up on
-- disconnect, and `cooldown list` already answers "why can't I swing".
--
-- Exposes:
--   DAEMON.queue.define_track(name, spec) / track(name) / tracks()
--   DAEMON.queue.enqueue(entity, track, entry, opts) -> ok, why
--   DAEMON.queue.list(entity, track) / clear(entity, track) / history(entity, track)
--   DAEMON.queue.roundtime(entity, track) / in_roundtime(entity, track)
--   DAEMON.queue.mark(entity, track, amount, ctx) -> expires_at | false
--   DAEMON.queue.round_length(entity, track)      -> seconds
--   DAEMON.queue.policy(entity, track) / set_policy(entity, track, policy)
--   DAEMON.queue.advance(entity, track)           -> boolean
--   DAEMON.queue.tick()                           -> number
--   DAEMON.queue.cleanup(char_id) / detach(entity)
--
-- See docs/src/lua-api/queues.md.

local Queues    = require('lib.queues')
local Abilities = require('lib.abilities')

local M = {}

local NS = "queue"

--- Registered tracks. Module-level, so a hot reload of this file re-registers
--- them from whoever owns them — `combat_d` registers "combat" at its own load.
local _tracks = {}

--- Which tracks have already complained about a missing round trait. A warning
--- per track, not per action: a fight would otherwise fill the journal.
local _warned = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

do
    if DAEMON and DAEMON.cache then
        -- Memory, for two reasons that agree. An entry holds the **resolved
        -- target entity** — re-resolving by name at dequeue would let somebody
        -- retarget you by walking a matching creature into the room — and memory
        -- is the only tier that may hold a live reference. And intent that
        -- outlived a restart belongs to a world that no longer exists.
        --
        -- `owner = "none"` because the scope space is shared with creatures, and
        -- a mob scope in a char-owned namespace is one nothing ever evicts.
        -- Cleanup is explicit instead: `cleanup` on disconnect, `detach` on
        -- despawn.
        DAEMON.cache.define(NS, { tier = "memory", owner = "none" })
    else
        log("error", "QUEUE_D: cache_d is not loaded — action queues will not work")
    end
end

local function conf(key, default)
    local ok, v = pcall(config, key)
    if ok and type(v) == "number" then return v end
    return default
end

--- `char:<id>` for a player, `obj:<id>` for a creature — the convention
--- `effect_d`, `combat_d` and `ability_d` all use.
local function scope_of(entity)
    if type(entity) ~= "table" then return nil end
    if entity.char_id ~= nil then return "char:" .. tostring(entity.char_id) end
    if entity.id ~= nil then return "obj:" .. tostring(entity.id) end
    return nil
end

local function valid(entity, track, who)
    if type(entity) ~= "table" then
        log_warn("QUEUE_D." .. who .. ": expected an entity, got " .. type(entity))
        return false
    end
    if type(track) ~= "string" or track == "" then
        log_warn("QUEUE_D." .. who .. ": a track name must be a non-empty string")
        return false
    end
    return true
end

-- ─── Tracks ──────────────────────────────────────────────────────────────────

--- Register a lane of intent.
---
--- `combat_d` registers `"combat"`; a game registers `"crafting"` with its own
--- `resolve` and its own `round_trait` and touches nothing in the mudlib. That
--- is the test of whether this is genuinely generic rather than combat with a
--- coat of paint.
--- @param spec table { round_trait, round_seconds, max, history, stale, empty,
---                     default_entry, resolve }
--- @return boolean
function M.define_track(name, spec)
    if type(spec) ~= "table" then spec = {} end
    spec.id = name
    local normalised, err = Queues.normalise_track(spec)
    if not normalised then
        log_warn("QUEUE_D.define_track: " .. tostring(err))
        return false
    end
    normalised.max     = tonumber(normalised.max) or conf("game.queue_max", 3)
    normalised.history = tonumber(normalised.history) or conf("game.queue_history", 5)
    normalised.stale   = tonumber(normalised.stale) or conf("game.queue_stale_seconds", 30)
    _tracks[name] = normalised
    _warned[name] = nil
    return true
end

function M.track(name) return _tracks[name] end

function M.tracks()
    local out = {}
    for name in pairs(_tracks) do out[#out + 1] = name end
    table.sort(out)
    return out
end

-- ─── Roundtime ───────────────────────────────────────────────────────────────

--- How long one of this track's rounds is, for this entity.
---
--- A derived trait the **game** defines, so agility, encumbrance and the wielded
--- weapon all reach it through the trait graph and the existing
--- `stat_bonus` → `equip_trait_<id>` machinery with no new code here.
---
--- An absent trait falls back and **says so, once per track**. `Abilities.roll`
--- reads an absent trait as zero, which would silently make every roundtime
--- nothing — a silent wrong answer, which is not something this project ships.
--- @return number seconds
function M.round_length(entity, track)
    local spec = _tracks[track]
    if not spec then return conf("game.combat_round_seconds", 3) end

    if DAEMON and DAEMON.trait and DAEMON.trait.has
        and DAEMON.trait.has(entity, spec.round_trait) then
        local v = DAEMON.trait.value(entity, spec.round_trait)
        if type(v) == "number" and v > 0 then return v end
    end

    if not _warned[track] then
        _warned[track] = true
        log_warn("QUEUE_D: no '" .. tostring(spec.round_trait) .. "' trait, so the '"
            .. track .. "' track is using " .. spec.round_seconds
            .. "s rounds. Define it in the game layer to make roundtime respond "
            .. "to agility, encumbrance and what is wielded.")
    end
    return spec.round_seconds
end

--- Seconds until this track may act again. 0 means now.
function M.roundtime(entity, track)
    if not (DAEMON and DAEMON.cooldown) then return 0 end
    return DAEMON.cooldown.remaining(entity, Queues.rt_key(track))
end

function M.in_roundtime(entity, track)
    return M.roundtime(entity, track) > 0
end

--- Owe this track some recovery.
---
--- `amount` takes every shape an ability amount does, including `{ rounds = n }`
--- — which is why `ctx.round_length` is filled in here rather than by the
--- caller.
--- @return number|false expires_at
function M.mark(entity, track, amount, ctx)
    if not valid(entity, track, "mark") then return false end
    if not (DAEMON and DAEMON.cooldown) then return false end

    local c = {}
    for k, v in pairs(ctx or {}) do c[k] = v end
    c.user = c.user or entity
    c.round_length = M.round_length(entity, track)

    local seconds = Abilities.roll(amount, c, math.random)
    seconds = math.max(0, math.ceil(seconds))
    if seconds <= 0 then return false end

    return DAEMON.cooldown.mark(entity, Queues.rt_key(track), seconds, { durable = false })
end

-- ─── The queue ───────────────────────────────────────────────────────────────

local function slot(entity, track)
    local scope = scope_of(entity)
    if not scope then return nil, nil end
    local held = DAEMON.cache.get(NS, scope, track)
    if type(held) ~= "table" then
        held = { queue = {}, history = {}, self = entity }
        DAEMON.cache.set(NS, scope, track, held)
    end
    -- The live entity, so a tick can act on it without a lookup. `combat_d`
    -- keeps `self` in its own memory namespace for the same reason.
    held.self = entity
    return held, scope
end

--- Commit to doing something on a track.
--- @param opts table|nil { front = true }
--- @return boolean ok, string|nil why
function M.enqueue(entity, track, entry, opts)
    if not valid(entity, track, "enqueue") then return false, "No such track." end
    local spec = _tracks[track]
    if not spec then return false, "No such track." end
    if not (DAEMON and DAEMON.cache) then return false, "Queues are unavailable." end

    local normalised, err = Queues.normalise_entry(entry)
    if not normalised then
        log_warn("QUEUE_D.enqueue: " .. tostring(err))
        return false, "That cannot be queued."
    end
    normalised.at = normalised.at or os_time()

    local held = slot(entity, track)
    if not held then return false, "That cannot act." end

    local ok, why = Queues.push(held.queue, normalised,
        { max = spec.max, front = opts and opts.front })
    if not ok then return false, why end
    return true
end

function M.list(entity, track)
    local held = slot(entity, track)
    if not held then return {} end
    local out = {}
    for i, e in ipairs(held.queue) do out[i] = e end
    return out
end

function M.clear(entity, track)
    local held = slot(entity, track)
    if not held then return 0 end
    local n = #held.queue
    held.queue = {}
    return n
end

function M.history(entity, track)
    local held = slot(entity, track)
    if not held then return {} end
    local out = {}
    for i, e in ipairs(held.history) do out[i] = e end
    return out
end

-- ─── The empty-queue policy ──────────────────────────────────────────────────

--- What this entity does on this track with nothing queued.
function M.policy(entity, track)
    local held = slot(entity, track)
    if held and held.policy then return held.policy end
    local spec = _tracks[track]
    return spec and spec.empty or "idle"
end

function M.set_policy(entity, track, policy)
    if policy ~= "auto" and policy ~= "idle" and policy ~= "repeat" then
        return false, "One of: auto idle repeat"
    end
    local held = slot(entity, track)
    if not held then return false end
    held.policy = policy
    return true
end

-- ─── Advancing ───────────────────────────────────────────────────────────────

--- Run the head of a track's queue, if it may run.
---
--- Returns false when nothing happened, which is the ordinary case: in
--- roundtime, casting, or nothing queued.
--- @return boolean
function M.advance(entity, track)
    local spec = _tracks[track]
    if not spec or not (DAEMON and DAEMON.cache) then return false end

    -- Occupation is somebody else's concept, and this is the whole of the
    -- interaction with it.
    if DAEMON.ability and DAEMON.ability.casting and DAEMON.ability.casting(entity) then
        return false
    end
    if M.in_roundtime(entity, track) then return false end

    local held = slot(entity, track)
    if not held then return false end

    -- Drop anything the player would be surprised to see act, bounded so a queue
    -- of nothing but stale entries cannot spin.
    local now = os_time()
    local dropped = 0
    while #held.queue > 0 and dropped < spec.max
        and Queues.is_stale(held.queue[1], now, spec.stale) do
        table.remove(held.queue, 1)
        dropped = dropped + 1
    end

    local entry = Queues.pop(held.queue)
    if not entry then return false end

    if type(spec.resolve) ~= "function" then
        log_error("QUEUE_D: track '" .. track .. "' has no resolve function")
        return false
    end

    local ok, ran = pcall(spec.resolve, entity, entry)
    if not ok then
        log_error("QUEUE_D: resolving a '" .. track .. "' entry raised: " .. tostring(ran))
        return false
    end
    if ran == false then return false end

    Queues.remember(held.history, entry, spec.history)
    return true
end

--- Every entity with something to do, on every track.
---
--- Driven by one ticker rather than a timer per fighter: `ticker_d` holds its
--- callbacks in a module table that does not survive a hot reload of itself, so
--- one repeating timer strands one thing where N would strand every fight in the
--- game.
--- @return number  how many acted
function M.tick()
    if not (DAEMON and DAEMON.cache) then return 0 end
    local acted = 0
    for _, scope in ipairs(DAEMON.cache.scopes(NS)) do
        for _, track in ipairs(M.tracks()) do
            local held = DAEMON.cache.get(NS, scope, track)
            if type(held) == "table" and type(held.self) == "table" then
                if M.advance(held.self, track) then acted = acted + 1 end
            end
        end
    end
    return acted
end

-- ─── Housekeeping ────────────────────────────────────────────────────────────

--- Everything this character had planned, on disconnect.
function M.cleanup(char_id)
    if not (DAEMON and DAEMON.cache) then return end
    DAEMON.cache.drop(NS, "char:" .. tostring(char_id))
end

--- The same for a creature, from `mob_d.despawn`.
function M.detach(entity)
    local scope = scope_of(entity)
    if scope and DAEMON and DAEMON.cache then DAEMON.cache.drop(NS, scope) end
end

log("info", "queue_d daemon loaded")

return M
