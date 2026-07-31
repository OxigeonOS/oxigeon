-- mudlib/cmds/say.lua — Say something to all connected players

local M = {}

M.name       = "say"
M.aliases    = { "'" }    -- traditional MUD alias: 'hello world
M.category   = "communication"
M.summary    = "Say something aloud for everyone to hear."
M.permission = nil

function M.execute(session_id, args_str, args)
    if args_str == "" then
        send(session_id, "\r\nSay what?\r\n")

        return
    end

    local session = get_session(session_id)
    local name = "Someone"
    if session and session.character_id then
        local char = get_character(session.character_id)
        if char then name = char.name end
    end

    broadcast("\r\n" .. name .. " says: " .. args_str .. "\r\n")

end

return M
