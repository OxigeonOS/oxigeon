-- mudlib/cmds/quit.lua — Quit/exit the game

local M = {}

M.name       = "quit"
M.aliases    = { "exit", "bye" }
M.category   = "general"
M.summary    = "Disconnect from the game."
M.permission = nil  -- any playing session

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if player then
        player:send("{cyan}Farewell! Until next time.{/}")
    end
    disconnect(session_id)
end

return M
