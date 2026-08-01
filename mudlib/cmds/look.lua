local M = {}

M.name = 'look'
M.aliases = {'l'}
M.category = 'navigation'
M.summary = 'Look at your surroundings or examine an item.'
M.permission = nil

local Object = require('lib.object')

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then
        send(session_id, "You are nowhere. This is concerning.\r\n")
        return
    end

    local session = get_session(session_id)
    if not session or not session.character_id then
        player:send("{red}You are nowhere. This is concerning.{/}")
        return
    end

    local char_id = session.character_id
    local room = DAEMON.world.get_character_room_obj(char_id)

    if not room then
        player:send("{red}You are nowhere. This is concerning.{/}")
        return
    end

    -- If no arguments, show the full room
    if not args[1] or args_str == "" then
        -- room:get_appearance returns pre-formatted text with color tags
        player:send_raw(room:get_appearance(session_id))
        return
    end

    -- look <keyword> — check room items (scenery)
    local keyword = args[1]:lower()
    local item_desc = room:get_item(keyword)
    if item_desc then
        -- Resolve lfun: item descriptions can be strings or functions
        local resolved = Object.resolve(item_desc, room)
        if resolved then
            player:send(resolved)
        else
            player:send("You see nothing special.")
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
                player:send(player_obj:examine())
            else
                player:send(char_data.name .. " is here.")
            end
            return
        end
    end

    player:send("You don't see that here.")
end

return M
