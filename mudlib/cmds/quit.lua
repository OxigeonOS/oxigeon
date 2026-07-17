-- mudlib/cmds/quit.lua — Quit/exit the game

local M = {}

M.name       = "quit"
M.aliases    = { "exit", "bye" }
M.category   = "general"
M.summary    = "Disconnect from the game."
M.permission = nil  -- any playing session

function M.execute(session_id, args_str, args)
    send(session_id, "\r\nFarewell! Until next time.\r\n")
    disconnect(session_id)
end

return M
