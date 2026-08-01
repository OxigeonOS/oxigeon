local M = {}
M.name = 'teleport'
M.aliases = {'tp'}
M.category = 'admin'
M.summary = 'Teleport another player.'
M.permission = 'admin'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args or #args < 2 then
        player:send("{cyan}Usage: teleport <player> <room_id>{/}")
        return
    end

    local target_name = args[1]
    local room_id = args[2]

    local target_player
    for _, sid in ipairs(all_sessions()) do
        local p = get_player(sid)
        if p and p.name and p.name:lower() == target_name:lower() then
            target_player = p
            break
        end
    end

    if not target_player then
        player:send("{red}No player named '{yellow}" .. target_name .. "{red}' is online.{/}")
        return
    end

    local ok = pcall(function() target_player:move_to(room_id) end)
    if ok then
        player:send("{green}You teleport {yellow}" .. target_player.name .. "{green} to {yellow}" .. room_id .. "{green}.{/}")
        target_player:send("{cyan}You are teleported by {yellow}" .. player.name .. "{cyan} to {yellow}" .. room_id .. "{cyan}.{/}")
        if DAEMON and DAEMON.audit then
            pcall(DAEMON.audit.log, "cmd.teleport", true, "teleported " .. target_player.name .. " to " .. room_id)
        end
    else
        player:send("{red}Room '{yellow}" .. room_id .. "{red}' does not exist or move failed.{/}")
    end
end

return M
