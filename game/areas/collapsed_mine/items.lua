-- game/areas/collapsed_mine/items.lua

local Item      = require('lib.item')
local Weapon    = require('lib.weapon')
local Container = require('lib.container')

local items = {}

items[#items + 1] = Item:new{
    id          = "iron_ore",
    short       = "a lump of iron ore",
    description = "Heavy for its size, rust-coloured, and warm on one face where "
               .. "it came off the seam.",
    weight      = 4,
    value       = 25,
    stackable   = true,
    tags        = { "ore", "reagent" },
}

items[#items + 1] = Weapon{
    id          = "miners_pick",
    short       = "a long-handled miner's pick",
    description = "Four feet of ash with a steel head, one point and one blade. "
               .. "It is a tool that a desperate person can use twice.",
    slot        = "weapon",
    weight      = 9,
    value       = 55,
    damage      = { min = 6, max = 12 },
    speed       = 0.8,
    weapon_type = "pick",
    two_handed  = true,
    required_strength = 13,
    hit_message = "You bring the pick down on {target} point-first.",
    tags        = { "weapon", "tool" },
}

items[#items + 1] = Weapon{
    id          = "delvers_claw",
    short       = "a Delver's claw",
    description = "Taken off the thing itself: a foot of something between horn "
               .. "and slate, still warm. Held at the base it is a weapon, and "
               .. "held anywhere else it is a mistake.",
    slot        = "weapon",
    weight      = 5,
    value       = 800,
    damage      = { min = 11, max = 19 },
    speed       = 1.0,
    weapon_type = "claw",
    required_level = 12,
    hit_message = "The claw goes into {target} the way it was made to.",
    crit_message = "The claw finds the gap it was looking for.",
    tags        = { "weapon", "boss" },
}

-- The corpse: the third kind of container. Not carried, not fixed, and it goes
-- away. `capacity = 0` means unlimited — a boss that dropped eleven things
-- should not silently lose the eleventh.
items[#items + 1] = Container{
    id              = "delver_corpse",
    short           = "the Delver's corpse",
    description     = "Long-armed and much too deliberate even now. Something in "
                   .. "it is still warm.",
    weight          = 300,
    value           = 0,
    capacity        = 0,
    closeable       = false,
    tags            = { "container", "corpse" },
}

return items
