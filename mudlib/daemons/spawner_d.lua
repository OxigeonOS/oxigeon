-- mudlib/daemons/spawner_d.lua — Places that produce creatures.
--
-- ─── Not the same thing as `spawn_room` ──────────────────────────────────────
--
-- `mob_d` already knew how to put creatures in the world, and this does not
-- replace it. The two answer different questions and both are worth having:
--
--   mob.spawn_room + mob.count    a **fixed population**. Bellow is in the
--                                 smithy and there is one of her. `populate()`
--                                 tops that up and is idempotent.
--   room.spawn_*                  a **source**. This nest makes rats, of these
--                                 kinds, up to this many, over time.
--
-- The difference shows in the cap. `populate` counts *per template*: three rat
-- templates with `count = 2` is six rats, and there is no way to say "six rats
-- of any kind is too many for one pantry". A spawner's cap spans its whole
-- table, which is the thing that could not be expressed before.
--
-- ─── Authored on the room ────────────────────────────────────────────────────
--
-- `spawn_max`, `spawn_interval` and `spawn_table` are ordinary room schema
-- fields in types the schema already had, so OLC authors a spawner with no new
-- machinery: `olc set spawn_max 4` works, `verify` checks the templates exist,
-- and a generated `rooms.lua` round-trips it. The cost is one spawner per room,
-- seeding itself — which is what a nest is.
--
-- ─── Filled at load, a trickle afterwards ────────────────────────────────────
--
-- The first tick fills the room to `spawn_max` in one go, and every tick after
-- that adds at most one. A server that has just started should not have empty
-- rooms for `max * interval` seconds, and a room that has just been cleared
-- should refill at a rate the player can outrun. Those are different needs and
-- one rule cannot serve both.
--
-- Nothing is scheduled per creature. **A template in a spawn table must not
-- also carry `respawn_time`**, or a kill schedules a `mob_d` respawn *and* the
-- spawner tops up, and the room slowly doubles. `verify` reports that.
--
-- Exposes:
--   DAEMON.spawner.notice(room)        world_d calls this as rooms register
--   DAEMON.spawner.rooms()             sorted array of room ids with a spawner
--   DAEMON.spawner.spec(room_id)       the live spawner, read off the room
--   DAEMON.spawner.population(room_id) how many of its kinds are alive there
--   DAEMON.spawner.tick(room_id)       one top-up, ignoring the clock
--   DAEMON.spawner.fill(room_id)       straight to `spawn_max`
--
-- See docs/src/lua-api/spawners.md.

local M = {}

--- How often the single heartbeat runs, in seconds.
---
--- One timer for every spawner in the world rather than one each: a hundred
--- nests is a hundred entries in `ticker_d.list()` and a hundred closures, to
--- do work that is a handful of table lookups. Each spawner keeps its own
--- `due` timestamp, so `spawn_interval` still means what it says — this only
--- bounds how precisely it is honoured.
local HEARTBEAT = 5

--- room_id -> { due = <epoch seconds>, seeded = <boolean> }
---
--- Deliberately **not** a copy of the room's numbers. `spec()` reads them off
--- the live room every time, so `olc set spawn_max 4` takes effect on the next
--- tick the way every other OLC edit takes effect immediately. Caching them
--- here would make the spawner the one field a builder has to reload for.
local _state = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

--- Injected so a test can pin the weighted pick, the way `combat_d._roll` is.
--- @return number  in [0, 1)
function M._random()
    return math.random()
end

local function now()
    if type(os_time) == "function" then
        local ok, t = pcall(os_time)
        if ok and type(t) == "number" then return t end
    end
    return os.time()
end

-- ─── Reading the room ────────────────────────────────────────────────────────

