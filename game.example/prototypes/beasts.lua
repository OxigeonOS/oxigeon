-- game/prototypes/beasts.lua — What the creatures of this game have in common.
--
-- Hand-written, like `custom.lua`, and for the same reason: these hold
-- functions. OLC never reads or writes this file. Edits take effect on
-- `areas reset`.
--
-- ─── What this replaced ──────────────────────────────────────────────────────
--
-- `mine_crawler`, `shale_lurker`, `reed_crawler` and `marsh_lurker` were four
-- copies of one twelve-key skeleton with four numbers changed. Changing what a
-- crawler *is* meant finding every crawler, in two areas, and getting all of
-- them.
--
-- ─── The chain, and why it is shaped this way ────────────────────────────────
--
--     beast                 what every creature here is
--       ├ mine.beast        + faction and tags
--       │   ├ mine.crawler  + the noun
--       │   └ mine.lurker
--       ├ marsh.beast
--       │   ├ marsh.crawler
--       │   └ marsh.venomous  + a bite that carries something
--       └ vermin.rat        what a rat is, and what the workshop nest makes
--           ├ vermin.rat.black
--           ├ vermin.rat.scrawny
--           └ vermin.rat.muscular
--
-- A crawler is a crawler in both areas, and the area is not the noun — so there
-- are two things worth inheriting from and inheritance is single. The area wins,
-- because it carries three fields and the noun carries one. That is the whole
-- of the trade, and it is the one the doc means by *"if you want two parents,
-- you want a component, or a third prototype that names one of them."*
--
-- Dotted names are a naming convention and nothing else: `mine.crawler` does not
-- implicitly inherit `mine`, and every `prototype = ...` below is written out.
-- Implicit hierarchy would mean a rename silently reparents.

--- A bite that carries something, as a shared hook.
---
--- Two creatures had a near-identical private copy of this — `lurker_bite` in
--- the marsh and `delver_on_combat` in the mine — differing only in which effect
--- and how often. The effect id is closed over rather than read from a field on
--- the creature, because a field no schema declares is kept and *reported*, and
--- a note on every venomous thing in the game is a linter people stop reading.
local function bite_carrying(effect_id, chance)
    return function(mob, target)
        if not (DAEMON and DAEMON.effect) then return end
        if math.random(100) > chance then return end
        DAEMON.effect.apply(target, effect_id, {
            source = "mob:" .. tostring(mob.id),
        })
    end
end

return {
    mobs = {
        -- The root. Nothing here is about any one creature: it is what "a beast
        -- in this game" means, and it is the thing you edit when that changes.
        ["beast"] = {
            race        = "beast",
            aggressive  = true,
            damage_type = "physical",
            count       = 1,
        },
        -- `race` already resolves to the `beast` layout — `body` is the first
        -- rung and `race` the second, so nothing above needs to name one. What
        -- follows overrides that where the shape is genuinely different, which
        -- is the only reason `body` is written down at all.

        -- ─── The collapsed mine ──────────────────────────────────────────────
        ["mine.beast"] = {
            prototype = "beast",
            faction   = "mine",
            tags      = { "beast", "mine" },
        },
        ["mine.crawler"] = { prototype = "mine.beast", name = "crawler",
                             body = "insectile" },
        ["mine.lurker"]  = { prototype = "mine.beast", name = "lurker" },

        -- ─── Greywater marsh ────────────────────────────────────────────────
        ["marsh.beast"] = {
            prototype = "beast",
            faction   = "marsh",
            tags      = { "beast", "marsh" },
        },
        ["marsh.crawler"] = { prototype = "marsh.beast", name = "crawler",
                              body = "insectile" },

        -- A function in a prototype, which is the half `custom.lua` cannot do:
        -- `custom.lua` gives *this area's* hooks, and this gives everyone's.
        ["marsh.venomous"] = {
            prototype = "marsh.beast",
            name      = "lurker",
            on_combat = bite_carrying("marsh_poison", 25),
        },

        -- ─── Rats ────────────────────────────────────────────────────────────
        --
        -- What the workshop's nest makes. This is the branch to read if you
        -- want to see what prototypes buy, because the three children below are
        -- four lines each and are genuinely different creatures.
        --
        -- `vermin.rat` is the old `workshop_rat` template with its identity
        -- taken out: everything here is true of any rat, and nothing here says
        -- which rat. It carries no `spawn_room`, `count` or `respawn_time` —
        -- the nest is the only thing that decides where rats come from and how
        -- many there are, and a template that also carried those would be fed
        -- by two sources at once. `verify` reports that if anyone adds them.
        ["vermin.rat"] = {
            prototype   = "beast",
            name        = "rat",
            race        = "beast",
            body        = "beast",
            faction     = "vermin",
            aggressive  = false,
            xp_award    = 12,
            damage      = { min = 2, max = 5 },
            -- **Fast and weak**, which is the whole point of a rat and could
            -- not be said before: rate came from `round_length` alone, which
            -- moves 0.05s per point of dexterity, so a rat and a bear were
            -- within a tenth of a second of each other. 2.5 bites to a round
            -- against a swordsman's one.
            speed       = 2.5,
            stats       = { hp = 24, max_hp_flat = 24, strength = 6, dexterity = 12,
                            constitution = 8, intelligence = 2, wisdom = 4, level = 1 },
            loot_table  = { { item_id = "empty_vial", chance = 0.35 } },
            tags        = { "vermin" },
        },

        -- The common one. Nothing but prose differs, which is the point: most
        -- of a bestiary is prose, and a prototype is what stops the twelve keys
        -- underneath it being retyped for every line of it.
        ["vermin.rat.black"] = {
            prototype   = "vermin.rat",
            short       = "a black rat",
            description = "Sleek, unhurried and entirely unbothered by being "
                       .. "looked at. It has the manner of something that lives "
                       .. "here and knows you do not.",
        },

        -- Weaker, and worth less. Two numbers.
        ["vermin.rat.scrawny"] = {
            prototype   = "vermin.rat",
            short       = "a scrawny grey rat",
            description = "Matted fur over a frame you can count. It has been "
                       .. "living on spilled reagents and it has not agreed with "
                       .. "it.",
            stats       = { hp = 14, max_hp_flat = 14, strength = 4, dexterity = 14 },
            xp_award    = 6,
        },

        -- The one worth being careful about. **`stats` is a map, so a patch of
        -- it merges key by key** — `dexterity`, `intelligence` and `wisdom` are
        -- inherited untouched rather than being wiped by the four named here.
        -- An array field would have replaced the lot, and `tags` below does
        -- exactly that, which is why it restates `vermin`.
        ["vermin.rat.muscular"] = {
            prototype   = "vermin.rat",
            short       = "a muscular red rat",
            description = "Half again the size of the others and the colour of "
                       .. "old brick. Something in the pantry has agreed with "
                       .. "this one enormously.",
            stats       = { hp = 40, max_hp_flat = 40, strength = 11, constitution = 12 },
            damage      = { min = 4, max = 8 },
            -- Bigger and heavier, so slower than its cousins — still quicker
            -- than a person.
            speed       = 1.6,
            xp_award    = 30,
            aggressive  = true,
            tags        = { "vermin", "dangerous" },
        },
    },
}
