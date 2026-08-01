local movement = require('lib.movement')
local M = {}
M.name = 'east'
M.aliases = {'e'}
M.category = 'navigation'
M.summary = 'Go east.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'east')
end
return M
