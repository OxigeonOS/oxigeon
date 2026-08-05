-- mudlib/components/drinkable.lua — The `drinkable` component and its system.
--
-- The same three parts as `weapon` and `armour`, which this did not have: it
-- splatted five flat fields onto the item's top level, had no `is`, and left
-- its message formatting inside `cmds/drink.lua`, so `examine` on a potion said
-- nothing at all about it being drinkable.
--
--   drinkable.apply(item, {...})   the ARCHETYPE — applied to an existing Item
--   item.drinkable = {...}         the COMPONENT — data, no functions
--   drinkable.messages(item, who)  the SYSTEM   — module functions
--
-- `apply` rather than a constructor, unlike Weapon and Armor: a healing draught
-- is an ordinary `Item` that happens to be drinkable, and it may also be a quest
-- token or a light source. That is exactly the case components exist for.
--
--   local drinkable = require('components.drinkable')
--   local potion = Item:new{ id = "healing_potion", short = "a healing potion" }
--   drinkable.apply(potion, {
--       drink_message = "You drink it and feel warmth spread through you.",
--       on_drink      = function(item, player) player:heal(50) end,
--   })
--
-- See docs/src/lua-api/components.md.

local M = {}

--- Component identity, for `components/init.lua`.
--- `component` is the field this owns on an item; `order` is where its
--- lines sort in `examine`.
M.component = "drinkable"
M.order = 40

-- ─── The component ───────────────────────────────────────────────────────────

--- Build a `drinkable` component from flat authoring data.
--- @param data table
---   drink_message       string    shown to the drinker; {name} and {short} expand
---   drink_room_message  string    shown to everyone else
---   consumed            boolean   destroy the item afterwards (default true)
--- @return table
function M.from_data(data)
    data = type(data) == "table" and data or {}
    return {
        drink_message      = data.drink_message or "You drink {short}.",
        drink_room_message = data.drink_room_message or "{name} drinks {short}.",
        -- Default true: the overwhelmingly common case is a potion that goes
        -- away, and a bottomless one should have to say so.
        consumed           = data.consumed ~= false,
    }
end

-- ─── The archetype ───────────────────────────────────────────────────────────

--- Make an existing item drinkable.
---
--- `on_drink` stays a top-level field rather than going inside the component,
--- for the reason `weapon.lua` gives about its message lfuns: a component holds
--- data, and functions belong on the template. Item hooks (`on_use`,
--- `on_pickup`, …) already live at the top level and are read from the
--- template, so this follows the existing rule rather than adding an exception.
--- @param item table
--- @param config table|nil
--- @return table the same item, for chaining
function M.apply(item, config)
    if type(item) ~= "table" then
        log("error", "DRINKABLE.apply: needs an item")
        return item
    end
    config = config or {}
    item.drinkable = M.from_data(config)
    item.on_drink = config.on_drink
    return item
end

-- ─── The system ──────────────────────────────────────────────────────────────

--- Does this item carry a drinkable component?
--- @param item any
--- @return boolean
function M.is(item)
    return type(item) == "table" and type(item.drinkable) == "table"
end

--- Is it destroyed by drinking it?
--- @param item table
--- @return boolean
function M.is_consumed(item)
    return M.is(item) and item.drinkable.consumed == true
end

--- The two messages a drink produces, with `{name}` and `{short}` expanded.
---
--- Here rather than in `cmds/drink.lua` so that anything else able to make a
--- character drink — a forced potion, a trap, a mob — words it identically.
--- @param item table
--- @param drinker table|nil
--- @return string to_drinker, string to_room
function M.messages(item, drinker)
    local d = M.is(item) and item.drinkable or M.from_data(nil)
    local name = (type(drinker) == "table" and drinker.name) or "Someone"
    local short = (type(item) == "table"
        and ((type(item.short) == "string" and item.short) or item.id)) or "it"

    local function fill(template)
        return (template:gsub("{name}", name):gsub("{short}", short))
    end
    return fill(d.drink_message), fill(d.drink_room_message)
end

--- The `examine` lines. Unindented — the caller owns layout.
--- @param item table
--- @return table  array of strings
function M.describe(item)
    if not M.is(item) then return {} end
    if M.is_consumed(item) then
        return { "You could drink this." }
    end
    return { "You could drink this, and it would not run out." }
end

return M
