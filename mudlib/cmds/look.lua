local M = {}

M.name = 'look'
M.aliases = {'l'}
M.category = 'navigation'
M.summary = 'Look at your surroundings or examine an item.'
M.permission = nil

local Object = require('lib.object')
local Light  = require('lib.light')

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

    -- `Room.light_level` has been a field since rooms existed and nothing read
    -- it. This is what reads it — and it is checked before *everything*,
    -- because "look at the lever" in a pitch-dark room should not work either.
    -- Exits stay listed: you can feel your way to a doorway.
    if not Light.can_see(player, room) then
        player:send("{cyan}" .. Light.DARKNESS .. "{/}")
        local exits = {}
        for dir, exit in pairs(room.exits or {}) do
            if not (type(exit) == "table" and exit.hidden) then exits[#exits + 1] = dir end
        end
        table.sort(exits)
        if #exits > 0 then
            player:send("You can feel your way " .. table.concat(exits, ", ") .. ".")
        end
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
