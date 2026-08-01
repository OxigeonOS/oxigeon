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
    local player = get_player(session_id)
    if not player then return end

    if not args[1] then
        local lines = {}
        table.insert(lines, "Usage: verify <path>")
        table.insert(lines, "Example: verify cmds/who.lua")
        table.insert(lines, "         verify login.lua")
        player:send(table.concat(lines, "\r\n"))
        return
    end

    local path = args_str:match("^%S+")  -- first token = path
    player:send("{cyan}Verifying: {yellow}" .. path .. "{/}")

    local ok, err = verify_file(path)

    if ok then
        player:send("  {green}✓ File compiles successfully.{/}")
    else
        local lines = {}
        table.insert(lines, "  {red}✗ Compile error:{/}")
        -- Indent each line of the error
        for line in (err or "unknown error"):gmatch("[^\n]+") do
            table.insert(lines, "      " .. line)
        end
        player:send(table.concat(lines, "\r\n"))
    end
end

return M
