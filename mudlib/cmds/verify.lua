-- mudlib/cmds/verify.lua — Compile-check a Lua file in-game
-- Uses the verify_file(path) efun which reads the file and compiles it
-- without executing it. Reports success or the full error+stacktrace.

local M = {}

M.name       = "verify"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Compile-check a Lua file without executing it. Usage: verify <path>"
M.permission = "efun.verify"

function M.execute(session_id, args_str, args)
    if not args[1] then
        send(session_id, "\r\nUsage: verify <path>\r\n")
        send(session_id, "Example: verify cmds/who.lua\r\n")
        send(session_id, "         verify login.lua\r\n")

        return
    end

    local path = args_str:match("^%S+")  -- first token = path
    send(session_id, "\r\nVerifying: " .. path .. "\r\n")

    local ok, err = verify_file(path)

    if ok then
        send(session_id, "\r\n  \u2713 File compiles successfully.\r\n")
    else
        send(session_id, "\r\n  \u2717 Compile error:\r\n")
        -- Indent each line of the error
        for line in (err or "unknown error"):gmatch("[^\n]+") do
            send(session_id, "      " .. line .. "\r\n")
        end
    end


end

return M
