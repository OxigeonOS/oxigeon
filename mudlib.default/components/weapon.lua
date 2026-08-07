-- mudlib/components/weapon.lua — The `weapon` component, its archetype, and its system.
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
--   local Weapon = require('components.weapon')
--   Weapon{ id = "silver_dagger", short = "a silver dagger",
--           damage = {2, 8}, speed = 1.2, damage_type = "magic",
--           required_level = 3 }

local Item     = require('lib.item')
local requires = require('components.requires')

local M = {}

--- Component identity, for `components/init.lua`.
--- `component` is the field this owns on an item; `order` is where its
--- lines sort in `examine`.
M.component = "weapon"
M.order = 10


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
        -- Physical reach: a spear gets at a giant's chest where a dagger gets
        -- at its shins. 0 adds none.
        length       = tonumber(data.length) or 0,
        -- A crossbow is not a parrying implement.
        parry        = data.parry ~= false,

        -- Trait bonuses while wielded, exactly as armour's. A sword of strength
        -- is an ordinary thing, and this is also how a weapon reaches
        -- `round_length` — `speed` says how many swings fit in a round, and
        -- this says how long the round is.
        stat_bonus   = type(data.stat_bonus) == "table" and data.stat_bonus or {},

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

--- The flat authoring fields this component reads, in emit order.
---
--- Here rather than in a central table, and discovered the way `is` and `order`
--- already are: a component describes itself by existing. `CLAUDE.md` gives the
--- reason under the trait rules — a list somewhere else of what applies where is
--- the thing that rots.
M.fields = {
    { name = "damage", type = "range", default = { min = 1, max = 1 }, editable = true,
      help = "Spread per swing. A single number means min == max." },
    { name = "speed", type = "number", default = 1.0, min = 0.1, max = 5, editable = true,
      help = "Swings per unit time; higher is faster. Multiplies into dps." },
    { name = "weapon_type", type = "string", editable = true,
      help = "dagger, sword, axe. Free text; skills and examine read it." },
    -- Physical reach, beside the rest of what this thing *is*. Feeds the window
    -- of body parts a swing can get at: a spear reaches a giant's chest where a
    -- dagger reaches its shins.
    { name = "length", type = "number", default = 0, min = 0, editable = true,
      help = "Reach in centimetres. 0 means it adds none." },
    -- A crossbow is not a parrying implement.
    { name = "stat_bonus", type = "map", of = "number", key_source = "trait",
      default = {}, editable = true,
      help = "trait id -> bonus, applied while wielded." },

    { name = "parry", type = "boolean", default = true, editable = true,
      help = "Whether this can be parried with. Feeds combat's parry channel." },
    { name = "damage_type", type = "string", default = "physical", editable = true,
      help = "What an armour resist table is matched against." },
    { name = "two_handed", type = "boolean", default = false, editable = true,
      help = "Clears the offhand when wielded." },
    { name = "range", type = "enum", values = { "melee", "thrown", "ranged" },
      default = "melee", editable = true },
    { name = "hit_message", type = "lfun", editable = false,
      help = "On a hit. {target} expands. A function belongs in custom.lua." },
    { name = "miss_message", type = "lfun", editable = false },
    { name = "crit_message", type = "lfun", editable = false },
}

--- What this component makes true of the item itself when nothing says
--- otherwise. `M.new` did this inline; `components.build` reads it from here, so
--- the loader and the archetype cannot disagree about what a weapon defaults to.
M.item_defaults = { slot = "weapon" }

--- The inverse of `from_data`.
---
--- Next to it deliberately: two halves of one mapping kept in two files is two
--- mappings, and the second one is always the one that is out of date.
--- @param item table
--- @return table|nil  flat authoring fields
function M.to_data(item)
    if not M.is(item) then return nil end
    local w = item.weapon
    return {
        damage       = { min = w.min, max = w.max },
        speed        = w.speed,
        weapon_type  = w.weapon_type,
        damage_type  = w.damage_type,
        two_handed   = w.two_handed,
        range        = w.range,
        length       = w.length,
        parry        = w.parry,
        stat_bonus   = w.stat_bonus,
        hit_message  = w.hit_message,
        miss_message = w.miss_message,
        crit_message = w.crit_message,
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

--- What wielding this contributes, as effect specs.
---
--- Discovered by `components.equip_specs`, so nothing in `lib/equipment.lua`
--- names this component — the same route armour's mitigation takes.
---
--- `lib/equipment.lua` has carried a top-level `item.stat_bonus` branch since it
--- was written, commented "a weapon may carry stat bonuses too". It was
--- unreachable: `Item:new` copies a fixed list of fields and `stat_bonus` is not
--- among them, so nothing authored could ever land there. That is the third
--- instance of the same hazard — `drinkable`'s `on_drink` and the room spawner
--- fields were the other two — and the fix here is the one armour already uses
--- rather than another entry on that list.
--- @param item table
--- @param ctx table  { trait_effect }
--- @return table  array of effect specs
function M.equip_specs(item, ctx)
    if not M.is(item) or type(ctx) ~= "table" then return {} end
    local specs = {}

    for trait_id, amount in pairs(item.weapon.stat_bonus or {}) do
        if type(amount) == "number" and amount ~= 0
            and type(ctx.trait_effect) == "function" then
            local def_id = ctx.trait_effect(trait_id)
            if def_id then
                specs[#specs + 1] = { def = def_id, state = { amount = amount } }
            end
        end
    end

    return specs
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
