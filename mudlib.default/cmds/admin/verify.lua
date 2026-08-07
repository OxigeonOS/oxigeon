-- mudlib/cmds/admin/verify.lua — Does this compile, and does this area work?
--
-- Two questions, and it used to answer only the first. `verify <path>` is the
-- compile check and is unchanged; `verify area <name>` is the content lint, and
-- the difference matters because a file can compile perfectly and still describe
-- a room with an exit into nothing.
--
-- The lint is in `daemons/verify_d.lua`. It reports and never changes anything:
-- the gate lives at `olc save`, which is where a write is about to happen.

local M = {}

M.name       = "verify"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Compile-check a file, or lint a whole area."
M.usage      = {
    "verify <path>          does this file compile? e.g. `verify cmds/who.lua`",
    "verify area <name>     lint an area, read from disk",
    "verify                 lint the area you are building",
    "verify all             every discovered area",
    "verify prototypes      lint the prototype library itself",
}
M.permission = "cmd.verify"

local function compile_check(player, path)
    player:send("{cyan}Verifying: {yellow}" .. path .. "{/}")

    local ok, err = verify_file(path)
    if ok then
        player:send("  {green}✓ File compiles successfully.{/}")
        return
    end

    local lines = { "  {red}✗ Compile error:{/}" }
    for line in (err or "unknown error"):gmatch("[^\n]+") do
        lines[#lines + 1] = "      " .. line
    end
    player:send(table.concat(lines, "\r\n"))
end

local function lint(player, area_name)
    if not DAEMON.verify then
        player:send("{red}The linter is unavailable (verify_d is not loaded).{/}")
        return
    end
    local report = DAEMON.verify.area(area_name)
    player:send_paged(table.concat(DAEMON.verify.render(report), "\r\n"))
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    args_str = (args_str or ""):gsub("^%s+", ""):gsub("%s+$", "")

    -- Bare `verify` lints what you are building. Reciting a usage line at
    -- somebody mid-build who typed the obvious thing is not help.
    if args_str == "" then
        if DAEMON.olc and DAEMON.olc.is_active(session_id) then
            return lint(player, DAEMON.olc.get_state(session_id).area_name)
        end
        return player:send_lines(M.usage)
    end

    local head, rest = args_str:match("^(%S+)%s*(.*)$")

    if head:lower() == "area" then
        if rest == "" then return player:send_lines(M.usage) end
        return lint(player, rest:match("^(%S+)"))
    end

    -- Worth its own form rather than folding into `all`: a broken prototype
    -- breaks every area that names it, and the area reports would tell you which
    -- children noticed rather than what is actually wrong.
    if head:lower() == "prototypes" or head:lower() == "proto" then
        if not DAEMON.verify then
            return player:send("{red}The linter is unavailable (verify_d is not loaded).{/}")
        end
        local report = DAEMON.verify.prototypes()
        return player:send_paged(table.concat(DAEMON.verify.render(report), "\r\n"))
    end

    if head:lower() == "all" then
        local areaload = require('lib.areaload')
        local names = areaload.discover()
        if #names == 0 then
            return player:send("{yellow}No areas were discovered.{/}")
        end
        for _, name in ipairs(names) do lint(player, name) end
        return
    end

    -- Anything else is a path. `verify` has always meant that and the muscle
    -- memory is worth keeping.
    compile_check(player, head)
end

return M
