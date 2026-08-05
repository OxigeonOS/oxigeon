-- mudlib/cmds/in.lua — Go in.
--
-- See `up.lua`: these four directions were in `movement.OPPOSITES` and usable
-- in an area file, and had no verb behind them.
--
-- No single-letter alias. `i` is `inventory` and has been for as long as MUDs
-- have had one, and taking it for a direction nobody uses often would break the
-- most-typed command in the game.

local movement = require('lib.movement')
local M = {}
M.name = 'in'
M.aliases = {}
M.category = 'navigation'
M.summary = 'Go in.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'in')
end
return M
