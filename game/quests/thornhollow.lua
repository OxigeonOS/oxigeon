-- game/quests/thornhollow.lua — Five quests, one per persistence shape.
--
-- Deliberately one of each, because the interesting thing about a quest system
-- is that it needs all three tiers at once and choosing wrongly is invisible
-- until it is not:
--
--   fetch        collect items      -> counted from what you are holding
--   kill-count   a write-behind counter, which is what that tier is for
--   delivery     cross-area, on `room.entered`
--   daily        a **durable** cooldown, which survives an area reset — the
--                bug `task_list.md` opens with, stated as a quest
--   chain        gated on another quest's flag, which is a SAVE_FIELD

return {
    --- FETCH. The counter is recomputed from the inventory rather than
    --- incremented, because an item picked up, dropped and picked up again is
    --- one item and a counter would call it two.
    {
        id      = "roots_for_the_apothecary",
        name    = "Roots for the Apothecary",
        summary = "Bring the apothecary three bunches of marshroot.",
        giver   = "town_apothecary",
        level   = 1,
        objective = { kind = "collect", target = "dried_marshroot", count = 3 },
        reward  = { xp = 120, gold = 45, skill = "herbalism", skill_amount = 2 },
        completion = "\"That will do. Mind your head on the way out.\"",
        repeatable = true,
    },

    --- KILL-COUNT. The counter is the write-behind tier's worked example:
    --- written on every kill, read almost never, and worth losing thirty
    --- seconds of on a crash.
    {
        id      = "thin_the_crawlers",
        name    = "Thin the Crawlers",
        summary = "Kill five reed crawlers in the marsh. The guard would rather "
               .. "you did it than they did.",
        giver   = "town_guard",
        level   = 2,
        objective = { kind = "kill", target = "reed_crawler", count = 5 },
        reward  = { xp = 300, gold = 90, items = { "healing_draught" } },
        completion = "\"Right. That's five fewer.\"",
        repeatable = true,
    },

    --- DELIVERY. Cross-area, and the objective completes on `room.entered` —
    --- so the quest system never has to be told, the same way a room
    --- description never has to be told about the weather.
    {
        id      = "word_to_the_deep",
        name    = "Word to the Deep",
        summary = "Bellow wants to know whether the pump house is still standing. "
               .. "Get to it and look.",
        giver   = "town_smith",
        level   = 4,
        objective = { kind = "visit", target = "collapsed_mine.pump_house", count = 1 },
        reward  = { xp = 250, gold = 60, items = { "hooded_lantern" } },
        completion = "\"Still there. Good. That's the last of the good news.\"",
    },

    --- DAILY. A durable cooldown, over the threshold, so it is written through
    --- and survives both a restart and an area reset. **Not** room object
    --- state, which an area reset wipes — that is the original bug, and this is
    --- the shape that does not have it.
    {
        id      = "the_days_ore",
        name    = "The Day's Ore",
        summary = "Bring Bellow a lump of iron ore. Once a day; she will not "
               .. "take two.",
        giver   = "town_smith",
        level   = 3,
        objective = { kind = "collect", target = "iron_ore", count = 1 },
        reward  = { xp = 150, gold = 120, skill = "mining", skill_amount = 1 },
        completion = "\"Same again tomorrow, if you're still going down there.\"",
        repeatable = "daily",
    },

    --- CHAIN. Gated on the flag the delivery quest sets, and the flag is a
    --- SAVE_FIELD — a forever answer, in the tier for forever answers.
    {
        id      = "what_is_down_there",
        name    = "What Is Down There",
        summary = "Something made the tool marks in the deep workings. Bellow "
               .. "wants it dead, and is not pretending otherwise.",
        giver   = "town_smith",
        level   = 10,
        requires = { flag = "quest.done.word_to_the_deep", level = 10 },
        objective = { kind = "kill", target = "the_delver", count = 1 },
        reward  = {
            xp = 2000, gold = 800,
            items = { "warded_cloak" },
            effect = "insight", effect_duration = 3600,
        },
        completion = "Bellow puts the hammer down, which she does not do.\r\n"
                  .. "\"Right,\" she says. \"Right.\"",
        on_complete = function(player)
            -- A flag another part of the game can read. The chain's own gate
            -- uses `quest.done.*`, which `quest_d` sets; this is a *world*
            -- fact rather than a quest fact, and worth its own name.
            player:set_quest_flag("mine_reopened", true)
        end,
    },
}
