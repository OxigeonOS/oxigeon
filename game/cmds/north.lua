local movement = require('lib.movement')
local M = {}
M.name = 'north'
M.aliases = {'n'}
M.category = 'navigation'
M.summary = 'Go north.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'north')
end
return M
