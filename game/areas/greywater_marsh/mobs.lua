-- game/areas/greywater_marsh/mobs.lua — What is in the water.
--
-- `aggressive` finally means something: `aggro_d` reads it on `room.entered`.
-- The Wisp is the `unique` case *and* the `damage_type` case — it deals magic,
-- which is what makes the warded cloak's resist table worth carrying.

return {
    {
        id          = "marsh_lurker",
        name        = "lurker",
        short       = "a marsh lurker",
        description = "Long, flat and the colour of the bottom, with a mouth "
                   .. "that opens further back than a mouth should. It is only "
                   .. "visible when it moves, and it does not often move.",
        stats       = { hp = 45, max_hp_flat = 45, strength = 13, dexterity = 14,
                        constitution = 11, level = 5 },
        damage      = { min = 4, max = 9 },
        xp_award    = 60,
        -- The first template in the game where this does anything.
        aggressive  = true,
        faction     = "marsh",
        spawn_room  = "greywater_marsh.stilt_village",
        count       = 2,
        respawn_time = 240,
        tags        = { "beast", "marsh" },

        loot_table = {
            { item_id = "dried_marshroot", chance = 0.4 },
        },

        -- Biting you gives you what it has. Applied from the death-adjacent
        -- hook rather than from combat, because "this creature's attacks
        -- poison" is the creature's business and combat should not grow a
        -- special case per monster.
        on_combat = function(mob, target)
            if DAEMON and DAEMON.effect and math.random(100) <= 25 then
                DAEMON.effect.apply(target, "marsh_poison", {
                    source = "mob:" .. tostring(mob.id),
                })
            end
        end,
    },

    {
        id          = "reed_crawler",
        name        = "crawler",
        short       = "a reed crawler",
        description = "Waist high, many-legged, and it walks on the reeds "
                   .. "rather than through them. It has no eyes anyone has "
                   .. "found.",
        stats       = { hp = 30, max_hp_flat = 30, strength = 10, dexterity = 16,
                        constitution = 9, level = 3 },
        damage      = { min = 2, max = 6 },
        xp_award    = 30,
        aggressive  = true,
        faction     = "marsh",
        spawn_room  = "greywater_marsh.herb_beds",
        count       = 3,
        respawn_time = 180,
        tags        = { "beast", "marsh" },
    },

    {
        id          = "greywater_wisp",
        name        = "wisp",
        short       = "a pale wisp",
        description = "A hand-sized light with nothing holding it up, the colour "
                   .. "of a candle seen through water. It keeps exactly the "
                   .. "distance it wants and no other.",
        stats       = { hp = 80, max_hp_flat = 80, strength = 8, dexterity = 20,
                        constitution = 14, intelligence = 18, wisdom = 16,
                        level = 10 },
        damage      = { min = 8, max = 14 },
        xp_award    = 200,
        aggressive  = true,
        -- One at a time, however often `populate` runs.
        unique      = true,
        faction     = "marsh",
        spawn_room  = "greywater_marsh.deep_water",
        count       = 1,
        respawn_time = 900,
        tags        = { "magical", "unique", "marsh" },

        -- Its attacks are magic, which is the whole reason `damage_type` and
        -- `armour.resist` exist. A warded cloak visibly blunts this and does
        -- nothing at all against a lurker.
        damage_type = "magic",

        loot_table = {
            { item_id = "warded_cloak",  chance = 0.25 },
            { item_id = "silver_dagger", chance = 0.15 },
        },

        on_combat = function(mob, target)
            -- `survives_death`, so respawning does not clear it. A curse you
            -- can remove by walking into a rat is not a curse.
            if DAEMON and DAEMON.effect and math.random(100) <= 30 then
                DAEMON.effect.apply(target, "wisp_mark", {
                    source = "mob:" .. tostring(mob.id),
                })
            end
        end,

        dialogue = {
            greeting = "The light does not change. After a moment you realise "
                    .. "it has moved closer without appearing to move.",
        },
    },
}
