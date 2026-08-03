-- mudlib/lib/light.lua — Whether you can see.
--
-- `Room.light_level` has been a field since rooms existed and **nothing read
-- it**: every room was equally visible, and the field documented an intention.
-- This is what reads it, and the shape is deliberately small — one question,
-- one answer, one place to change it.
--
--   0  pitch dark   you see nothing without a light of your own
--   1  dim          you can see, but not detail
--   2  normal
--   3  bright
--
-- ─── What counts as a light ──────────────────────────────────────────────────
--
-- An item is a light when it is **lit**, which is per-instance object state and
-- not a property of the template: two lanterns must be able to disagree about
-- whether they are burning. A template may also declare `always_lit` for
-- something that cannot be put out — a glowing sword, a torch that is only ever
-- alight.
--
-- Carried counts as well as equipped. Insisting a lantern be in the `light`
-- slot before it lights anything is a rule players discover by dying in the
-- dark, and it buys nothing.

local M = {}

--- The level below which a room is dark.
M.DARK = 0

--- Is this item currently giving light?
--- @param entry table   the instance
--- @param item table     the resolved item
--- @return boolean
function M.is_lit(entry, item)
    if type(item) ~= "table" then return false end
    if item.always_lit then return true end
    if type(entry) ~= "table" or type(entry.id) ~= "string" then return false end
    if type(get_object_state) ~= "function" then return false end

    local ok, lit = pcall(get_object_state, entry.id, "lit")
    return ok and lit == true
end

--- Everything an entity is carrying or wearing that is currently alight.
--- @param entity table
--- @return table  array of resolved items
function M.sources(entity)
    local out = {}
    if type(entity) ~= "table" or not (DAEMON and DAEMON.items) then return out end

    local function consider(entry)
        if type(entry) ~= "table" then return end
        local item = DAEMON.items.resolve(entry)
        if M.is_lit(entry, item) then out[#out + 1] = item end
    end

    for _, entry in ipairs(entity.inventory or {}) do consider(entry) end
    for _, entry in pairs(entity.equipment or {}) do consider(entry) end
    return out
end

--- How much light this entity brings with it.
--- @param entity table
--- @return number
function M.carried(entity)
    local best = 0
    for _, item in ipairs(M.sources(entity)) do
        best = math.max(best, tonumber(item.light) or 2)
    end
    return best
end

--- Can this entity see in this room?
---
--- A room's *effective* light, so weather counts — a fogbound marsh at night is
--- as dark as a mine, and it would be strange for one to need a lantern and the
--- other not.
--- @param entity table
--- @param room table
--- @return boolean can_see, number level
function M.can_see(entity, room)
    if type(room) ~= "table" then return true, 2 end

    local level = room.effective_light and room:effective_light()
        or room.light_level or 2
    if level > M.DARK then return true, level end

    local carried = M.carried(entity)
    if carried > M.DARK then return true, carried end
    return false, 0
end

--- What to say when they cannot see. A sentence rather than a refusal, because
--- being in the dark is a situation rather than an error.
M.DARKNESS = "It is pitch dark. You can feel a floor under you and nothing else."

return M
