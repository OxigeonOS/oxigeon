-- mudlib/lib/weapon.lua — Weapon class
-- Inherits from Item. Represents any weapon: swords, axes, bows, staves, daggers.
--
-- Weapons add damage, speed, type classification, and combat messaging
-- on top of the base Item properties.

local Item = require('lib.item')

local Weapon = setmetatable({}, { __index = Item })
Weapon.__index = Weapon

--- Create a new Weapon from a data table.
-- @param data table  Weapon definition
-- @return table      The new Weapon
function Weapon:new(data)
    -- Default slot to "weapon" if not specified
    data.slot = data.slot or "weapon"

    local obj = Item.new(self, data)

    -- ─── Damage ──────────────────────────────────────────────────────────────
    local dmg = data.damage or {}
    if type(dmg) == "number" then
        obj.damage = { min = dmg, max = dmg }
    else
        obj.damage = {
            min = dmg.min or dmg[1] or 1,
            max = dmg.max or dmg[2] or dmg.min or dmg[1] or 1,
        }
    end

    -- ─── Combat properties ───────────────────────────────────────────────────
    obj.speed        = data.speed or 1.0       -- Attacks per round (multiplier)
    obj.weapon_type  = data.weapon_type        -- "sword", "axe", "bow", "staff", etc.
    obj.damage_type  = data.damage_type or "physical" -- "physical", "fire", "ice", "magic"
    obj.two_handed   = data.two_handed or false -- Requires both hands
    obj.range        = data.range or "melee"   -- "melee" or "ranged"

    -- ─── Requirements ────────────────────────────────────────────────────────
    obj.required_level    = data.required_level    -- Minimum level to equip
    obj.required_strength = data.required_strength -- Minimum strength to equip

    -- ─── Combat messaging (lfuns) ────────────────────────────────────────────
    obj.hit_message  = data.hit_message   -- function(weapon, attacker, target) or string
    obj.miss_message = data.miss_message  -- function(weapon, attacker, target) or string
    obj.crit_message = data.crit_message  -- function(weapon, attacker, target) or string

    return obj
end

-- ─── Damage calculation ──────────────────────────────────────────────────────

--- Roll random damage between min and max.
-- @return number
function Weapon:roll_damage()
    return math.random(self.damage.min, self.damage.max)
end

--- Get the average damage.
-- @return number
function Weapon:avg_damage()
    return (self.damage.min + self.damage.max) / 2
end

--- Get the DPS (damage per second, accounting for speed).
-- @return number
function Weapon:dps()
    return self:avg_damage() * self.speed
end

-- ─── Requirement checks ──────────────────────────────────────────────────────

--- Check if a character meets the requirements to equip this weapon.
-- @param stats table  Character stats { level, strength, ... }
-- @return boolean, string  true if met; false + reason if not
function Weapon:meets_requirements(stats)
    if self.required_level and (stats.level or 1) < self.required_level then
        return false, "Requires level " .. self.required_level
    end
    if self.required_strength and (stats.strength or 0) < self.required_strength then
        return false, "Requires " .. self.required_strength .. " strength"
    end
    return true
end

--- Get the full examination text.
-- @return string
function Weapon:examine()
    local Object = require('lib.object')
    local resolve = Object.resolve
    local parts = {}

    parts[#parts + 1] = resolve(self.short, self) or "A weapon"
    parts[#parts + 1] = resolve(self.description, self) or "You see nothing special."
    parts[#parts + 1] = "Damage: " .. self.damage.min .. "-" .. self.damage.max
    if self.weapon_type then
        parts[#parts + 1] = "Type: " .. self.weapon_type
    end
    if self.damage_type ~= "physical" then
        parts[#parts + 1] = "Element: " .. self.damage_type
    end
    parts[#parts + 1] = "Speed: " .. self.speed
    if self.two_handed then
        parts[#parts + 1] = "Two-handed"
    end
    if self.range ~= "melee" then
        parts[#parts + 1] = "Range: " .. self.range
    end
    if self.weight > 0 then
        parts[#parts + 1] = "Weight: " .. self.weight
    end
    if self.value > 0 then
        parts[#parts + 1] = "Value: " .. self.value .. " coins"
    end

    return table.concat(parts, "\r\n") .. "\r\n"
end

return Weapon
