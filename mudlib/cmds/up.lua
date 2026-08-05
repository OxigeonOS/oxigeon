-- mudlib/cmds/up.lua — Go up.
--
-- `up`, `down`, `in` and `out` were in `movement.OPPOSITES` and in area
-- files from the beginning, and had no command. A stair you can describe and
-- cannot climb is a stair that only an admin with `goto` can use.

local movement = require('lib.movement')
local M = {}
M.name = 'up'
M.aliases = {'u'}
M.category = 'navigation'
M.summary = 'Go up.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'up')
end
return M
