local movement = require('lib.movement')
local M = {}
M.name = 'southeast'
M.aliases = {'se'}
M.category = 'navigation'
M.summary = 'Go southeast.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'southeast')
end
return M
