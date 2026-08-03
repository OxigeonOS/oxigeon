-- game/effects/core.lua — The effects this game has.
--
-- An effect definition holds functions, so it lives in code and is never
-- written anywhere. What gets saved is the *instance*: nine plain fields
-- saying which definition, when it started, when it ends.
--
-- Handlers declare a phase rather than an order, because the answer depends on
-- it: 30 damage with "-15%" and "-5 flat" is 20 if the percentage goes first
-- and 21 if it does not, and no player should have to know which buff landed
-- first to predict their own health. See mudlib/lib/effects.lua.

return {
    -- ─── The four worked examples ────────────────────────────────────────────

    --- "a Buff that makes me make 20% more experience"
    {
        id = "insight", label = "Insight",
        desc = "Everything you do teaches you a little more.",
        duration = 3600, stack = "stack", max_stacks = 3,
        hooks = {
            xp_gained = { phase = "mult", fn = function(ev, ctx)
                ev.scale = ev.scale + 0.20 * (ctx.stacks or 1)
            end },
        },
        on_apply = function(ctx)
            if ctx.entity.send then ctx.entity:send("{cyan}Your thoughts sharpen.{/}") end
        end,
        on_expire = function(ctx)
            if ctx.entity.send then ctx.entity:send("{cyan}Your thoughts dull again.{/}") end
        end,
    },

    --- "take 15% less damage" AND "negate 5 from each bit of damage" —
    --- one effect, two handlers, two phases. The percentage is applied to the
    --- incoming number, then the flat reduction comes off what is left.
    {
        id = "stoneskin", label = "Stoneskin",
        desc = "Your skin is hard as granite.",
        duration = 60, potency = 5, stack = "refresh",
        -- Sugar: desugars into an ordinary `trait:constitution` handler in the
        -- `add` phase. One mechanism, two ways to write it.
        modifiers = { constitution = 2 },
        hooks = {
            damage_taken = { phase = "mult", fn = function(ev)
                ev.scale = ev.scale - 0.15
            end },
            ["damage_taken#flat"] = { hook = "damage_taken", phase = "reduce",
                fn = function(ev, ctx)
                    ev.amount = math.max(0, ev.amount - (ctx.potency or 5))
                end },
        },
        on_apply = function(ctx)
            if ctx.entity.send then ctx.entity:send("{yellow}Your skin hardens to stone.{/}") end
        end,
        on_expire = function(ctx)
            if ctx.entity.send then ctx.entity:send("{yellow}The stone sheen fades from your skin.{/}") end
        end,
    },

    --- "heal 2% of missing HP every tick"
    ---
    --- Note it heals through `entity:heal`, which runs the `heal_received`
    --- pipeline — so a healing-amplification buff would compose with this for
    --- free, and the re-entrancy guard is what stops that becoming a loop.
    {
        id = "regeneration", label = "Regeneration",
        desc = "Your wounds are closing.",
        duration = 300, tick = 3, stack = "refresh",
        hooks = {
            heartbeat = { phase = "post", fn = function(ev, ctx)
                local e = ctx.entity
                local missing = e:trait("max_hp") - e:trait("hp")
                if missing <= 0 then return end
                local amount = math.max(1, math.floor(missing * 0.02 * (ev.ticks or 1)))
                e:heal(amount, { source = "effect:regeneration" })
            end },
        },
        on_apply = function(ctx)
            if ctx.entity.send then ctx.entity:send("{green}A warm glow suffuses you.{/}") end
        end,
        on_expire = function(ctx)
            if ctx.entity.send then ctx.entity:send("{green}The warm glow fades.{/}") end
        end,
    },

    -- ─── A few more, to show the shape ───────────────────────────────────────

    --- A debuff, to prove modifiers go both ways.
    {
        id = "weakened", label = "Weakened",
        desc = "Your arms feel like lead.",
        duration = 120, stack = "refresh",
        modifiers = { strength = -3 },
    },

    --- A percentage modifier on a derived trait: nothing special is needed,
    --- because a gauge's maximum is just another trait.
    {
        id = "hearty", label = "Hearty",
        desc = "You feel unusually robust.",
        duration = 600, stack = "refresh",
        modifiers = { max_hp = "+10%" },
    },

    --- Survives death, to show the flag doing something.
    {
        id = "cursed", label = "Cursed",
        desc = "Something is very wrong.",
        duration = 1800, stack = "refresh", survives_death = true,
        modifiers = { willpower = -2 },
    },

    --- The buff `wardskin` applies. A `condition` over `lib/checks.lua`, so it
    --- is refused *before* it lands rather than landing and expiring — those
    --- are different, and only one of them can tell you why.
    {
        id = "wardskin", label = "Wardskin",
        desc = "Your skin has gone hard and slightly grey.",
        duration = 120, potency = 2, stack = "refresh",
        -- Only on someone with the will for it. `spell_power` is derived from
        -- a derived trait, so this reaches two levels into the graph.
        condition = function(entity)
            if entity.trait and entity:trait("willpower") < 0 then
                return false, "your will is not in it"
            end
            return true
        end,
        modifiers = { constitution = 1 },
        hooks = {
            damage_taken = { phase = "reduce", fn = function(ev, ctx)
                ev.amount = math.max(0, ev.amount - (ctx.potency or 2))
            end },
        },
        on_apply = function(ctx)
            if ctx.entity.send then ctx.entity:send("{cyan}Your skin hardens and greys.{/}") end
        end,
        on_expire = function(ctx)
            if ctx.entity.send then ctx.entity:send("{cyan}Your skin goes soft again.{/}") end
        end,
    },
}
