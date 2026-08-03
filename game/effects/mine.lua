-- game/effects/mine.lua — What the mine leaves on you.

return {
    --- The boss's curse. `survives_death`, so dying does not shake it off, and
    --- long enough that the only real answer is to go back up.
    {
        id = "delvers_regard", label = "The Delver's Regard",
        desc = "It has taken an interest, and it does not stop taking one.",
        duration = 1200, stack = "refresh",
        survives_death = true,
        persist = true,
        modifiers = { dexterity = -2, wisdom = -1 },
        hooks = {
            -- Everything in the dark finds you more easily. A `mult` handler,
            -- so it scales the incoming number before any flat reduction comes
            -- off it — which is the ordering the phases exist to guarantee.
            damage_taken = { phase = "mult", fn = function(ev)
                ev.scale = ev.scale + 0.15
            end },
        },
        on_apply = function(ctx)
            if ctx.entity.send then
                ctx.entity:send("{red}It stops working and looks at you, and then goes back to work.{/}")
            end
        end,
        on_expire = function(ctx)
            if ctx.entity.send then
                ctx.entity:send("{green}The feeling of being worked out goes away.{/}")
            end
        end,
    },
}
