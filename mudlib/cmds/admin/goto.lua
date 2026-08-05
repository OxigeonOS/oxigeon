local M = {}
M.name = 'goto'
M.aliases = {}
M.category = 'admin'
M.summary = 'Teleport to any room.'
M.permission = 'admin'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args or #args == 0 then
        player:send("{cyan}Go to where? Usage: goto <room_id>{/}")
        return
    end

    local room_id = args[1]
    
    local room = DAEMON.world.get_room(room_id)
    if not room then
        player:send("{red}Room '{yellow}" .. room_id .. "{red}' does not exist.{/}")
        return
    end
    
    player:message_room("{yellow}" .. player.name .. "{/} vanishes in a flash of light.")
    
    local ok = pcall(function() player:move_to(room_id) end)
    if not ok then
        player:send("{red}Room '{yellow}" .. room_id .. "{red}' does not exist.{/}")
    end
end

return M
