-- mudlib/schema/item.lua — What an item template is, as data.
--
-- `M.components = true` is what makes component fields flatten in here. The
-- fields themselves live in `mudlib/components/*.lua` beside the `from_data`
-- that reads them, discovered rather than listed — see `components/init.lua`.

local equipment = require('lib.equipment')
local components = require('components')

local M = {}

M.kind       = "item"
M.order      = 20
M.components = true

M.fields = {
    { name = "id", type = "id", target = "item", required = true, editable = false,
      help = "Template id. Loot tables, shops and inventories all name this." },

    -- Resolved through `Object.resolve`, so a string or a function.
    { name = "short", type = "string", default = "Something", editable = true,
      lfun = true, help = "How it reads in a list: 'a corroded bone saw'." },

    { name = "description", type = "text", default = "You see nothing special.",
      editable = true, lfun = true, help = "What `examine` shows." },

    { name = "weight", type = "number", default = 1, min = 0, editable = true,
      help = "Counts against carry capacity, and against a container's." },

    { name = "value", type = "integer", default = 0, min = 0, editable = true,
      help = "What a shop pays attention to." },

    { name = "stackable", type = "boolean", default = false, editable = true },
    { name = "quantity", type = "integer", default = 1, min = 1, editable = true },

    { name = "slot", type = "enum", values = equipment.SLOTS, editable = true,
      help = "Where it is worn. Absent means it cannot be equipped." },

    { name = "equippable", type = "boolean", editable = true,
      help = "Override. Defaults to true when a slot is set." },

    -- Not the room field of the same name: this is emission, that is ambient.
    { name = "light", type = "integer", min = 0, max = 3, editable = true,
      help = "How brightly it burns when lit. NOT the room field of the same name." },

    { name = "always_lit", type = "boolean", default = false, editable = true },

    { name = "stats", type = "map", of = "number", key_source = "trait", editable = true,
      help = "Trait id -> value carried by the item itself." },

    { name = "tags", type = "string_array", default = {}, editable = true,
      help = "weapon, tool, quest, reagent. Shops match on these." },

    -- Explicit, never inferred. `speed = 1.1` on a lantern must not silently
    -- make it a weapon, and clearing `damage` must not silently un-weapon a
    -- sword — see `components.claimed`.
    { name = "components", type = "string_array", default = {}, editable = true,
      values = function() return components.names() end,
      help = "Which components this carries. Their fields follow, flat." },
}

--- Item hooks. All functions, all top-level, all `custom.lua`'s business.
M.hand_written = {
    "on_use", "on_pickup", "on_drop", "on_equip", "on_remove", "on_drink",
}

return M
