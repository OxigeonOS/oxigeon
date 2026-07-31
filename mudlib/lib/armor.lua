-- mudlib/lib/armor.lua — Armor class
-- Inherits from Item. Represents any defensive equipment:
-- helmets, breastplates, shields, boots, gloves, robes.
--
-- Armor provides defense values, damage resistances, and slot-specific
-- properties on top of the base Item.

local Item = require('lib.item')

local Armor = setmetatable({}, { __index = Item })
Armor.__index = Armor

--- Create a new Armor from a data table.
-- @param data table  Armor definition
-- @return table      The new Armor
function Armor:new(data)
    -- Armor must have a slot
    if not data.slot then
        log("warn", "ARMOR: Created armor '" .. tostring(data.id)
            .. "' without a slot, defaulting to 'chest'")
    end
    data.slot = data.slot or "chest"

    local obj = Item.new(self, data)

    -- ─── Defense ─────────────────────────────────────────────────────────────
    obj.defense     = data.defense or 1        -- Base damage reduction
    obj.armor_type  = data.armor_type or "medium" -- "cloth", "light", "medium", "heavy"

    -- ─── Resistances ─────────────────────────────────────────────────────────
    -- Table of damage_type → reduction value (flat or percentage)
    -- e.g., { fire = 5, ice = -3 } (ice weakness)
    obj.resist = data.resist or {}

    -- ─── Requirements ────────────────────────────────────────────────────────
    obj.required_level      = data.required_level      -- Minimum level to equip
    obj.required_strength   = data.required_strength   -- For heavy armor
    obj.required_dexterity  = data.required_dexterity  -- For light armor

    -- ─── Passive effects ─────────────────────────────────────────────────────
    -- Table of stat_name → bonus value applied while equipped
    -- e.g., { max_hp = 20, strength = 2 }
    obj.stat_bonus = data.stat_bonus or {}

    return obj
end

-- ─── Defense calculation ─────────────────────────────────────────────────────

--- Get the total defense value (base + any state modifiers).
-- @return number
function Armor:get_defense()
    local bonus = 0
    local state_bonus = self:get_state("defense_bonus")
    if state_bonus then
        bonus = tonumber(state_bonus) or 0
    end
    return self.defense + bonus
end

--- Get the resistance value for a damage type.
-- @param damage_type string  e.g. "fire", "ice", "magic"
-- @return number             Resistance value (0 if none)
function Armor:get_resist(damage_type)
    return self.resist[damage_type] or 0
end

-- ─── Requirement checks ──────────────────────────────────────────────────────

--- Check if a character meets the requirements to equip this armor.
-- @param stats table  Character stats { level, strength, dexterity, ... }
-- @return boolean, string  true if met; false + reason if not
function Armor:meets_requirements(stats)
    if self.required_level and (stats.level or 1) < self.required_level then
        return false, "Requires level " .. self.required_level
    end
    if self.required_strength and (stats.strength or 0) < self.required_strength then
        return false, "Requires " .. self.required_strength .. " strength"
    end
    if self.required_dexterity and (stats.dexterity or 0) < self.required_dexterity then
        return false, "Requires " .. self.required_dexterity .. " dexterity"
    end
    return true
end

-- ─── Armor type helpers ──────────────────────────────────────────────────────

--- Armor type weight classes for movement/dexterity penalties.
local ARMOR_WEIGHTS = {
    cloth  = 0,
    light  = 1,
    medium = 2,
    heavy  = 3,
}

--- Get the encumbrance tier of this armor (0-3).
-- @return number
function Armor:encumbrance()
    return ARMOR_WEIGHTS[self.armor_type] or 1
end

--- Get the full examination text.
-- @return string
function Armor:examine()
    local Object = require('lib.object')
    local resolve = Object.resolve
    local parts = {}

    parts[#parts + 1] = resolve(self.short, self) or "A piece of armor"
    parts[#parts + 1] = resolve(self.description, self) or "You see nothing special."
    parts[#parts + 1] = "Defense: " .. self.defense
    parts[#parts + 1] = "Type: " .. self.armor_type
    parts[#parts + 1] = "Slot: " .. self.slot

    -- Show resistances
    local resist_parts = {}
    for dtype, val in pairs(self.resist) do
        if val > 0 then
            resist_parts[#resist_parts + 1] = dtype .. " +" .. val
        elseif val < 0 then
            resist_parts[#resist_parts + 1] = dtype .. " " .. val
        end
    end
    if #resist_parts > 0 then
        parts[#parts + 1] = "Resist: " .. table.concat(resist_parts, ", ")
    end

    -- Show stat bonuses
    local bonus_parts = {}
    for stat, val in pairs(self.stat_bonus) do
        if val > 0 then
            bonus_parts[#bonus_parts + 1] = stat .. " +" .. val
        elseif val < 0 then
            bonus_parts[#bonus_parts + 1] = stat .. " " .. val
        end
    end
    if #bonus_parts > 0 then
        parts[#parts + 1] = "Bonus: " .. table.concat(bonus_parts, ", ")
    end

    if self.weight > 0 then
        parts[#parts + 1] = "Weight: " .. self.weight
    end
    if self.value > 0 then
        parts[#parts + 1] = "Value: " .. self.value .. " coins"
    end

    return table.concat(parts, "\r\n") .. "\r\n"
end

return Armor
