-- mudlib/cmds/say.lua — Say something to everyone in the same room

local M = {}

M.name       = "say"
M.aliases    = { "'" }    -- traditional MUD alias: 'hello world
M.category   = "communication"
M.summary    = "Say something aloud to those nearby."
M.permission = nil

function M.execute(session_id, args_str, args)
    if args_str == "" then
        send(session_id, "\r\nSay what?\r\n")
        return
    end

    local player = get_player(session_id)
    if not player then
        send(session_id, "\r\nYou need to be in the game to do that.\r\n")
        return
    end

    -- Send to the speaker
    player:send("You say: " .. args_str)

    -- Broadcast to the room (excluding the speaker)
    player:message_room(player.name .. " says: " .. args_str)
end

return M
