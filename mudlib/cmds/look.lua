local M = {}

M.name = 'look'
M.aliases = {'l'}
M.category = 'navigation'
M.summary = 'Look at your surroundings or examine an item.'
M.permission = nil

local Object = require('lib.object')

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

    -- If no arguments, show the full room
    if not args[1] or args_str == "" then
        send(session_id, room:get_appearance(session_id))
        return
    end

    -- look <keyword> — check room items (scenery)
    local keyword = args[1]:lower()
    local item_desc = room:get_item(keyword)
    if item_desc then
        -- Resolve lfun: item descriptions can be strings or functions
        local resolved = Object.resolve(item_desc, room)
        if resolved then
            send(session_id, resolved .. "\r\n")
        else
            send(session_id, "You see nothing special.\r\n")
        end
        return
    end

    -- look <player_name> — check players in the room
    for _, cid in ipairs(room:get_characters()) do
        local char_data = get_character(cid)
        if char_data and char_data.name:lower() == keyword then
            -- Try to get the Player object for a richer examine
            local player_obj = DAEMON.character.get(cid)
            if player_obj and player_obj.examine then
                send(session_id, player_obj:examine() .. "\r\n")
            else
                send(session_id, char_data.name .. " is here.\r\n")
            end
            return
        end
    end

    send(session_id, "You don't see that here.\r\n")
end

return M
