-- mudlib/cmds/building/cat.lua — Read a file.
--
-- The step that closes the loop: `ls` to find it, `cat` to see it, `verify` to
-- check it, `olc set` to fix it. Without this, every "what does that file
-- actually say" costs a trip out of the game.
--
-- `-n` numbers the lines, because `verify` and `verify_file` both report
-- positions and a line number you cannot see is not a position.

local M = {}

M.name       = "cat"
M.aliases    = {}
M.category   = "building"
M.summary    = "Show the contents of a file."
M.usage      = {
    "cat <path>       show a file",
    "cat -n <path>    with line numbers",
}
M.permission = "cmd.cat"

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.fs) then
        player:send("{red}The file shell is unavailable (fs_d is not loaded).{/}")
        return
    end

    local flags, target = DAEMON.fs.flags(args_str)
    local numbered = flags.n

    if target == "" then
        player:send_lines(M.usage)
        return
    end

    local virtual = DAEMON.fs.resolve(session_id, target)

    local missing = DAEMON.fs.missing_permission(session_id, virtual, "read")
    if missing then
        player:send("{red}" .. virtual .. " — you lack '" .. missing .. "'.{/}")
        return
    end

    -- A directory is a common typo, and "no such file" would send you looking
    -- for a spelling mistake that is not there.
    if DAEMON.fs.is_dir(virtual) then
        player:send("{red}" .. virtual .. " is a directory. Try: ls " .. virtual .. "{/}")
        return
    end

    local efun_path, why = DAEMON.fs.to_efun_path(virtual)
    if not efun_path then
        player:send("{red}" .. tostring(why) .. "{/}")
        return
    end

    local content = read_file(efun_path)
    if not content then
        player:send("{red}Cannot read " .. virtual .. "{/}")
        return
    end

    local lines = {}
    local n = 0
    for line in (content .. "\n"):gmatch("(.-)\r?\n") do
        n = n + 1
        lines[n] = numbered and string.format("%4d  %s", n, line) or line
    end
    -- `gmatch` over `content .. "\n"` yields one trailing empty line for a file
    -- that already ended in a newline. Dropping it keeps the count honest.
    if n > 0 and lines[n]:match("^%s*$") then
        lines[n] = nil
        n = n - 1
    end

    -- The header is coloured and sent first; the body is not.
    --
    -- A mudlib file is full of `{red}` and `{/}`, so rendering the body would
    -- paint the listing in the colours of the code you were trying to read, and
    -- stripping it would silently delete tags from the source you are
    -- inspecting. `literal` is the only honest answer for a file.
    player:send(string.format("{cyan}%s{/}  {yellow}%d line%s, %d bytes{/}",
        virtual, n, n == 1 and "" or "s", #content))
    player:send_paged(table.concat(lines, "\r\n"), { literal = true })
end

return M
