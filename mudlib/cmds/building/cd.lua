-- mudlib/cmds/building/cd.lua — Move around the file tree.
--
-- The tree has two mount points, `/game` and `/mudlib`, and `cd` with no
-- argument goes to `~`: the area you are building, or the virtual root if you
-- are not. See mudlib/daemons/fs_d.lua for the path rules.

local M = {}

M.name       = "cd"
M.aliases    = {}
M.category   = "building"
M.summary    = "Change the file-tree directory you are working in."
M.usage      = {
    "cd <path>    absolute (/game/areas) or relative (crypt, ..)",
    "cd           your build area, or / if you are not building",
    "cd -         back to where you were",
}
M.permission = "cmd.cd"

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.fs) then
        player:send("{red}The file shell is unavailable (fs_d is not loaded).{/}")
        return
    end

    -- The whole remainder, not `args[1]`: a directory may contain a space, and
    -- splitting on one would make it unreachable rather than merely awkward.
    local target = args_str
    if not target or target:match("^%s*$") then target = "~" end
    target = target:gsub("^%s+", ""):gsub("%s+$", "")

    local resolved, why = DAEMON.fs.chdir(session_id, target)
    if not resolved then
        player:send("{red}" .. tostring(why) .. "{/}")
        return
    end

    -- Say where you landed rather than staying silent. `cd ..` from a mount
    -- point lands at `/`, and `cd -` lands somewhere you may not remember;
    -- both are cases where silence reads as a failure.
    player:send("{cyan}" .. resolved .. "{/}")
end

return M
