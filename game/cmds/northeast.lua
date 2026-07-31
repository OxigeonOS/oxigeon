local movement = require('lib.movement')
local M = {}
M.name = 'northeast'
M.aliases = {'ne'}
M.category = 'navigation'
M.summary = 'Go northeast.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'northeast')
end
return M
