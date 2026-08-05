-- game/effects/marsh.lua — What the marsh does to you.
--
-- Three effects, each one a thing the effect system claims to support and
-- nothing had used:
--
--   marsh_poison   `tick`, driven by the shared heartbeat rather than by a
--                  timer per effect — damage over time with no per-effect timer
--   marsh_chill    a `condition`, checked through `lib/checks.lua` predicates,
--                  which had no user at all
--   wisp_mark      `survives_death`, so it is still on you after you respawn
--
-- Poison is deliberately *not* a `trait:hp` modifier. Effects modify attributes
-- and derived traits and never gauges — a poison that edited your current
-- health would have to be unapplied symmetrically, which is the exact mistake
-- the whole design avoids. It deals damage, which is an event.

local Effects = require('lib.effects')

-- ─── Handlers ────────────────────────────────────────────────────────────────

--- marsh_poison: damage per tick, through `take_damage` so armour and
--- resistances get their say. A poison that bypassed the pipeline would be the
--- one damage source a warded cloak cannot touch, for no reason anyone wrote
--- down.
local function poison_tick(ev, ctx)
    local per_tick = ctx.potency or 3
    ctx.entity:take_damage(per_tick * (ev.ticks or 1),
        { damage_type = "poison", source = "marsh_poison" })
end

--- wisp_mark: everything magical finds you more easily. A `mult` handler, so it
--- scales the number before any reduction is taken off it.
local function wisp_mark_magic(ev)
    if ev.damage_type == "magic" then
        ev.scale = ev.scale + 0.25
    end
end

local checks = require('lib.checks')

return {
    --- Damage over time, on the shared heartbeat.
    ---
    --- `tick = 5` earns whole ticks from `last_tick` and advances by exactly
    --- the ticks it fired, so a coarse heartbeat loses no accuracy and one that
    --- skipped a beat catches up. Two global timers drive every effect in the
    --- game; this one owns none of its own.
    {
        id = "marsh_poison", label = "Marsh Fever",
        desc = "Something from the water is in you.",
        duration = 180, tick = 5, stack = "refresh",
        potency = 3,
        hooks = {
            heartbeat = { phase = "post", fn = poison_tick },
        },
        on_apply = Effects.says("{green}Your skin goes cold and your mouth fills with the taste of the water.{/}"),
        on_expire = Effects.says("{green}The fever breaks. You can taste your own mouth again.{/}"),
    },

    --- A `condition`, which is checked when the effect is applied and refuses
    --- it outright rather than landing and expiring immediately.
    ---
    --- The first user of `lib/checks.lua`, whose predicate library had no
    --- caller anywhere. The point of a predicate library is that a condition
    --- reads as a sentence rather than as a paragraph of `and`s.
    {
        id = "marsh_chill", label = "Chilled",
        desc = "The cold has got into you.",
        duration = 300, stack = "refresh",
        -- Only lands on someone who is not carrying a light. A lantern is warm,
        -- which is a thin justification for a rule and a good demonstration of
        -- a condition that reads the world.
        condition = checks.all(
            checks.not_(checks.has_item("hooded_lantern")),
            checks.has_level(1)
        ),
        modifiers = { dexterity = -2 },
        on_apply = Effects.says("{cyan}The cold works into your hands. They are slow to answer.{/}"),
        on_expire = Effects.says("{cyan}Feeling comes back into your hands.{/}"),
    },

    --- `survives_death`. Dying clears your effects — except the ones that say
    --- otherwise, because a curse you can remove by walking into a rat is not a
    --- curse.
    {
        id = "wisp_mark", label = "Marked",
        desc = "The wisp has seen you, and has not stopped.",
        duration = 1800, stack = "ignore",
        survives_death = true,
        persist = true,
        modifiers = { wisdom = -1 },
        hooks = {
            damage_taken = { phase = "mult", fn = wisp_mark_magic },
        },
        on_apply = Effects.says("{cyan}The light turns towards you, and stays turned.{/}"),
        on_expire = Effects.says("{cyan}Whatever was watching you stops.{/}"),
    },
}
