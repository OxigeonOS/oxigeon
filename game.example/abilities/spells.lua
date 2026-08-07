-- game/abilities/spells.lua — Four spells, and how much Lua each one needs.
--
-- These were four hand-written `cast` functions. Two of them are now data with
-- no code at all, one keeps a single arithmetic helper, and one is still a
-- program. That spread is the point: the mudlib's job is to make the common
-- case free without making the uncommon one impossible.
--
--   emberlance   ZERO LUA   damage through the pipeline, so armour and resists
--                           meet it exactly as they meet a sword
--   mend         ZERO LUA   a gauge adjust through `heal`, so a healing-
--                           amplification effect composes with it for free
--   wardskin     one helper an effect with a `condition`, refused before it lands
--   farsight     `run`      a room-peek loop is a program, not a data bag
--
-- ─── The cost model, unchanged ───────────────────────────────────────────────
--
-- Mana is a gauge, so it is *spent* and never modified: a spell that "modified"
-- your mana would be a buff you have to unapply, which is the mistake the whole
-- effect design avoids. Power scales with `spell_power`, a derived-of-derived
-- trait, so a wisdom buff reaches a fireball through two levels of the graph
-- without anything here knowing.

local Object = require('lib.object')

--- `floor(power / 3)` is not a per-point slope, and inventing a `div` knob to
--- make it one is how a spec grows a language. This is what a function is for.
local function wardskin_potency(ctx)
    return 2 + math.floor((ctx.power or 1) / 3)
end

--- One line of what is through an exit.
local function peek(dir, target_id)
    local next_room = DAEMON.world.get_room(target_id)
    if not next_room then return nil end

    local creatures = DAEMON.mobs and #DAEMON.mobs.in_room(target_id) or 0
    local items = DAEMON.items and #DAEMON.items.in_room(target_id) or 0
    return string.format("  %-9s %s%s%s", dir,
        Object.resolve(next_room.short, next_room) or target_id,
        creatures > 0 and ("  {red}(" .. creatures .. " creature(s)){/}") or "",
        items > 0 and ("  {yellow}(" .. items .. " item(s)){/}") or "")
end

--- The escape hatch, and a fair use of it.
local function farsight_run(ctx)
    local player = ctx.user
    local room = DAEMON.world.get_character_room_obj(player.char_id)
    if not room then
        player:send("You are nowhere to see from.")
        return
    end

    local lines = { "{cyan}You send your sight out.{/}" }
    local dirs = {}
    for dir in pairs(room.exits or {}) do dirs[#dirs + 1] = dir end
    table.sort(dirs)

    for _, dir in ipairs(dirs) do
        local exit = room.exits[dir]
        local line = peek(dir, type(exit) == "table" and exit.target or exit)
        if line then lines[#lines + 1] = line end
    end

    if #lines == 1 then lines[#lines + 1] = "  Nothing leads anywhere." end
    player:send_lines(lines)
end

return {
    -- 6 + power*2, where power is 1 + spell_power: the same numbers the hand-
    -- written version produced, said as `min`/`max` plus a slope.
    {
        id       = "emberlance",
        name     = "Emberlance",
        category = "spell",
        summary  = "A line of fire, at one thing.",
        open     = true,
        level    = 1,
        cost     = { mp = 8 },
        cooldown = 4,
        target   = "creature",

        -- **It costs you the round.** Without this the ability declared no
        -- roundtime, so it never marked the combat track and `auto_round` went
        -- on swinging your fist on its own clock — you cast *and* punched, in
        -- the same second, which is not what casting a spell should be.
        --
        -- `{ rounds = 1 }` rather than a number of seconds, so a quick caster
        -- recovers quicker: it is multiplied by `round_length`, which is the
        -- entity's own clock.
        roundtime = { rounds = 1 },

        -- **Through `resolve_attack`, which is what a sword goes through.**
        -- This file's own header claimed the damage met armour and resists
        -- "exactly as they meet a sword", and `damage` had stopped being that:
        -- a sword is a contest that can miss, is answered by a defence channel,
        -- lands somewhere on the target and reports a degree of success, and
        -- `damage` is a number applied. `attack` is the one that means what the
        -- header says.
        --
        -- `damage_type = "fire"`, so a resist table can meet it. An ability that
        -- dealt untyped damage would be the one thing in the game no armour
        -- could ever be designed against.
        --
        -- Magic is answered by *evasion only*: you cannot parry a line of fire
        -- and a buckler is not in its way. Naming the channel is how that is
        -- said — a spell that omitted `defenses` would be blocked by a shield.
        attack   = {
            defenses = { dodge = 1.0 },
            damage   = { min = 8, max = 8, type = "fire",
                         scale = { trait = "spell_power", per = 2 } },
        },
        engage   = true,

        -- One authored sentence, rendered per reader: the caster reads "You
        -- draw", the target reads "you" as the thing being drawn at, and the
        -- room reads names throughout. Before the render layer this was three
        -- strings kept in step by hand, and the room's copy did not name a
        -- target at all.
        messages = {
            line   = "{red}$Actor $actor.v(draw) a line of fire at $target.{/}",
            result = "It takes $dealt.",
            -- `attack` can miss, so there has to be something to read when it
            -- does. `damage` never could, which is why this line is new.
            miss   = "The fire goes wide of $target.",
        },
    },

    -- 10 + power*3. Through `heal`, so the `heal_received` pipeline runs.
    {
        id       = "mend",
        name     = "Mend",
        category = "spell",
        summary  = "Close what is open.",
        open     = true,
        level    = 1,
        cost     = { mp = 12 },
        cooldown = 6,
        target   = "self",

        heal     = { min = 13, max = 13, to = "self",
                     scale = { trait = "spell_power", per = 3 } },

        messages = {
            self = "{green}The wound closes. ($healed){/}",
            room = "$name's wounds close over.",
        },
    },

    {
        id       = "wardskin",
        name     = "Wardskin",
        category = "spell",
        summary  = "Harden yourself, if you have the will for it.",
        open     = true,
        level    = 3,
        cost     = { mp = 15 },
        cooldown = 30,
        target   = "self",

        -- The effect carries a `condition`, so it is refused *before* it lands
        -- rather than landing and expiring immediately. Those are different, and
        -- only one of them can tell you why — which `$why` prints.
        apply = { { effect = "wardskin", to = "self",
                    duration = { min = 125, max = 125,
                                 scale = { trait = "spell_power", per = 5 } },
                    potency = wardskin_potency } },

        messages = {
            room = "$name's skin takes on a dull sheen.",
            fail = "{yellow}The working will not take: $why{/}",
        },
    },

    -- Six seconds is under `cooldown_durable_seconds`, so the gate lives in
    -- memory and is correctly forgotten on a restart. Nobody would notice, and
    -- nobody should pay a database write for it.
    {
        id       = "farsight",
        name     = "Farsight",
        category = "spell",
        summary  = "See what is in the next room.",
        open     = true,
        level    = 2,
        cost     = { mp = 5 },
        cooldown = 6,
        target   = "none",
        run      = farsight_run,
    },
}
