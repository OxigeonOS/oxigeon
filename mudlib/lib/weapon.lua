-- mudlib/lib/weapon.lua — The `weapon` component, its archetype, and its system.
--
-- Weapon used to be a class inheriting from Item. It is not one any more.
-- Every method it had was a pure function of the item's own data, so the
-- inheritance bought the method-call syntax and nothing else — while an item
-- that is a weapon *and* a light source *and* a quest token had no single class
-- it could be. See docs/src/lua-api/components.md.
--
-- Three things live in this file, and keeping them apart is the whole point:
--
--   Weapon{...}                 the ARCHETYPE — flat authoring data in,
--                               an Item carrying components out
--   item.weapon = {...}         the COMPONENT — data, no functions
--   weapon.roll_damage(item)    the SYSTEM — behaviour, as module functions
--
-- Authoring is unchanged from the class it replaces:
--
--   local Weapon = require('lib.weapon')
--   Weapon{ id = "silver_dagger", short = "a silver dagger",
--           damage = {2, 8}, speed = 1.2, damage_type = "magic",
--           required_level = 3 }

local Item     = require('lib.item')
local requires = require('lib.requires')

local M = {}

-- ─── The component ───────────────────────────────────────────────────────────

--- Normalise a damage spread. Accepts `8`, `{2, 8}` or `{min = 2, max = 8}`,
--- which are all in use in authored content, and coerces to numbers so a typo
--- in an area file cannot reach `math.random` as a string.
local function spread_of(dmg)
    if type(dmg) == "number" then
        return dmg, dmg
    end
    if type(dmg) ~= "table" then
        return 1, 1
    end
    local min = tonumber(dmg.min or dmg[1]) or 1
    local max = tonumber(dmg.max or dmg[2]) or min
    if max < min then min, max = max, min end
    return min, max
end

--- Build a `weapon` component from flat authoring data.
--- @param data table
--- @return table
function M.from_data(data)
    data = type(data) == "table" and data or {}
    local min, max = spread_of(data.damage)

    return {
        min          = min,
        max          = max,
        speed        = tonumber(data.speed) or 1.0,
        weapon_type  = data.weapon_type,
        damage_type  = data.damage_type or "physical",
        two_handed   = data.two_handed or false,
        range        = data.range or "melee",

        -- These three are lfuns — a string or a function returning one. They
        -- are the one part of this component that may hold a function, and it
        -- is safe only because they live on a TEMPLATE, which is code and is
        -- never serialized. An item *instance* must not carry them: see the
        -- definitions-vs-instances rule in effect_d.lua.
        hit_message  = data.hit_message,
        miss_message = data.miss_message,
        crit_message = data.crit_message,
    }
end

-- ─── The system ──────────────────────────────────────────────────────────────

--- Does this item carry a weapon component?
--- @param item any
--- @return boolean
function M.is(item)
    return type(item) == "table" and type(item.weapon) == "table"
end

--- Roll the damage for one swing.
---
--- `roll` is injected rather than reached for, because the class version called
--- `math.random` directly. That made it a second source of randomness, and
--- combat_d's contract is that `DAEMON.combat._roll` is the only one — so a
--- test that pinned the roll still got random weapon damage. Callers pass
--- `M._roll`; the default keeps the function usable on its own.
--- @param item table
--- @param roll function|nil  `f(n) -> 1..n`
--- @return number|nil        nil when the item is not a weapon
function M.roll_damage(item, roll)
    if not M.is(item) then return nil end
    local w = item.weapon
    roll = roll or function(n) return math.random(1, n) end
    return w.min + roll(math.max(1, w.max - w.min + 1)) - 1
end

--- Mean damage per swing.
--- @param item table
--- @return number|nil
function M.avg_damage(item)
    if not M.is(item) then return nil end
    return (item.weapon.min + item.weapon.max) / 2
end

--- Mean damage scaled by attack speed.
--- @param item table
--- @return number|nil
function M.dps(item)
    if not M.is(item) then return nil end
    return M.avg_damage(item) * item.weapon.speed
end

--- The `examine` lines for the weapon component, in a fixed order.
--- @param item table
--- @return table  array of strings, empty when the item is not a weapon
function M.describe(item)
    if not M.is(item) then return {} end
    local w = item.weapon

    local lines = { "Damage: " .. w.min .. "-" .. w.max }
    if w.weapon_type then
        lines[#lines + 1] = "Type: " .. w.weapon_type
    end
    if w.damage_type ~= "physical" then
        lines[#lines + 1] = "Element: " .. w.damage_type
    end
    lines[#lines + 1] = "Speed: " .. w.speed
    if w.two_handed then
        lines[#lines + 1] = "Two-handed"
    end
    if w.range ~= "melee" then
        lines[#lines + 1] = "Range: " .. w.range
    end
    return lines
end

-- ─── The archetype ───────────────────────────────────────────────────────────

--- Create an Item carrying a `weapon` component.
--- @param data table  flat authoring data
--- @return table      an Item
function M.new(data)
    data = type(data) == "table" and data or {}
    data.slot = data.slot or "weapon"

    local item = Item:new(data)
    item.weapon   = M.from_data(data)
    item.requires = requires.from_data(data)
    return item
end

-- `Weapon{...}` reads better in an area file than `Weapon.new{...}`, and both
-- work. There is deliberately no `Weapon:new(...)` — the colon form would
-- suggest a class, which is what this stopped being.
return setmetatable(M, { __call = function(_, data) return M.new(data) end })
