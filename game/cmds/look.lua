local M = {}

M.name = 'look'
M.aliases = {'l'}
M.category = 'navigation'
M.summary = 'Look at your surroundings.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local session = get_session(session_id)
    if not session or not session.character_id then
        send(session_id, "You are nowhere. This is concerning.\r\n")
    
        return
    end
    
    local char_id = session.character_id
    local room = DAEMON.world.get_character_room_obj(char_id)
    
    if not room then
        send(session_id, "You are nowhere. This is concerning.\r\n")
    
        return
    end
    
    send(session_id, room:get_appearance(session_id))

end

return M
