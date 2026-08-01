local movement = require('lib.movement')
local M = {}
M.name = 'northwest'
M.aliases = {'nw'}
M.category = 'navigation'
M.summary = 'Go northwest.'
M.permission = nil
function M.execute(session_id, args_str, args)
    movement.move(session_id, 'northwest')
end
return M