--- The spawner on a room, or nil.
---
--- Read live, and validated here rather than trusted: these fields are authored,
--- and an author who writes `spawn_max = 3` and no table has made a room that
--- would otherwise sit in the index doing nothing at every heartbeat for ever.
--- @param room_id string
--- @return table|nil { room, max, interval, table }
function M.spec(room_id)
    if not (DAEMON and DAEMON.world) then return nil end
    local room = DAEMON.world.get_room(room_id)
    if type(room) ~= "table" then return nil end

    local max = tonumber(room.spawn_max) or 0
    if max <= 0 then return nil end

    local entries = {}
    for _, e in ipairs(room.spawn_table or {}) do
        if type(e) == "table" and type(e.template) == "string" then
            local weight = tonumber(e.weight) or 1
            if weight > 0 then
                entries[#entries + 1] = { template = e.template, weight = weight }
            end
        end
    end
    if #entries == 0 then return nil end

    return {
        room     = room_id,
        max      = math.floor(max),
        interval = math.max(1, tonumber(room.spawn_interval) or 60),
        table    = entries,
    }
end

--- Every room with a live spawner, sorted.
function M.rooms()
    local ids = {}
    for id in pairs(_state) do ids[#ids + 1] = id end
    table.sort(ids)
    return ids
end

-- ─── The index ───────────────────────────────────────────────────────────────

--- Told about a room as it registers. Called from `world_d.register_room`,
--- beside the `tag_d.index` call and for the same reason: a room entering the
--- world is the one moment every path goes through, so an index fed there
--- cannot drift. A room that loses its spawner on an area reset drops out here
--- rather than lingering.
--- @param room table
function M.notice(room)
    if type(room) ~= "table" or not room.id then return end

    local has = tonumber(room.spawn_max) and tonumber(room.spawn_max) > 0
        and type(room.spawn_table) == "table" and #room.spawn_table > 0

    if not has then
        _state[room.id] = nil
        return
    end

    -- `seeded = false` on every registration, so an area reset refills the room
    -- rather than trickling. `fill` counts what is already alive, so a reset of
    -- a full room does nothing.
    _state[room.id] = { due = 0, seeded = false }
end

-- ─── Counting ────────────────────────────────────────────────────────────────

--- How many creatures of this spawner's kinds are alive in its room.
---
--- **Its own kinds, not every creature present.** Counting everything would let
--- a player switch a nest off by luring something unrelated into the room, and
--- would stop a guard patrol route through a spawner room from ever refilling.
--- A rat that wandered in from next door does count, because it is a rat and
--- the cap is about how many rats the room should hold.
--- @param room_id string
--- @return number
function M.population(room_id)
    local spec = M.spec(room_id)
    if not spec or not (DAEMON and DAEMON.mobs) then return 0 end

    local mine = {}
    for _, e in ipairs(spec.table) do mine[e.template] = true end

    local n = 0
    local ok, occupants = pcall(DAEMON.mobs.in_room, room_id)
    if not ok or type(occupants) ~= "table" then return 0 end
    for _, mob in ipairs(occupants) do
        if mine[mob.template_id] then n = n + 1 end
    end
    return n
end

-- ─── Spawning ────────────────────────────────────────────────────────────────

--- One weighted pick from a spawn table.
---
--- Weights are relative to each other and nothing else, so `{5, 3, 1}` and
--- `{50, 30, 10}` are the same table. That is worth stating because the
--- alternative — weights as probabilities that must sum to one — is a rule
--- authors get wrong silently, and the silent version of getting it wrong is a
--- creature that never appears.
--- @return string|nil template_id
local function pick(spec)
    local total = 0
    for _, e in ipairs(spec.table) do total = total + e.weight end
    if total <= 0 then return nil end

    local roll = M._random() * total
    for _, e in ipairs(spec.table) do
        roll = roll - e.weight
        if roll <= 0 then return e.template end
    end
    -- Floating point: the walk above can fall through on the last entry.
    return spec.table[#spec.table].template
end

--- Put one creature in, if there is room for it.
--- @return string|nil template_id spawned
local function spawn_one(spec)
    if M.population(spec.room) >= spec.max then return nil end

    local template_id = pick(spec)
    if not template_id then return nil end

    if not (DAEMON and DAEMON.mobs) then return nil end
    if DAEMON.mobs.get(template_id) == nil then
        log_warn("SPAWNER_D: " .. spec.room .. " names creature '" .. template_id
            .. "', which is not registered — nothing will come of it")
        return nil
    end

    local ok, mob = pcall(DAEMON.mobs.spawn, template_id, spec.room)
    if not ok then
        log_error("SPAWNER_D: " .. spec.room .. " failed to spawn '"
            .. template_id .. "': " .. tostring(mob))
        return nil
    end
    return mob and template_id or nil
end

--- Top the room straight up to `spawn_max`.
---
--- Bounded by `max` iterations rather than by "until full": if a template is
--- missing or a spawn is refused, `population` never rises and an
--- until-full loop would run for ever on the game thread.
--- @return number  how many were added
function M.fill(room_id)
    local spec = M.spec(room_id)
    if not spec then return 0 end

    local added = 0
    for _ = 1, spec.max do
        if not spawn_one(spec) then break end
        added = added + 1
    end
    return added
end

--- Fill every spawner in the world, now.
---
--- The counterpart to `mob_d.populate()`, and called beside it for the same
--- reason: a world that has just loaded should be populated when the first
--- player arrives, not `HEARTBEAT` seconds later. It cannot happen as rooms
--- register, because `areaload` loads in passes — items, then rooms, then mobs —
--- so at the moment a room is noticed the creatures it names do not exist yet.
---
--- Idempotent, because `fill` counts what is already alive. Safe to call on
--- every area reset, which is what makes it safe to call from an `on_load`.
--- @return number  how many were added
function M.fill_all()
    local n = 0
    for _, room_id in ipairs(M.rooms()) do
        local ok, added = pcall(M.fill, room_id)
        if ok then
            n = n + (added or 0)
            _state[room_id].seeded = true
        else
            log_error("SPAWNER_D: filling " .. room_id .. ": " .. tostring(added))
        end
    end
    if n > 0 then log("info", "SPAWNER_D: seeded " .. n .. " creature(s)") end
    return n
end

--- One top-up, ignoring the clock. The unit a test drives.
--- @return number  how many were added (0 or 1)
function M.tick(room_id)
    local spec = M.spec(room_id)
    if not spec then return 0 end
    return spawn_one(spec) and 1 or 0
end

-- ─── The heartbeat ───────────────────────────────────────────────────────────

local function heartbeat()
    local t = now()
    for _, room_id in ipairs(M.rooms()) do
        local state = _state[room_id]
        local spec = M.spec(room_id)

        if not spec then
            -- The room lost its spawner without re-registering — an `olc set
            -- spawn_max 0`, say. Drop it rather than asking again for ever.
            _state[room_id] = nil
        elseif t >= (state.due or 0) then
            local ok, err = pcall(function()
                if not state.seeded then
                    M.fill(room_id)
                    state.seeded = true
                else
                    M.tick(room_id)
                end
            end)
            if not ok then
                log_error("SPAWNER_D: " .. room_id .. ": " .. tostring(err))
            end
            state.due = t + spec.interval
        end
    end
end

--- Stop and forget everything. For tests and for a full world reload.
function M.clear()
    _state = {}
end

if DAEMON and DAEMON.ticker then
    DAEMON.ticker.every(HEARTBEAT, "spawner.heartbeat", heartbeat)
else
    log("warn", "SPAWNER_D: no ticker_d, so nothing will refill")
end

log("debug", "SPAWNER_D: daemon loaded")

return M
