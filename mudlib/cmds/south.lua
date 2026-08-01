local movement = require('lib.movement')
local M = {}
M.name = 'south'
M.aliases = {'s'}
M.category = 'navigation'
M.summary = 'Go south.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'south')
end
return M
