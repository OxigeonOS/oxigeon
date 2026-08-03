-- game/areas/collapsed_mine/mobs.lua — What is still down there.
--
-- The boss is the `on_death` case: it drops a **corpse container** with its
-- loot inside rather than scattering it on the floor, and it lays a curse that
-- `survives_death` so respawning does not clear it.

return {
    {
        id          = "mine_crawler",
        name        = "crawler",
        short       = "a pale mine crawler",
        description = "The same shape as the reed crawlers in the marsh and half "
                   .. "again the size, and this one has no colour at all.",
        stats       = { hp = 55, max_hp = 55, strength = 14, dexterity = 12,
                        constitution = 13, level = 7 },
        damage      = { min = 5, max = 11 },
        xp_award    = 90,
        aggressive  = true,
        faction     = "mine",
        spawn_room  = "collapsed_mine.first_level",
        count       = 2,
        respawn_time = 300,
        tags        = { "beast", "mine" },

        loot_table = {
            { item_id = "iron_ore", chance = 0.3 },
        },
    },

    {
        id          = "shale_lurker",
        name        = "lurker",
        short       = "something under the shale",
        description = "You can see where it is by where the floor is not level. "
                   .. "That is all you can see of it.",
        stats       = { hp = 70, max_hp = 70, strength = 16, dexterity = 15,
                        constitution = 14, level = 9 },
        damage      = { min = 7, max = 13 },
        xp_award    = 130,
        aggressive  = true,
        faction     = "mine",
        spawn_room  = "collapsed_mine.deep_workings",
        count       = 1,
        respawn_time = 420,
        tags        = { "beast", "mine" },
    },

    {
        id          = "the_delver",
        name        = "delver",
        short       = "the Delver",
        description = "It is the shape of the thing that made the tool marks: "
                   .. "long-armed, working from the elbow, and much too "
                   .. "deliberate. It stops when you look at it and starts again "
                   .. "when you do not.",
        stats       = { hp = 260, max_hp = 260, strength = 22, dexterity = 14,
                        constitution = 20, intelligence = 12, wisdom = 14,
                        level = 15 },
        damage      = { min = 12, max = 22 },
        xp_award    = 900,
        aggressive  = true,
        unique      = true,
        faction     = "mine",
        spawn_room  = "collapsed_mine.the_sump",
        count       = 1,
        respawn_time = 1800,
        tags        = { "boss", "unique", "mine" },

        loot_table = {
            -- Nothing here, deliberately. The boss's loot goes into a corpse
            -- rather than onto the floor, and the loot table is the floor path.
        },

        --- A curse that outlives dying, laid at random during the fight.
        on_combat = function(mob, target)
            if DAEMON and DAEMON.effect and math.random(100) <= 20 then
                DAEMON.effect.apply(target, "delvers_regard", {
                    source = "mob:" .. tostring(mob.id),
                })
            end
        end,

        --- Death drops a **corpse**, which is a container with the loot in it.
        ---
        --- The reason a boss gets one and a rat does not: a rat drops one thing
        --- and a boss drops six, and six items on a floor is a wall of text
        --- where a corpse is one line and a decision. It is also the third kind
        --- of container — not carried, not fixed, and it goes away.
        on_death = function(mob)
            if not (DAEMON and DAEMON.items and mob.room_id) then return end

            local corpse = DAEMON.items.spawn("delver_corpse",
                DAEMON.items.location("room", mob.room_id))
            if not corpse then return end

            local into = DAEMON.items.location("item", corpse.id)
            for _, template in ipairs({
                "delvers_claw", "iron_ore", "iron_ore", "brass_key",
            }) do
                DAEMON.items.spawn(template, into)
            end

            -- The corpse rots. A ticker rather than a task, because it is one
            -- corpse and not a recurring job — and by id, so a second boss's
            -- corpse arms its own timer rather than replacing this one.
            if DAEMON.ticker then
                DAEMON.ticker.after(600, "corpse.rot." .. corpse.id, function()
                    local still = DAEMON.items.get_instance(corpse.id)
                    if still then
                        local messaging = require('lib.messaging')
                        pcall(messaging.send_to_room, mob.room_id,
                            "The corpse comes apart into things that are not a corpse.", nil)
                        DAEMON.items.destroy(still)
                    end
                end)
            end

            -- An area-wide event on the boss's death: the thing `signals.md`
            -- uses as its worked example, made real.
            if DAEMON.event then
                pcall(DAEMON.event.emit, "area.collapsed_mine.delver_slain", {
                    room_id = mob.room_id,
                    killer_char_id = mob._killed_by and mob._killed_by.char_id,
                })
            end
        end,
    },
}
