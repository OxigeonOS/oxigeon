-- game/daemons/reach_d.lua — The Drowned Reach: an infinite grid, generated.
--
-- `register_virtual` and `evict_virtual` both existed; the first had no game
-- using it and the second had **zero callers anywhere**, so `world_d._rooms`
-- accumulated every virtual room ever generated. Bounded for a small ocean and
-- unbounded for a grid like this one, which is why the eviction work was a
-- prerequisite for this area rather than a cleanup after it.
--
-- ─── What makes a virtual room work ──────────────────────────────────────────
--
-- **The id is the persistence.** `reach.3.-7` is not a name for a room; it *is*
-- the room, and the object is regenerated from it identically every time. That
-- is why throwing the object away when the last person leaves costs nothing,
-- and why anything that must persist out here has to live somewhere else —
-- object state on a virtual room goes when the room does, on purpose.
--
-- ─── Determinism ─────────────────────────────────────────────────────────────
--
-- The description varies by coordinate and **not** by `math.random`: two people
-- standing in `reach.4.4` must read the same thing, and the same person coming
-- back an hour later must too. A hash of the coordinates gives that for free
-- and needs no storage. This is the one place in the game where the newly
-- seeded PRNG would be actively wrong.

local M = {}

--- How far the grid goes before the edge of the world. Not infinite in
--- practice: a coordinate space with no bound is a coordinate space where a
--- typo sends somebody to `reach.99999999.0` and the pathfinder never returns.
M.EXTENT = 40

--- The causeway's western end joins the grid here.
M.ENTRANCE = "reach.0.0"

local DIRECTIONS = {
    north = {  0,  1 },
    south = {  0, -1 },
    east  = {  1,  0 },
    west  = { -1,  0 },
}

--- A stable pseudo-random number for a coordinate pair.
---
--- Deterministic on purpose — see the header. Two multiplications and a fold,
--- which is enough scatter for choosing between four sentences and cheap enough
--- to run on every room generation.
local function hash(x, y)
    local h = (x * 73856093) + (y * 19349663)
    h = h % 2147483647
    if h < 0 then h = h + 2147483647 end
    return h
end

local WATER = {
    "Grey water in every direction, moving without going anywhere.",
    "The water here is skinned with something pale that parts and closes again.",
    "Reed stumps break the surface in rows, which means this was land.",
    "Open water, and the bottom is a long way down.",
}

local FEATURES = {
    [0] = "",
    [1] = "\r\n\r\nA post stands out of the water with a ring bolted to it.",
    [2] = "\r\n\r\nSomething broad and pale is just under the surface, keeping pace.",
    [3] = "\r\n\r\nA line of pale flat stones runs away east, laid rather than fallen.",
}

--- Parse a room id into coordinates.
--- @param room_id string
--- @return number|nil x, number|nil y
function M.coords(room_id)
    if type(room_id) ~= "string" then return nil, nil end
    local x, y = room_id:match("^reach%.(%-?%d+)%.(%-?%d+)$")
    if not x then return nil, nil end
    return tonumber(x), tonumber(y)
end

--- @return string
function M.room_id(x, y)
    return "reach." .. x .. "." .. y
end

--- @return boolean
function M.in_bounds(x, y)
    return type(x) == "number" and type(y) == "number"
        and math.abs(x) <= M.EXTENT and math.abs(y) <= M.EXTENT
end

--- Build the room for one coordinate.
---
--- Returns `nil` for anything off the grid, which is what `world_d` wants: a
--- provider that generated a room for every string would make every typo a
--- valid destination.
--- @param room_id string
--- @return table|nil
function M.generate(room_id)
    local x, y = M.coords(room_id)
    if not M.in_bounds(x, y) then return nil end
    if not (DAEMON and DAEMON.room) then return nil end

    local h = hash(x, y)
    local water = WATER[(h % #WATER) + 1]
    local feature = FEATURES[(math.floor(h / 7) % 4)]

    local exits = {}
    for dir, delta in pairs(DIRECTIONS) do
        local nx, ny = x + delta[1], y + delta[2]
        if M.in_bounds(nx, ny) then
            exits[dir] = M.room_id(nx, ny)
        end
    end
    -- The one fixed edge: the origin joins the static world, so the grid is
    -- reachable and escapable without a teleport.
    if x == 0 and y == 0 then
        exits.east = "greywater_marsh.deep_water"
    end

    return DAEMON.room.from_data({
        id    = room_id,
        short = "The Drowned Reach (" .. x .. ", " .. y .. ")",
        light = 1,
        tags  = { "outdoor", "marsh", "virtual" },
        smell = "Cold water and reed rot.",
        sound = "Water, and nothing that is not water.",
        description = water .. feature,
        exits = exits,
        items = {
            water = "Grey, opaque, and colder than the air above it.",
        },
    })
end

--- Every room adjacent to this one, for the pathfinder.
---
--- Built from the same table `generate` uses, so a graph and a walk cannot
--- disagree about where the exits are — which is the failure mode a pathfinder
--- with its own idea of the map has.
--- @param room_id string
--- @return table  direction -> room_id
function M.neighbours(room_id)
    local x, y = M.coords(room_id)
    if not M.in_bounds(x, y) then return {} end

    local out = {}
    for dir, delta in pairs(DIRECTIONS) do
        local nx, ny = x + delta[1], y + delta[2]
        if M.in_bounds(nx, ny) then out[dir] = M.room_id(nx, ny) end
    end
    if x == 0 and y == 0 then out.east = "greywater_marsh.deep_water" end
    return out
end

-- ─── Registration ────────────────────────────────────────────────────────────

if DAEMON and DAEMON.world then
    local ok, err = pcall(DAEMON.world.register_virtual, "reach", M.generate)
    if not ok then
        log("error", "REACH_D: could not register the provider: " .. tostring(err))
        if DAEMON.journal then
            pcall(DAEMON.journal.error, "REACH_D: provider registration failed: " .. tostring(err))
        end
    end
end

log("info", "reach_d loaded — the grid is " .. (M.EXTENT * 2 + 1) .. " on a side")

return M
