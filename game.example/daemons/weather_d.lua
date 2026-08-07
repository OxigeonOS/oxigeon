-- game/daemons/weather_d.lua — Weather, and what reads it.
--
-- The daemon recipe from the docs, made real. Game layer, because whether it
-- rains is not the driver's business, and because the interesting part is that
-- *nothing has to be told*: a room description is an lfun, so it asks what the
-- weather is when it is looked at. There is no push, no subscription list, no
-- per-room state to keep in step.
--
-- ─── Why `tag_d` rather than a walk ──────────────────────────────────────────
--
-- The tick has to reach outdoor rooms and only outdoor rooms. Walking
-- `world_d._rooms` and testing each one is O(every room in the world) every
-- tick, forever, to find the dozen that are outside. `DAEMON.tag.find("room",
-- "outdoor")` is a lookup. That is the entire reason the tag index exists, and
-- this is its first consumer.
--
-- ─── State ───────────────────────────────────────────────────────────────────
--
-- Memory tier, by the rule in state-cache.md: if the server restarts it is a
-- new day and the weather is whatever the roll says. Nobody would notice, and
-- nobody should pay a write for it.

local M = {}

--- The states, in the order they can move between. A weather system that can
--- jump from clear to storm reads as broken rather than as dramatic, so
--- transitions are to a neighbour.
M.STATES = { "clear", "overcast", "drizzle", "rain", "storm", "fog" }

--- What each state does to a room, and to anyone in it.
---
--- `light` is a *delta* applied to an outdoor room's own level, not a value:
--- the mine is dark whatever the sky is doing, and a storm should make a bright
--- square dim rather than making every outdoor room the same brightness.
M.EFFECTS = {
    clear    = { light = 0,  ambience = "The sky is a washed-out white, and for once it is dry." },
    overcast = { light = 0,  ambience = "The cloud has come down and sits on the reeds." },
    drizzle  = { light = -1, ambience = "A fine rain falls, too light to hear and too heavy to ignore." },
    rain     = { light = -1, ambience = "Rain comes down hard enough to flatten the reed heads." },
    storm    = { light = -2, ambience = "Wind drives the rain sideways. Somewhere a shutter is losing." },
    fog      = { light = -2, ambience = "Fog stands on the water in walls. Ten feet, and then nothing." },
}

local current = "overcast"
local since = 0

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- @return string
function M.current() return current end

--- @return number  seconds in this state
function M.age() return os_time() - since end

--- @return table  the current state's effects
function M.effects() return M.EFFECTS[current] or M.EFFECTS.overcast end

--- What a room's light level is right now, given the weather.
---
--- Only outdoor rooms are affected, and the floor is 0 rather than a negative
--- number: a room cannot be darker than dark, and a light level below zero
--- would make every `> 0` test in the mudlib subtly wrong.
--- @param room table
--- @return number
function M.light_for(room)
    local base = (type(room) == "table" and room.light_level) or 2
    if type(room) ~= "table" or not room.has_tag or not room:has_tag("outdoor") then
        return base
    end
    return math.max(0, base + (M.effects().light or 0))
end

--- The line a room description appends. Empty indoors.
--- @param room table
--- @return string
function M.ambience_for(room)
    if type(room) ~= "table" or not room.has_tag or not room:has_tag("outdoor") then
        return ""
    end
    return M.effects().ambience or ""
end

--- Move to a neighbouring state.
---
--- The walk is deliberately not uniform: `clear` and `storm` are the ends of
--- the range, so they are half as likely to be entered as the middle states,
--- which is roughly how weather behaves and needs no extra table to say so.
--- @param forced string|nil  a state to move to directly, for an admin command
--- @return string  the new state
function M.advance(forced)
    local previous = current

    if forced and M.EFFECTS[forced] then
        current = forced
    else
        local index
        for i, name in ipairs(M.STATES) do
            if name == current then index = i break end
        end
        index = index or 2

        local roll = math.random(3)
        if roll == 1 then
            index = math.max(1, index - 1)
        elseif roll == 2 then
            index = math.min(#M.STATES, index + 1)
        end
        current = M.STATES[index]
    end

    if current ~= previous then
        since = os_time()
        M.announce(previous)
        if DAEMON and DAEMON.event then
            pcall(DAEMON.event.emit, "weather.changed", {
                from = previous, to = current,
            })
        end
    end
    return current
end

--- Tell everyone standing outdoors.
---
--- One `tag_d` lookup and then only the rooms that matter. Nobody indoors is
--- told, which is the whole point of the tag.
--- @param previous string|nil
function M.announce(previous)
    if not (DAEMON and DAEMON.tag and DAEMON.world) then return end

    local line = "{cyan}" .. (M.effects().ambience or "") .. "{/}"
    local messaging = require('lib.messaging')

    for _, room_id in ipairs(DAEMON.tag.find("room", "outdoor")) do
        local ok, err = pcall(messaging.send_to_room, room_id, line, nil)
        if not ok then
            log_error("WEATHER_D: could not announce in '" .. room_id .. "': " .. tostring(err))
        end
    end
end

-- ─── The tick ────────────────────────────────────────────────────────────────

if DAEMON and DAEMON.task then
    local seconds = 300
    if type(config) == "function" then
        local ok, configured = pcall(config, "game.weather_seconds")
        if ok and type(configured) == "number" and configured > 0 then
            seconds = configured
        end
    end

    local ok, err = pcall(DAEMON.task.schedule, {
        id       = "weather.advance",
        interval = seconds,
        label    = "Advance the weather",
        func     = function() return M.advance() end,
    })
    if not ok then
        log_error("WEATHER_D: could not schedule the weather: " .. tostring(err))
    end
end

since = os_time()
log("info", "weather_d loaded — it is " .. current)

return M
