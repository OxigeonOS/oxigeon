-- mudlib/components/drinkable.lua — Drinkable mixin for Item objects
-- Applies the "drinkable" behavior to any Item. Once applied, the item
-- can be consumed via the mudlib "drink" command.
--
-- Usage:
--   local drinkable = require('components.drinkable')
--   local potion = Item:new({ id = "healing_potion", short = "a healing potion", ... })
--   drinkable.apply(potion, {
--       drink_message      = "You drink the healing potion and feel warmth spread through you.",
--       drink_room_message = "{name} drinks a healing potion.",
--       on_drink           = function(item, player) player:heal(50) end,
--   })

local M = {}

--- Apply the drinkable component to an item.
-- @param item   table  The Item object to enhance
-- @param config table  Configuration options:
--   drink_message       string   Text shown to the drinker (supports {name}, {short})
--   drink_room_message  string   Text broadcast to the room (supports {name}, {short})
--   on_drink            function(item, player)  Custom callback after drinking
--   consumed            boolean  Destroy item after drinking? (default true)
--   charges             number   Number of uses before empty (default 1)
function M.apply(item, config)
    config = config or {}
    item.drinkable          = true
    item.drink_message      = config.drink_message or "You drink {short}."
    item.drink_room_message = config.drink_room_message or "{name} drinks {short}."
    item.on_drink           = config.on_drink           -- function(item, player)
    item.consumed           = config.consumed ~= false  -- default true
    item.charges            = config.charges or 1
end

return M
