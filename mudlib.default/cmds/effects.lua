local M = {}
M.name = 'effects'
M.aliases = { 'buffs', 'affects' }
M.category = 'information'
M.summary = 'List what is currently affecting you.'
M.permission = nil

--- "4m12s" reads better than "252 seconds" at a glance.
local function duration(seconds)
    if seconds < 0 then return "permanent" end
    if seconds < 60 then return string.format("%ds", math.floor(seconds)) end
    if seconds < 3600 then
        return string.format("%dm%02ds", math.floor(seconds / 60), math.floor(seconds % 60))
    end
    return string.format("%dh%02dm", math.floor(seconds / 3600), math.floor((seconds % 3600) / 60))
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.effect) then
        player:send("{red}The effect system is not available.{/}")
        return
    end

    local ok, active = pcall(DAEMON.effect.active, player)
    if not ok then
        player:send("{red}Something went wrong reading your effects.{/}")
        return
    end
    if #active == 0 then
        player:send("You are not affected by anything.")
        return
    end

    local now = os_time()
    local lines = { "{cyan}Currently affecting you:{/}" }
    for _, e in ipairs(active) do
        local left = e.inst.expires and (e.inst.expires - now) or -1
        local label = e.def.label or e.inst.def
        if (e.inst.stacks or 1) > 1 then
            label = label .. " x" .. e.inst.stacks
        end
        local line = string.format("  {green}%-20s{/} %-10s", label, duration(left))
        if e.def.desc then line = line .. "  " .. e.def.desc end
        lines[#lines + 1] = line
    end

    if DAEMON.cooldown then
        local cok, cooldowns = pcall(DAEMON.cooldown.list, player.char_id)
        if cok and #cooldowns > 0 then
            lines[#lines + 1] = ""
            lines[#lines + 1] = "{cyan}Waiting on:{/}"
            for _, cd in ipairs(cooldowns) do
                lines[#lines + 1] = string.format("  {yellow}%-20s{/} %s",
                    cd.what, duration(cd.remaining))
            end
        end
    end

    player:send_lines(lines)
end

return M
