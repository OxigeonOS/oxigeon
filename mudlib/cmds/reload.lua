-- mudlib/cmds/reload.lua — Hot-reload a Lua module (admin only)
-- Requires the "efun.reload" permission (enforced once Phase 2 is complete).
-- Until then the permission check in commands.lua is a no-op (has_permission not yet defined).

local M = {}

M.name       = "reload"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Hot-reload a Lua module. Usage: reload <module>"
M.permission = "efun.reload"   -- requires this permission string (Phase 2)

function M.execute(session_id, args_str, args)
    if not args[1] then
        send(session_id, "\r\nUsage: reload <module>\r\n")
        send(session_id, "Example: reload login\r\n")
    
        return
    end

    local module_name = args[1]
    send(session_id, "\r\nReloading '" .. module_name .. "'...\r\n")

    -- reload() efun sends LuaCommand::Reload to the engine
    if type(reload) == "function" then
        reload(module_name)
        send(session_id, "Reload request sent. Check server log for result.\r\n")
    else
        send(session_id, "Reload efun not available.\r\n")
    end


end

return M
