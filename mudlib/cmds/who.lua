-- mudlib/cmds/who.lua — List connected players

local M = {}

M.name       = "who"
M.aliases    = {}
M.category   = "general"
M.summary    = "Show who is currently connected."
M.permission = nil

function M.execute(session_id, args_str, args)
    local sessions = all_sessions()
    local playing = {}
    local total = #sessions

    for _, sid in ipairs(sessions) do
        local s = get_session(sid)
        if s and s.state == "playing" and s.character_id then
            local char = get_character(s.character_id)
            if char then
                playing[#playing + 1] = char.name
            end
        end
    end

    send(session_id, "\r\n")
    if #playing == 0 then
        send(session_id, "No players are currently in the game.\r\n")
    else
        send(session_id, "Players in the game (" .. #playing .. "):\r\n")
        for _, name in ipairs(playing) do
            send(session_id, "  " .. name .. "\r\n")
        end
    end
    send(session_id, "Total connections: " .. total .. "\r\n")

end

return M
