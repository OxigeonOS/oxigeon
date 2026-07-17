-- mudlib/cmds/time.lua — Display server time

local M = {}

M.name       = "time"
M.aliases    = {}
M.category   = "general"
M.summary    = "Show the current server date and time."
M.permission = nil

function M.execute(session_id, args_str, args)
    local date_str = os_date("%A, %B %d %Y  %H:%M:%S")
    send(session_id, "\r\nServer time: " .. date_str .. "\r\n")
    send_prompt(session_id, "\r\n> ")
end

return M
