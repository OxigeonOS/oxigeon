local M = {}

function M.find_session_for_character(char_id)
    local sessions = all_sessions()
    for _, sid in ipairs(sessions) do
        local s = get_session(sid)
        if s and s.character_id == char_id then
            return sid
        end
    end
    return nil
end

function M.send_to_room(room_id, text, exclude_char_id)
    local room = DAEMON.world.get_room(room_id)
    if not room then return end
    
    for _, char_id in ipairs(room:get_characters()) do
        if char_id ~= exclude_char_id then
            local sid = M.find_session_for_character(char_id)
            if sid then
                send(sid, text .. "\r\n")

            end
        end
    end
end

return M
