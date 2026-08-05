-- game/areas/wizard_workshop/mobs.lua — Creatures in the Wizard's Workshop.
--
-- Plain data, like rooms and items. MOB_D registers these as templates and
-- `populate()` spawns them; each spawned rat is its own Mobile with its own
-- health, its own effects and its own place in the world.
--
-- Nothing here is saved. A mob is not worth persisting — if the server
-- restarts, the rat is a new rat.

return {
    {
        id          = "workshop_rat",
        name        = "rat",
        short       = "a grey rat",
        description = "A scrawny grey rat with matted fur and clever, wary eyes. "
                   .. "It has been living well on spilled reagents, and something "
                   .. "about the way it moves suggests that was not without cost.",

        stats = {
            hp = 24, max_hp_flat = 24,
            strength = 6, dexterity = 12, constitution = 8,
            intelligence = 2, wisdom = 4,
            level = 1,
        },

        -- The template's own damage spread, used when nothing is wielded.
        damage       = { min = 2, max = 5 },
        xp_award     = 12,
        aggressive   = false,
        spawn_room   = "wizard_workshop.pantry",
        count        = 2,
        respawn_time = 120,
        tags         = { "vermin" },

        loot_table = {
            { item_id = "empty_vial", chance = 0.35 },
        },
    },

    {
        id          = "dust_mephit",
        name        = "mephit",
        short       = "a swirling dust mephit",
        description = "A knee-high creature of animate dust and static, held together "
                   .. "by a spell nobody remembered to end. It regards you with the "
                   .. "sullen resentment of something that would rather be a cloud.",

        stats = {
            hp = 40, max_hp_flat = 40,
            strength = 10, dexterity = 9, constitution = 12,
            intelligence = 6, wisdom = 8,
            level = 3,
        },

        damage       = { min = 4, max = 8 },
        xp_award     = 35,
        aggressive   = false,
        spawn_room   = "wizard_workshop.laboratory",
        count        = 1,
        respawn_time = 300,
        tags         = { "elemental", "magical" },
    },
}
