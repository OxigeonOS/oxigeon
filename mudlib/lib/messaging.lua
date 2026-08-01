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

--- Send a message to all characters in a room, excluding specified characters.
-- @param room_id string           The room to broadcast to
-- @param text string              The message text
-- @param exclude_char_id number|table|nil  A single char_id or an array of char_ids to exclude
function M.send_to_room(room_id, text, exclude_char_id)
    local room = DAEMON.world.get_room(room_id)
    if not room then return end

    -- Normalize exclude list: single value → set, table → set, nil → empty set
    local excluded = {}
    if type(exclude_char_id) == "table" then
        for _, id in ipairs(exclude_char_id) do
            excluded[id] = true
        end
    elseif exclude_char_id ~= nil then
        excluded[exclude_char_id] = true
    end

    for _, char_id in ipairs(room:get_characters()) do
        if not excluded[char_id] then
            local sid = M.find_session_for_character(char_id)
            if sid then
                send(sid, text .. "\r\n")
            end
        end
    end
end

return M
