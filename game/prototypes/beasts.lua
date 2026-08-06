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
--       └ marsh.beast
--           ├ marsh.crawler
--           └ marsh.venomous  + a bite that carries something
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

        -- ─── The collapsed mine ──────────────────────────────────────────────
        ["mine.beast"] = {
            prototype = "beast",
            faction   = "mine",
            tags      = { "beast", "mine" },
        },
        ["mine.crawler"] = { prototype = "mine.beast", name = "crawler" },
        ["mine.lurker"]  = { prototype = "mine.beast", name = "lurker" },

        -- ─── Greywater marsh ────────────────────────────────────────────────
        ["marsh.beast"] = {
            prototype = "beast",
            faction   = "marsh",
            tags      = { "beast", "marsh" },
        },
        ["marsh.crawler"] = { prototype = "marsh.beast", name = "crawler" },

        -- A function in a prototype, which is the half `custom.lua` cannot do:
        -- `custom.lua` gives *this area's* hooks, and this gives everyone's.
        ["marsh.venomous"] = {
            prototype = "marsh.beast",
            name      = "lurker",
            on_combat = bite_carrying("marsh_poison", 25),
        },
    },
}
