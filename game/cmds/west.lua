local movement = require('lib.movement')
local M = {}
M.name = 'west'
M.aliases = {'w'}
M.category = 'navigation'
M.summary = 'Go west.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'west')
end
return M
