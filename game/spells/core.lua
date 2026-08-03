-- game/spells/core.lua — Four spells, one per mechanism.

return {
    --- DAMAGE, through the pipeline. A spell is not a special case: armour,
    --- resists and phase ordering apply to it exactly as they do to a sword,
    --- because it goes through `take_damage` like everything else.
    {
        id      = "emberlance",
        name    = "Emberlance",
        summary = "A line of fire, at one thing.",
        cost    = 8,
        cooldown = 4,
        target  = "creature",
        level   = 1,
        cast = function(player, target, power)
            local damage = 6 + power * 2
            player:send("{red}You draw a line of fire at "
                .. (target.short or "it") .. ".{/}")
            player:message_room(player.name .. " draws a line of fire.")

            -- `damage_type = "fire"`, so a resist table can meet it. A spell
            -- that dealt untyped damage would be the one thing in the game no
            -- armour could ever be designed against.
            local _, dealt = target:take_damage(damage, {
                damage_type = "fire",
                attacker = player,
            })
            player:send("It takes " .. dealt .. ".")

            -- Being set on fire is a reason to fight back.
            if DAEMON.combat and target.is_alive and target:is_alive() then
                DAEMON.combat.engage(target, player)
            end
        end,
    },

    --- HEAL, as a gauge `adjust`. Not a modifier: a buff that "modified" your
    --- current health would have to be unapplied symmetrically, which is the
    --- mistake the effect design exists to avoid. `adjust` settles regeneration
    --- first, so it lands on the value as it is now.
    {
        id      = "mend",
        name    = "Mend",
        summary = "Close what is open.",
        cost    = 12,
        cooldown = 6,
        target  = "self",
        level   = 1,
        cast = function(player, target, power)
            local amount = 10 + power * 3
            -- Through `heal`, so the `heal_received` pipeline runs and a
            -- healing-amplification effect composes with this for free.
            local _, healed = player:heal(amount, { source = "spell:mend" })
            player:send("{green}The wound closes. (" .. healed .. "){/}")
            player:message_room(player.name .. "'s wounds close over.")
        end,
    },

    --- BUFF, with a `condition` — refused *before* it lands rather than
    --- landing and expiring immediately. Those are different, and only one of
    --- them can tell you why.
    {
        id      = "wardskin",
        name    = "Wardskin",
        summary = "Harden yourself, if you have the will for it.",
        cost    = 15,
        cooldown = 30,
        target  = "self",
        level   = 3,
        cast = function(player, target, power)
            local inst, why = DAEMON.effect.apply(player, "wardskin", {
                source = "spell:wardskin",
                duration = 120 + power * 5,
                potency = 2 + math.floor(power / 3),
            })
            if not inst then
                player:send("{yellow}The working will not take: "
                    .. tostring(why or "something is in the way") .. "{/}")
                return
            end
            player:message_room(player.name .. "'s skin takes on a dull sheen.")
        end,
    },

    --- UTILITY, on a **memory-tier** cooldown. Six seconds is under the durable
    --- threshold, so it lives in memory and is correctly forgotten on a
    --- restart — nobody would notice, and nobody should pay a write for it.
    {
        id      = "farsight",
        name    = "Farsight",
        summary = "See what is in the next room.",
        cost    = 5,
        cooldown = 6,
        target  = "none",
        level   = 2,
        cast = function(player, target, power)
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
                local target_id = type(exit) == "table" and exit.target or exit
                local next_room = DAEMON.world.get_room(target_id)
                if next_room then
                    local creatures = DAEMON.mobs and #DAEMON.mobs.in_room(target_id) or 0
                    local items = DAEMON.items and #DAEMON.items.in_room(target_id) or 0
                    lines[#lines + 1] = string.format("  %-9s %s%s%s", dir,
                        require('lib.object').resolve(next_room.short, next_room) or target_id,
                        creatures > 0 and ("  {red}(" .. creatures .. " creature(s)){/}") or "",
                        items > 0 and ("  {yellow}(" .. items .. " item(s)){/}") or "")
                end
            end

            if #lines == 1 then lines[#lines + 1] = "  Nothing leads anywhere." end
            player:send_lines(lines)
        end,
    },
}
