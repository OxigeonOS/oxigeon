-- mudlib/lib/armor.lua — The `armour` component, its archetype, and its system.
--
-- Armor used to be a class inheriting from Item; see lib/weapon.lua for why it
-- is not one any more. Same three parts:
--
--   Armor{...}                    the ARCHETYPE
--   item.armour = {...}           the COMPONENT — data, no functions
--   armour.defense(item)          the SYSTEM
--
-- The component key is `armour` and the module is `armor.lua`: the file keeps
-- the spelling every existing reference uses, and the component key avoids
-- colliding with the `armor_type` field inside it.
--
--   local Armor = require('lib.armor')
--   Armor{ id = "warded_cloak", short = "a warded cloak", slot = "back",
--          defense = 4, armor_type = "cloth", resist = { magic = 6 } }

local Item     = require('lib.item')
local requires = require('lib.requires')

local M = {}

-- Weight classes, for movement and dexterity penalties.
local ARMOR_WEIGHTS = { cloth = 0, light = 1, medium = 2, heavy = 3 }

-- ─── The component ───────────────────────────────────────────────────────────

--- Build an `armour` component from flat authoring data.
--- @param data table
--- @return table
function M.from_data(data)
    data = type(data) == "table" and data or {}

    return {
        defense    = tonumber(data.defense) or 1,
        armor_type = data.armor_type or "medium",
        -- damage_type -> reduction. Negative is a weakness: { fire = 5, ice = -3 }
        resist     = type(data.resist) == "table" and data.resist or {},
        -- trait id -> bonus, applied while worn: { max_hp = 20, strength = 2 }
        stat_bonus = type(data.stat_bonus) == "table" and data.stat_bonus or {},
    }
end

-- ─── The system ──────────────────────────────────────────────────────────────

--- Does this item carry an armour component?
--- @param item any
--- @return boolean
function M.is(item)
    return type(item) == "table" and type(item.armour) == "table"
end

--- Total defence: the component's base plus any runtime bonus.
---
--- The bonus is read from object state, which is keyed on the item's `id`.
--- While items are shared registry templates that means every copy of a
--- breastplate shares one enchantment. Per-instance identity is what fixes it.
--- @param item table
--- @return number|nil
function M.defense(item)
    if not M.is(item) then return nil end

    local bonus = 0
    if type(item.get_state) == "function" then
        local ok, stored = pcall(item.get_state, item, "defense_bonus")
        if ok then bonus = tonumber(stored) or 0 end
    end
    return item.armour.defense + bonus
end

--- Reduction for one damage type. Zero when the armour says nothing about it,
--- so callers can add the result unconditionally.
--- @param item table
--- @param damage_type string
--- @return number
function M.resist(item, damage_type)
    if not M.is(item) then return 0 end
    return tonumber(item.armour.resist[damage_type]) or 0
end

--- Weight-class tier, 0 (cloth) to 3 (heavy).
--- @param item table
--- @return number|nil
function M.encumbrance(item)
    if not M.is(item) then return nil end
    return ARMOR_WEIGHTS[item.armour.armor_type] or 1
end

--- Render a `name -> number` map as "a +2, b -1", in key order.
--- Sorted rather than iterated with `pairs`, because this reaches a player and
--- the same armour must read the same way every time.
local function signed_list(map)
    local keys = {}
    for k, v in pairs(map) do
        if type(v) == "number" and v ~= 0 then keys[#keys + 1] = k end
    end
    if #keys == 0 then return nil end
    table.sort(keys)

    local parts = {}
    for _, k in ipairs(keys) do
        local v = map[k]
        parts[#parts + 1] = k .. (v > 0 and (" +" .. v) or (" " .. v))
    end
    return table.concat(parts, ", ")
end

--- The `examine` lines for the armour component, in a fixed order.
--- @param item table
--- @return table  array of strings, empty when the item is not armour
function M.describe(item)
    if not M.is(item) then return {} end
    local a = item.armour

    local lines = {
        "Defense: " .. M.defense(item),
        "Type: " .. a.armor_type,
    }

    local resists = signed_list(a.resist)
    if resists then lines[#lines + 1] = "Resist: " .. resists end

    local bonuses = signed_list(a.stat_bonus)
    if bonuses then lines[#lines + 1] = "Bonus: " .. bonuses end

    return lines
end

-- ─── The archetype ───────────────────────────────────────────────────────────

--- Create an Item carrying an `armour` component.
--- @param data table  flat authoring data
--- @return table      an Item
function M.new(data)
    data = type(data) == "table" and data or {}

    if not data.slot then
        log("warn", "ARMOR: '" .. tostring(data.id)
            .. "' has no slot, defaulting to 'chest'")
        data.slot = "chest"
    end

    local item = Item:new(data)
    item.armour   = M.from_data(data)
    item.requires = requires.from_data(data)
    return item
end

return setmetatable(M, { __call = function(_, data) return M.new(data) end })
