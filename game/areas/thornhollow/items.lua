-- game/areas/thornhollow/items.lua — What the town sells and keeps.

local Item      = require('lib.item')
local Container = require('lib.container')
local drinkable = require('components.drinkable')

local items = {}

-- ─── Provisions ──────────────────────────────────────────────────────────────

items[#items + 1] = Item:new{
    id          = "hemp_rope",
    short       = "a coil of hemp rope",
    description = "Twenty feet of three-strand hemp, stiff with tar at both ends.",
    weight      = 5,
    value       = 12,
    tags        = { "tool" },
}

items[#items + 1] = Item:new{
    id          = "hooded_lantern",
    short       = "a hooded tin lantern",
    description = "A tin lantern with a sliding hood, so you can go dark without "
               .. "going out. There is oil in it.",
    weight      = 3,
    value       = 45,
    slot        = "light",
    -- Bright enough to read by, which is level 2 — the same scale a room uses,
    -- so a lantern in a pitch-dark mine gives you exactly a normal room.
    light       = 2,
    tags        = { "tool", "light" },
    -- `use` toggles it. The state is per instance, which is what makes two
    -- lanterns able to disagree about whether they are lit.
    on_use = function(item, char_id)
        local player = DAEMON.character and DAEMON.character.get(char_id)
        if not player then return end
        -- The instance's id, not the template's: `item.id` on a resolved
        -- pristine item *is* the template's, so the caller passes nothing
        -- useful and this asks the inventory instead.
        local Carry = require('lib.carry')
        local entry = select(1, Carry.find(player, "lantern",
            { inventory = true, room = false, equipped = true }))
        if not entry then return "You are not holding it." end

        local lit = get_object_state(entry.id, "lit") == true
        set_object_state(entry.id, "lit", not lit)
        return lit and "You slide the hood shut. The light goes out."
            or "You open the hood. Warm yellow light fills the space around you."
    end,
}

items[#items + 1] = Item:new{
    id          = "hard_rations",
    short       = "a bundle of hard rations",
    description = "Biscuit, dried fish and something that was once fruit. It "
               .. "will not spoil, which is the entire point of it.",
    weight      = 2,
    value       = 8,
    stackable   = true,
    tags        = { "food" },
}

items[#items + 1] = Item:new{
    id          = "iron_lockpick",
    short       = "a bent iron lockpick",
    description = "A strip of iron with a hook filed into one end. Hobb sells "
               .. "them as 'shelf hooks' and will say so if asked.",
    weight      = 0,
    value       = 30,
    tags        = { "tool" },
}

-- ─── Apothecary ──────────────────────────────────────────────────────────────

local healing_draught = Item:new{
    id          = "healing_draught",
    short       = "a healing draught",
    description = "A stoppered vial of something dark red that moves too slowly.",
    weight      = 1,
    value       = 60,
    tags        = { "potion", "consumable" },
}
drinkable.apply(healing_draught, {
    drink_message      = "You drink the draught. It is bitter, then warm, and the "
                      .. "warmth goes where it is needed",
    drink_room_message = "{name} drinks a dark red draught.",
    on_drink = function(item, player)
        if DAEMON and DAEMON.trait then
            DAEMON.trait.adjust(player, "hp", 35)
        end
    end,
})
items[#items + 1] = healing_draught

local marsh_antidote = Item:new{
    id          = "marsh_antidote",
    short       = "a vial of marsh antidote",
    description = "Cloudy green, and it smells like the thing it is meant to "
               .. "cure. The apothecary says that is normal.",
    weight      = 1,
    value       = 40,
    tags        = { "potion", "consumable" },
}
drinkable.apply(marsh_antidote, {
    drink_message      = "You swallow the antidote. It tastes of the marsh, which "
                      .. "is somehow worse than the poison",
    drink_room_message = "{name} drinks something cloudy and green.",
    on_drink = function(item, player)
        -- Named removal rather than a blanket clear: an antidote that also
        -- stripped your blessings would be a trap wearing a helpful label.
        if DAEMON and DAEMON.effect then
            local n = DAEMON.effect.remove(player, "marsh_poison", { reason = "antidote" })
            if n == 0 then
                player:send("Nothing in you objects to it. Perhaps you were not poisoned.")
            end
        end
    end,
})
items[#items + 1] = marsh_antidote

items[#items + 1] = Item:new{
    id          = "dried_marshroot",
    short       = "a bunch of dried marshroot",
    description = "Pale, forked and unpleasantly warm to hold. Half the "
               .. "apothecary's stock starts as this.",
    weight      = 0,
    value       = 15,
    stackable   = true,
    tags        = { "herb", "reagent" },
}

-- ─── Containers ──────────────────────────────────────────────────────────────

items[#items + 1] = Container{
    id              = "vault_chest",
    short           = "the town strongbox",
    description     = "An iron-banded chest the size of a coffin for a short "
                   .. "person. It is bolted to the plinth, which answers the "
                   .. "first question anyone asks about it.",
    weight          = 400,       -- effectively immovable, and honestly so
    value           = 0,
    capacity        = 40,
    capacity_weight = 0,          -- a town vault does not have a weight limit
    closeable       = true,
    starts_closed   = false,
    tags            = { "container", "fixed" },
}

return items
