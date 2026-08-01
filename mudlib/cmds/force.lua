local M = {}
M.name = 'force'
M.aliases = {}
M.category = 'admin'
M.summary = 'Execute command as another player.'
M.permission = 'admin'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args or #args < 2 then
        player:send("{cyan}Usage: force <player> <command string>{/}")
        return
    end

    local target_name = args[1]
    local cmd_str = args_str:match("^%S+%s+(.*)$") or table.concat(args, " ", 2)

    local target_player, target_sid
    for _, sid in ipairs(all_sessions()) do
        local p = get_player(sid)
        if p and p.name and p.name:lower() == target_name:lower() then
            target_player = p
            target_sid = sid
            break
        end
    end

    if not target_player then
        player:send("{red}No player named '{yellow}" .. target_name .. "{red}' is online.{/}")
        return
    end

    player:send("{green}You force {yellow}" .. target_player.name .. "{green} to: {/}" .. cmd_str)
    send(target_sid, "{yellow}" .. player.name .. "{red} forces you to: {/}" .. cmd_str .. "\r\n")

    local ok, err = pcall(function()
        require('lib.commands').dispatch(target_sid, cmd_str)
    end)
    
    if DAEMON and DAEMON.audit then
        pcall(DAEMON.audit.log, "cmd.force", true, "forced " .. target_player.name .. " to: " .. cmd_str)
    end
end

return M
