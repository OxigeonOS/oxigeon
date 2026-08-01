local M = {}

M.name       = "say"
M.aliases    = { "'" }    -- traditional MUD alias: 'hello world
M.category   = "communication"
M.summary    = "Say something aloud to those nearby."
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if args_str == "" then
        player:send("{red}Say what?{/}")
        return
    end

    -- Send to the speaker
    player:send("{cyan}You say:{/} " .. args_str)

    -- Broadcast to the room (excluding the speaker)
    player:message_room("{cyan}" .. player.name .. " says:{/} " .. args_str)
end

return M
