-- game/effects/mine.lua — What the mine leaves on you.

local Effects = require('lib.effects')

--- delvers_regard: everything in the dark finds you more easily. A `mult`
--- handler, so it scales the incoming number before any flat reduction comes
--- off it — which is the ordering the phases exist to guarantee.
local function delvers_regard_mult(ev)
    ev.scale = ev.scale + 0.15
end

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
            damage_taken = { phase = "mult", fn = delvers_regard_mult },
        },
        on_apply = Effects.says("{red}It stops working and looks at you, and then goes back to work.{/}"),
        on_expire = Effects.says("{green}The feeling of being worked out goes away.{/}"),
    },
}
