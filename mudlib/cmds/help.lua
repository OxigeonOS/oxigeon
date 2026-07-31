-- mudlib/cmds/help.lua — Help system (stubbed)
-- Full categorized help (by directory structure, topic lookup) is a future feature.
-- This stub lists available commands and reserves the 'help <topic>' syntax.

local M = {}

M.name       = "help"
M.aliases    = { "?" }
M.category   = "general"
M.summary    = "Show available commands or get help on a topic."
M.permission = nil

-- Static command summary shown when no topic is given.
-- When the full help system is implemented this will be auto-generated
-- from the command registry (M.category + M.summary metadata).
local COMMAND_LIST = [[

  Navigation
  ----------
  look, l      Look at your surroundings
  north, n     Go north
  south, s     Go south
  east, e      Go east
  west, w      Go west
  ne, nw       Go northeast / northwest
  se, sw       Go southeast / southwest

  General
  -------
  who          Show who is connected
  time         Show the current server time
  help, ?      Show this help
  quit         Leave the game

  Communication
  -------------
  say <msg>    Say something to all players
  ' <msg>      Shorthand for 'say'

  Admin
  -----
  reload <mod> Hot-reload a Lua module (requires permission)

  Type 'help <topic>' for details on a topic.
  (Full topic help system coming soon.)

]]

function M.execute(session_id, args_str, args)
    if args[1] then
        -- Topic lookup — stub for now
        local topic = args_str:lower()
        send(session_id, "\r\nHelp topic '" .. topic .. "' is not yet available.\r\n")
        send(session_id, "Type 'help' for the command list.\r\n")
    else
        send(session_id, COMMAND_LIST)
    end

end

return M
