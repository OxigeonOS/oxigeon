-- game/abilities/techniques.lua — What a fighter does, with no Lua at all.
--
-- `cleave` exists to prove the whole spec surface in one record. It uses every
-- one of the four things the ability system was built for and contains not a
-- single line of code:
--
--   ownership     `rank_trait` — knowing it is having the trait, and the trait's
--                 value *is* the rank, so a rank-boosting effect reaches it
--                 through the graph without this file knowing
--   gating        `requires` — a declarative predicate with its own refusal
--   cost/cooldown a gauge spent, and a cooldown shared with other heavy blows
--   timing        a cast time you can be knocked out of
--   outcomes      damage scaled by rank, and a chance of a debuff
--
-- It is not granted by anything shipped: `swordsmanship` is a `sets = false`
-- counter, so a character has it only once somebody teaches them. That is the
-- point of a classless game — the ability exists, and what opens it is content.

return {
    {
        id       = "cleave",
        name     = "Cleave",
        category = "technique",
        summary  = "A sweeping blow that opens a wound.",

        -- Presence decided by storage, exactly as everywhere else: you know this
        -- if there is a number for `swordsmanship` on you.
        rank_trait = "swordsmanship",
        min_rank   = 1,

        requires = {
            { kind = "equipped", slot = "weapon", tag = "weapon",
              why = "You need a weapon in your hand for that." },
        },

        cost     = { stamina = 10 },
        -- Shared, so every heavy technique gates every other with no new
        -- mechanism: `shared` is used as the cooldown key verbatim.
        cooldown = { seconds = 8, shared = "technique.heavy" },

        target         = "creature",
        -- No argument means what you are already fighting.
        default_target = "combat",

        cast_time = 1,
        interrupt = { on_damage = true, on_move = true, threshold = 5 },

        -- Through the resolution pipeline, like a sword: it can miss, a defence
        -- channel answers it, it lands on a body part and it reports a degree.
        -- `damage` is a number applied and could do none of that, which made
        -- this a poor demonstration of a technique.
        attack = {
            damage = { min = 4, max = 9, type = "physical",
                       -- `rank` is a pseudo-trait resolving to this ability's
                       -- own rank, so the slope works whether the rank came
                       -- from the trait or from something that granted it.
                       scale = { trait = "rank", per = 1.5 } },
        },

        -- **A round and a half of recovery**, which is the queue half this
        -- record existed to demonstrate and did not.
        --
        -- `{ rounds = n }` is *multiplicative* — n times whatever a round is for
        -- this fighter — which is why it is not the same thing as `scale`, and
        -- why it cannot be desugared into one. A slow fighter's cleave costs
        -- them proportionally more real time than a quick one's, and neither
        -- number is written down here.
        roundtime = { rounds = 1.5 },

        apply = { { effect = "weakened", to = "target", duration = 12, chance = 0.4 } },

        messages = {
            begin  = "You set your feet.",
            self   = "You sweep your blade through $target.",
            room   = "$name sweeps a blade through $target.",
            result = "It takes $dealt.",
            miss   = "$Target steps inside the arc and it goes past.",
        },
    },
}
