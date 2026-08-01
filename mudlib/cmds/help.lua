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
  reload <mod>  Hot-reload a Lua module (requires permission)
  stat <target> Inspect a player or room in detail
  mudstatus     Server status dashboard
  areas         List/manage/reset areas
  tasks         List/manage background tasks
  events        List event subscriptions
  awho          Detailed admin who list

  Type 'help <topic>' for details on a topic.
  (Full topic help system coming soon.)

]]

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if args[1] then
        -- Topic lookup — stub for now
        local topic = args_str:lower()
        local lines = {}
        table.insert(lines, "Help topic '" .. topic .. "' is not yet available.")
        table.insert(lines, "Type 'help' for the command list.")
        player:send(table.concat(lines, "\r\n"))
    else
        -- Remove leading/trailing newlines if any, then send
        player:send(COMMAND_LIST:gsub("^%s+", ""):gsub("%s+$", ""))
    end
end

return M
