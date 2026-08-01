local movement = require('lib.movement')
local M = {}
M.name = 'southwest'
M.aliases = {'sw'}
M.category = 'navigation'
M.summary = 'Go southwest.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'southwest')
end
return M
