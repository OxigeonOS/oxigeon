-- game/areas/wizard_workshop/gear.lua — Something to wear, wield and carry.
--
-- The entire `Weapon`/`Armor` half of the object model had **zero instances**
-- anywhere: the archetypes existed, the components existed, the systems over
-- them existed, and no item in the game was one. Nothing read `armour.defense`,
-- `armour.resist` or `armour.stat_bonus`, `requires` had no refusal to make,
-- and `equipment` was a slot map nothing wrote.
--
-- This is the smallest set of gear that makes each of those real:
--
--   apprentice_dagger   a weapon anybody can use — the baseline
--   iron_greatsword     two-handed, `required_strength` — the refusal path,
--                       and the thing that clears the offhand
--   silver_dagger       `damage_type = "magic"` — a damage type that a resist
--                       table can actually meet
--   leather_jerkin      plain armour — damage numbers visibly drop
--   warded_cloak        `resist = { magic = 6 }` — mitigation by type
--   scholar_circlet     `stat_bonus = { intelligence = 2 }` — an `equip:`
--                       effect source, and proof that `score` shows it
--   oak_buckler         offhand — what a two-handed weapon displaces
--   leather_backpack    a container, so `put` and `get from` have a subject

local Weapon    = require('components.weapon')
local Armor     = require('components.armor')
local Container = require('components.container')

local gear = {}

-- ─── Weapons ─────────────────────────────────────────────────────────────────

gear[#gear + 1] = Weapon{
    id          = "apprentice_dagger",
    short       = "an apprentice's dagger",
    description = "A short blade with a chipped edge and a grip wound in cracked "
               .. "leather. The kind of thing given to someone who is not yet "
               .. "trusted with anything better.",
    slot        = "weapon",
    weight      = 2,
    value       = 15,
    damage      = { min = 2, max = 5 },
    speed       = 1.2,
    weapon_type = "dagger",
    hit_message = "You slip the dagger past {target}'s guard.",
    miss_message = "Your dagger scrapes off {target}'s side.",
    crit_message = "{target} does not see the dagger at all until it is in.",
    tags        = { "weapon", "starter" },
}

gear[#gear + 1] = Weapon{
    id          = "iron_greatsword",
    short       = "a pitted iron greatsword",
    description = "Four feet of unfashionable iron with a two-handed grip. It has "
               .. "been sharpened so many times the blade has gone narrow.",
    slot        = "weapon",
    weight      = 12,
    value       = 200,
    damage      = { min = 8, max = 16 },
    speed       = 0.7,
    weapon_type = "sword",
    two_handed  = true,
    -- The refusal path. One rule, in `components/requires.lua`, shared with armour.
    required_strength = 16,
    hit_message = "You bring the greatsword down on {target} with both hands.",
    tags        = { "weapon" },
}

gear[#gear + 1] = Weapon{
    id          = "silver_dagger",
    short       = "a silver ritual dagger",
    description = "The blade is silver rather than steel, too soft to hold an "
               .. "edge and far too cold to be only metal. It hums faintly near "
               .. "anything that should not exist.",
    slot        = "weapon",
    weight      = 2,
    value       = 400,
    damage      = { min = 3, max = 7 },
    speed       = 1.1,
    weapon_type = "dagger",
    -- Meets a resist table on the other side. Without a damage type that is
    -- not "physical", `armour.resist` has nothing to be about.
    damage_type = "magic",
    required_level = 3,
    hit_message = "The silver blade bites into {target} and hisses.",
    tags        = { "weapon", "magical" },
}

-- ─── Armour ──────────────────────────────────────────────────────────────────

gear[#gear + 1] = Armor{
    id          = "leather_jerkin",
    short       = "a scuffed leather jerkin",
    description = "Boiled leather, cut for someone slightly larger. It will stop "
               .. "a knife once.",
    slot        = "chest",
    weight      = 6,
    value       = 40,
    defense     = 3,
    armor_type  = "light",
    tags        = { "armour" },
}

gear[#gear + 1] = Armor{
    id          = "warded_cloak",
    short       = "a warded travelling cloak",
    description = "Grey wool, unremarkable except for the thread worked into the "
               .. "hem in a script that makes your eyes slide off it. The air "
               .. "around the wearer goes very slightly quiet.",
    slot        = "back",
    weight      = 3,
    value       = 350,
    defense     = 1,
    armor_type  = "cloth",
    -- Blunts the silver dagger and does nothing at all against a sword, which
    -- is what makes `damage_type` worth having.
    resist      = { magic = 6 },
    tags        = { "armour", "magical" },
}

gear[#gear + 1] = Armor{
    id          = "scholar_circlet",
    short       = "a thin silver circlet",
    description = "A student's circlet, worn to keep the hair out of the eyes and "
               .. "the mind out of the ditch. It is warm to the touch.",
    slot        = "head",
    weight      = 1,
    value       = 250,
    defense     = 0,
    armor_type  = "cloth",
    -- Becomes an `equip:head` effect source while worn, and vanishes with the
    -- circlet. Never persisted: what is worn is saved, and the aura is derived
    -- from that.
    stat_bonus  = { intelligence = 2 },
    tags        = { "armour", "magical" },
}

gear[#gear + 1] = Armor{
    id          = "oak_buckler",
    short       = "a small oak buckler",
    description = "A hand-sized shield of banded oak, scarred all over its face.",
    slot        = "offhand",
    weight      = 4,
    value       = 60,
    defense     = 2,
    armor_type  = "light",
    tags        = { "armour", "shield" },
}

-- ─── Containers ──────────────────────────────────────────────────────────────

gear[#gear + 1] = Container{
    id              = "leather_backpack",
    short           = "a battered leather backpack",
    description     = "Three buckles, two of which still work. It smells of old "
                   .. "rope and older bread.",
    slot            = "back",
    weight          = 2,
    value           = 25,
    capacity        = 12,
    capacity_weight = 40,
    tags            = { "container" },
}

gear[#gear + 1] = Container{
    id              = "iron_strongbox",
    short           = "a small iron strongbox",
    description     = "A squat box with a keyhole and no hinges anyone can see. "
                   .. "Heavier empty than it looks.",
    weight          = 15,
    value           = 80,
    capacity        = 6,
    closeable       = true,
    starts_closed   = true,
    key             = "brass_key",
    starts_locked   = true,
    tags            = { "container" },
}

gear[#gear + 1] = require('lib.item'):new{
    id          = "brass_key",
    short       = "a small brass key",
    description = "Barely longer than a thumb, with two teeth and a square bow.",
    weight      = 0,
    value       = 5,
    tags        = { "key" },
}

return gear
