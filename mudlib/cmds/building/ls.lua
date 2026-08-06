-- mudlib/cmds/building/ls.lua — What is in this directory?
--
-- Directories first, then files, both alphabetical. Sizes are human, because a
-- builder comparing `rooms.lua` against `items.lua` wants "2.1 K", not "2143".
--
-- ─── Hidden entries are counted, not vanished ────────────────────────────────
--
-- A directory you may not read is listed by *name* and not by contents, and the
-- footer says which permission you are missing. Omitting it silently makes `ls`
-- look broken; showing its contents would be the leak. The name of a directory
-- is not a secret — `/mudlib/admin` is in the repository — and naming the
-- permission turns "it does not work" into something a builder can go and ask
-- for.

local M = {}

M.name       = "ls"
M.aliases    = { "dir" }
M.category   = "building"
M.summary    = "List a directory in the file tree."
M.usage      = {
    "ls              the directory you are in",
    "ls <path>       absolute or relative",
    "ls -l <path>    with sizes and which layer each came from",
}
M.permission = "cmd.ls"

--- Bytes as something worth reading at a glance.
local function human(n)
    if n < 1024 then return string.format("%d B", n) end
    if n < 1024 * 1024 then return string.format("%.1f K", n / 1024) end
    return string.format("%.1f M", n / (1024 * 1024))
end

--- Order: directories first, then files, alphabetical within each.
---
--- Deliberately not "alphabetical over everything". A directory and a file are
--- different kinds of answer to "what is here", and an area directory holding
--- four files and one subdirectory reads much better with the subdirectory at
--- the top than buried between `items.lua` and `mobs.lua`.
local function by_kind_then_name(a, b)
    if a.is_dir ~= b.is_dir then return a.is_dir end
    return tostring(a.name) < tostring(b.name)
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.fs) then
        player:send("{red}The file shell is unavailable (fs_d is not loaded).{/}")
        return
    end

    local flags, target = DAEMON.fs.flags(args_str)
    local long = flags.l

    local virtual = DAEMON.fs.resolve(session_id, target)

    -- Reading the directory itself may be gated. Ask before listing, so the
    -- refusal names the permission rather than looking like an empty directory.
    local missing = DAEMON.fs.missing_permission(session_id, virtual, "read")
    if missing then
        player:send("{red}" .. virtual .. " — you lack '" .. missing .. "'.{/}")
        return
    end

    local entries, why = DAEMON.fs.list(virtual)
    if not entries then
        player:send("{red}" .. (why or ("no such directory: " .. virtual)) .. "{/}")
        return
    end

    table.sort(entries, by_kind_then_name)

    local lines = { "{cyan}" .. virtual .. "{/}" }
    local dirs, files, hidden = 0, 0, {}

    for _, e in ipairs(entries) do
        local child = (virtual == "/" and "/" or virtual .. "/") .. e.name
        local blocked = e.is_dir and DAEMON.fs.missing_permission(session_id, child, "read")

        if blocked then
            hidden[blocked] = true
        end

        if e.is_dir then
            dirs = dirs + 1
        else
            files = files + 1
        end

        local name = e.is_dir and (e.name .. "/") or e.name
        local size = e.is_dir and "-" or human(e.size or 0)

        if long then
            -- The root column only earns its place when the layers can differ:
            -- at `/` every entry *is* a root, and inside one they all share it.
            local root = ""
            if virtual == "/" then
                root = ""
            elseif e.root then
                root = "  {yellow}[" .. e.root .. "]{/}"
            end
            lines[#lines + 1] = string.format("  %8s  %s%s", size, name, root)
        else
            lines[#lines + 1] = "  " .. name
        end
    end

    if #entries == 0 then
        lines[#lines + 1] = "  {yellow}(empty){/}"
    end

    local summary = string.format("  %d file%s, %d director%s.",
        files, files == 1 and "" or "s",
        dirs, dirs == 1 and "y" or "ies")

    local names = {}
    for perm in pairs(hidden) do names[#names + 1] = perm end
    if #names > 0 then
        table.sort(names)
        summary = summary .. "  {yellow}(some entries not readable — you lack "
            .. table.concat(names, ", ") .. "){/}"
    end
    lines[#lines + 1] = summary

    -- `send_paged` rather than `send`: a listing of `/mudlib/cmds` is seventy
    -- entries and a screenful is not, and a file name must not be word-wrapped.
    player:send_paged(table.concat(lines, "\r\n"))
end

return M
