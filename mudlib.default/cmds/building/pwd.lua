-- mudlib/cmds/building/pwd.lua — Where am I in the file tree?
--
-- Three lines, and worth having: a working directory you cannot print is a
-- working directory you do not trust, and every relative `ls` and `cat` is
-- resolved against it.

local M = {}

M.name       = "pwd"
M.aliases    = {}
M.category   = "building"
M.summary    = "Print the file-tree directory you are working in."
M.usage      = "pwd"
M.permission = "cmd.pwd"

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.fs) then
        player:send("{red}The file shell is unavailable (fs_d is not loaded).{/}")
        return
    end

    player:send(DAEMON.fs.cwd(session_id))
end

return M
