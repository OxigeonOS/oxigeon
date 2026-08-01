-- game/items/wizard_workshop_items.lua — Item definitions for the Wizard's Workshop
-- All items used in the workshop puzzle are defined here and registered
-- with ITEM_D during game init.

local Item      = require('lib.item')
local drinkable = require('components.drinkable')

local items = {}

-- ─── Purple teleportation potion ─────────────────────────────────────────────
local purple_potion = Item:new({
    id          = "purple_potion",
    short       = "a glowing purple potion",
    description = "The vial pulses with otherworldly violet light, tiny motes of starlight drifting lazily from its surface.",
    weight      = 1,
    value       = 500,
    tags        = {"magical", "potion"},
})
drinkable.apply(purple_potion, {
    drink_message      = "You uncork the glowing purple vial and drink deeply. The liquid is ice-cold and tastes of starlight and old magic. The world dissolves into a tunnel of swirling violet energy",
    drink_room_message = "{name} drinks a glowing potion and vanishes in a burst of purple light!",
    on_drink = function(item, player)
        player:send("")
        player:send("...and you rematerialize in a place you've never seen before.")
        player:send("")
        player:move_to("wizard_workshop.treasure_vault")

        -- Arrival message for anyone already in the vault
        player:message_room(player.name .. " materializes from thin air in a burst of violet sparks!")
    end,
})
items[#items + 1] = purple_potion

-- ─── Reagent potions (not drinkable — meant for mixing) ─────────────────────
local reagent_colors = {
    { color = "red",   desc = "A small vial of swirling red liquid. It gives off a faint warmth." },
    { color = "blue",  desc = "A small vial of shimmering blue liquid. It's cold to the touch." },
    { color = "green", desc = "A small vial of bubbling green liquid. Tiny sparks dance within." },
}
for _, r in ipairs(reagent_colors) do
    items[#items + 1] = Item:new({
        id          = "potion_" .. r.color,
        short       = "a vial of " .. r.color .. " liquid",
        description = r.desc,
        weight      = 1,
        tags        = {"reagent"},
    })
end

-- ─── Empty vial ──────────────────────────────────────────────────────────────
items[#items + 1] = Item:new({
    id          = "empty_vial",
    short       = "an empty crystal vial",
    description = "A small, paper-thin crystal vial for collecting alchemical samples. The glass is so thin it's almost invisible.",
    weight      = 0,
})

-- ─── Manasteel bar ───────────────────────────────────────────────────────────
items[#items + 1] = Item:new({
    id          = "manasteel_bar",
    short       = "a bar of manasteel",
    description = "An impossibly dense bar of shimmering metal, thrumming with captive energy. One of the rarest crafting materials in existence.",
    weight      = 5,
    value       = 1000,
    stackable   = true,
    tags        = {"material", "rare"},
})

return items
