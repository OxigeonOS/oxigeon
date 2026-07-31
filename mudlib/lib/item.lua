-- mudlib/lib/item.lua — Item base class
-- Inherits from Object. Represents any tangible thing in the game world:
-- weapons, armor, potions, keys, treasure, junk.
--
-- Items can be picked up, dropped, equipped, and used.
-- Properties support the lfun pattern via Object.resolve().

local Object = require('lib.object')

local Item = setmetatable({}, { __index = Object })
Item.__index = Item

--- Create a new Item from a data table.
-- @param data table  Item definition
-- @return table      The new Item
function Item:new(data)
    local obj = Object.new(self, data)

    -- Core item properties
    obj.weight    = data.weight or 1       -- Affects carry capacity
    obj.value     = data.value or 0        -- Currency value (for shops, loot)
    obj.stackable = data.stackable or false -- Can multiples merge in inventory
    obj.quantity  = data.quantity or 1      -- Stack count (if stackable)

    -- Equipment
    obj.slot       = data.slot             -- nil = not equippable; "weapon", "head", "chest", etc.
    obj.equippable = data.equippable       -- Explicit override; defaults based on slot

    -- If slot is set but equippable wasn't explicitly set, default to true
    if obj.equippable == nil then
        obj.equippable = (obj.slot ~= nil)
    end

    -- Lfun hooks — called during interactions
    obj.on_use    = data.on_use            -- function(item, user_id) — "use" command
    obj.on_pickup = data.on_pickup         -- function(item, user_id) — picked up
    obj.on_drop   = data.on_drop           -- function(item, user_id) — dropped
    obj.on_equip  = data.on_equip          -- function(item, user_id) — equipped
    obj.on_remove = data.on_remove         -- function(item, user_id) — unequipped

    -- Tags for filtering and categorization
    obj.tags = data.tags or {}             -- e.g. {"quest", "fragile", "magical"}

    return obj
end

-- ─── Query methods ───────────────────────────────────────────────────────────

--- Check if this item can be equipped.
-- @return boolean
function Item:is_equippable()
    return self.equippable == true
end

--- Check if this item is stackable.
-- @return boolean
function Item:is_stackable()
    return self.stackable == true
end

--- Check if this item has a specific tag.
-- @param tag string
-- @return boolean
function Item:has_tag(tag)
    for _, t in ipairs(self.tags) do
        if t == tag then return true end
    end
    return false
end

--- Get the display name, including quantity for stacks.
-- @return string
function Item:display_name()
    local name = Object.resolve(self.short, self) or "something"
    if self.stackable and self.quantity > 1 then
        return name .. " (x" .. self.quantity .. ")"
    end
    return name
end

--- Get the full examination text.
-- @return string
function Item:examine()
    local parts = {}
    local resolve = Object.resolve

    parts[#parts + 1] = resolve(self.short, self) or "Something"
    parts[#parts + 1] = resolve(self.description, self) or "You see nothing special."

    if self.weight > 0 then
        parts[#parts + 1] = "Weight: " .. self.weight
    end
    if self.value > 0 then
        parts[#parts + 1] = "Value: " .. self.value .. " coins"
    end
    if self.slot then
        parts[#parts + 1] = "Slot: " .. self.slot
    end

    return table.concat(parts, "\r\n") .. "\r\n"
end

return Item
