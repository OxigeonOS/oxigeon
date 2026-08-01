-- mudlib/cmds/time.lua — Display server time

local M = {}

M.name       = "time"
M.aliases    = {}
M.category   = "general"
M.summary    = "Show the current server date and time."
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local date_str = os_date("%A, %B %d %Y  %H:%M:%S")
    player:send("Server time: {yellow}" .. date_str .. "{/}")
end

return M
