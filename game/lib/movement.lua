local messaging = require('lib.messaging')
local M = {}

M.OPPOSITES = {
    north = "south",
    south = "north",
    east = "west",
    west = "east",
    northeast = "southwest",
    southwest = "northeast",
    northwest = "southeast",
    southeast = "northwest",
    up = "down",
    down = "up",
    ["in"] = "out",
    out = "in"
}

function M.move(session_id, direction)
    local session = get_session(session_id)
    if not session or not session.character_id then return end
    
    local char_id = session.character_id
    local current_room_id = DAEMON.world.get_character_room(char_id)
    local current_room = DAEMON.world.get_room(current_room_id)
    
    if not current_room then
        send(session_id, "You are nowhere.\r\n")
    
        return
    end
    
    if not current_room:has_exit(direction) then
        send(session_id, "There is no exit in that direction.\r\n")
    
        return
    end
    
    local target_room_id = current_room:get_exit(direction)
    local target_room = DAEMON.world.get_room(target_room_id)
    
    if not target_room then
        send(session_id, "That exit leads nowhere.\r\n")
    
        return
    end
    
    local char_data = get_character(char_id)
    local char_name = char_data and char_data.name or "Someone"

    -- Notify the old room before moving
    messaging.send_to_room(current_room_id, char_name .. " leaves " .. direction .. ".", char_id)

    -- Attempt the move
    local moved = DAEMON.world.move_character(char_id, target_room_id)
    if not moved then
        log("error", "movement: move_character failed for char "
            .. tostring(char_id) .. " to room '" .. target_room_id .. "'")
        send(session_id, "Something went wrong with that exit.\r\n")
    
        return
    end

    local opp_dir = M.OPPOSITES[direction] or "somewhere"
    messaging.send_to_room(target_room_id, char_name .. " arrives from the " .. opp_dir .. ".", char_id)

    local appearance = target_room:get_appearance(session_id)
    send(session_id, appearance)

end

return M
