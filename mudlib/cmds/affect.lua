local M = {}
M.name = 'affect'
M.aliases = {}
M.category = 'admin'
M.summary = 'Inspect and drive the trait and effect systems.'
M.permission = 'admin'

--- The admin window onto traits and effects.
---
--- It is also, deliberately, the only way a test running against the real
--- mudlib can inject damage or experience: `RealVm::boot_real_mudlib` can send
--- commands and nothing else, and there is no other verb in the game that
--- deals damage on demand. Without this, half the pipeline would only ever be
--- exercised by unit tests running beside the real code rather than through it.
local USAGE = {
    "{cyan}affect{/} — trait and effect diagnostics",
    "  affect list                 what is affecting you",
    "  affect apply <id> [secs]    apply an effect to yourself",
    "  affect remove <id>          remove one",
    "  affect clear                remove everything",
    "  affect damage <n> [type]    take damage, through the pipeline",
    "  affect heal <n>             heal, through the pipeline",
    "  affect xp <n>               award experience, through the pipeline",
    "  affect settle               settle regenerating gauges now",
    "  affect traits               every trait, base and effective",
    "  affect defs                 registered effect definitions",
    "  affect cache                state cache statistics",
    "  affect cooldown <what> <s>  set a cooldown on yourself",
}

local function require_daemons(player)
    if not (DAEMON and DAEMON.trait and DAEMON.effect) then
        player:send("{red}The trait or effect daemon is not loaded.{/}")
        return false
    end
    return true
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    if not require_daemons(player) then return end

    local verb = (args[1] or ""):lower()

    if verb == "" or verb == "help" then
        player:send_lines(USAGE)
        return
    end

    if verb == "list" then
        local active = DAEMON.effect.active(player)
        if #active == 0 then
            player:send("Nothing is affecting you.")
            return
        end
        local now = os_time()
        local lines = {}
        for _, e in ipairs(active) do
            lines[#lines + 1] = string.format("  %-16s %-20s %s",
                e.inst.def, e.key,
                e.inst.expires and (math.floor(e.inst.expires - now) .. "s left") or "permanent")
        end
        player:send_lines(lines)
        return
    end

    if verb == "apply" then
        local id = args[2]
        if not id then player:send("Apply what?") return end
        local seconds = tonumber(args[3])
        local inst, why = DAEMON.effect.apply(player, id, {
            duration = seconds,
            source = "admin:" .. tostring(player.char_id),
        })
        if not inst then
            player:send("{red}Could not apply '" .. id .. "'"
                .. (why and (": " .. tostring(why)) or "") .. ".{/}")
        else
            player:send("{green}Applied " .. id .. ".{/}")
        end
        pcall(DAEMON.audit.log, "cmd.affect", true, "applied " .. id)
        return
    end

    if verb == "remove" then
        local id = args[2]
        if not id then player:send("Remove what?") return end
        local n = DAEMON.effect.remove(player, id, { reason = "admin" })
        player:send("Removed " .. n .. " effect(s).")
        return
    end

    if verb == "clear" then
        local n = DAEMON.effect.clear(player, { reason = "admin" })
        player:send("Cleared " .. n .. " effect(s).")
        return
    end

    if verb == "damage" then
        local amount = tonumber(args[2])
        if not amount then player:send("How much damage?") return end
        local before = player:stat("hp")
        local remaining, dealt = player:take_damage(amount, {
            damage_type = args[3] or "physical",
            attacker = player,
        })
        player:send(string.format(
            "{red}%d requested, %d dealt.{/} HP %d -> %d", amount, dealt, before, remaining))
        return
    end

    if verb == "heal" then
        local amount = tonumber(args[2])
        if not amount then player:send("Heal how much?") return end
        local before = player:stat("hp")
        local now_hp, healed = player:heal(amount, { source = "admin" })
        player:send(string.format(
            "{green}%d requested, %d applied.{/} HP %d -> %d", amount, healed, before, now_hp))
        return
    end

    if verb == "xp" then
        local amount = tonumber(args[2])
        if not amount then player:send("How much experience?") return end
        local gained = player:award_xp(amount, { source = "admin" })
        player:send(string.format("{green}%d requested, %d awarded.{/} Total %d",
            amount, gained or 0, player.xp or 0))
        return
    end

    if verb == "settle" then
        local changed = DAEMON.trait.touch(player)
        player:send(changed and "Gauges settled." or "Nothing to settle.")
        return
    end

    if verb == "traits" then
        local lines = { string.format("  %-16s %-10s %8s %8s", "trait", "kind", "base", "value") }
        for _, t in ipairs(DAEMON.trait.all(player)) do
            lines[#lines + 1] = string.format("  %-16s %-10s %8s %8s%s",
                t.id, t.kind, tostring(t.base), tostring(t.value),
                t.failed and ("  {red}" .. t.failed .. "{/}") or "")
        end
        local errors = DAEMON.trait.errors()
        if next(errors) then
            lines[#lines + 1] = ""
            lines[#lines + 1] = "{red}Broken definitions:{/}"
            for id, why in pairs(errors) do
                lines[#lines + 1] = "  " .. id .. ": " .. tostring(why)
            end
        end
        player:send_lines(lines)
        return
    end

    if verb == "defs" then
        local lines = {}
        local ids = {}
        for id in pairs(DAEMON.effect.defs()) do ids[#ids + 1] = id end
        table.sort(ids)
        for _, id in ipairs(ids) do
            local def = DAEMON.effect.defs()[id]
            lines[#lines + 1] = string.format("  %-16s %-10s %s",
                id, def.stack, def.duration and (def.duration .. "s") or "permanent")
        end
        player:send_lines(#lines > 0 and lines or { "No effects are defined." })
        return
    end

    if verb == "cache" then
        if not DAEMON.cache then player:send("{red}No cache daemon.{/}") return end
        local s = DAEMON.cache.stats()
        player:send_lines({
            string.format("  namespaces %d, scopes %d (%d dirty), about %d bytes",
                s.namespaces, s.loaded_scopes, s.dirty_scopes, s.bytes),
            string.format("  db: %d gets, %d puts, %d deletes",
                s.db_gets, s.db_puts, s.db_deletes),
            string.format("  refused %d write(s), %d failure(s), %d quarantined",
                s.rejected_writes, s.flush_failures, s.poisoned),
        })
        return
    end

    if verb == "cooldown" then
        if not DAEMON.cooldown then player:send("{red}No cooldown daemon.{/}") return end
        local what, seconds = args[2], tonumber(args[3])
        if not what or not seconds then player:send("affect cooldown <what> <seconds>") return end
        DAEMON.cooldown.mark(player.char_id, what, seconds)
        player:send("Set " .. what .. " for " .. seconds .. "s.")
        return
    end

    player:send("{red}Unknown subcommand '" .. verb .. "'.{/}")
    player:send_lines(USAGE)
end

return M
