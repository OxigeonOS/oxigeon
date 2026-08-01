-- mudlib/cmds/reload.lua — Hot-reload a Lua module (admin only)
-- Requires the "efun.reload" permission (enforced once Phase 2 is complete).
-- Until then the permission check in commands.lua is a no-op (has_permission not yet defined).
--
-- Supports glob patterns:
--   reload lib/*          — reload all modules under lib/
--   reload daemons/*_d    — reload all daemons matching *_d
--   reload lib/player     — reload a single module (existing behavior)

local M = {}

M.name       = "reload"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Hot-reload a Lua module. Usage: reload <module|pattern>"
M.permission = "efun.reload"   -- requires this permission string (Phase 2)

--- Convert a glob pattern (using * as wildcard) into a Lua pattern.
-- Slashes are normalized to dots (module-path style) for matching
-- against package.loaded keys.
-- @param glob string  e.g. "lib/*" or "daemons/*_d"
-- @return string      Lua pattern, e.g. "^lib%.[^%.]*$"
local function glob_to_pattern(glob)
    -- Normalize slashes to dots (Lua module paths use dots)
    local pat = glob:gsub("/", ".")
    -- Escape Lua pattern special chars (except *)
    pat = pat:gsub("([%(%)%.%%%+%-%?%[%]%^%$])", "%%%1")
    -- Convert * to match a single path segment (no dots)
    pat = pat:gsub("%*", "[^%.]*")
    return "^" .. pat .. "$"
end

--- Find all loaded modules matching a glob pattern.
-- @param glob string  e.g. "lib/*"
-- @return table       sorted array of matching module names
local function expand_glob(glob)
    local pattern = glob_to_pattern(glob)
    local matches = {}
    for mod_name, _ in pairs(package.loaded) do
        if type(mod_name) == "string" and mod_name:match(pattern) then
            matches[#matches + 1] = mod_name
        end
    end
    table.sort(matches)
    return matches
end

function M.execute(session_id, args_str, args)
    if not args[1] then
        send(session_id, "\r\nUsage: reload <module|pattern>\r\n")
        send(session_id, "Examples:\r\n")
        send(session_id, "  reload login          — reload a single module\r\n")
        send(session_id, "  reload lib/*          — reload all lib modules\r\n")
        send(session_id, "  reload daemons/*_d    — reload matching daemons\r\n")
        return
    end

    if type(reload) ~= "function" then
        send(session_id, "Reload efun not available.\r\n")
        return
    end

    -- Normalize slashes for display consistency
    local input = args[1]:gsub("\\", "/")

    -- Check for glob pattern
    if input:find("*") then
        local matches = expand_glob(input)
        if #matches == 0 then
            send(session_id, "\r\nNo loaded modules match '" .. input .. "'.\r\n")
            return
        end

        send(session_id, "\r\nReloading " .. #matches .. " module(s) matching '" .. input .. "':\r\n")
        for _, mod_name in ipairs(matches) do
            -- The reload efun expects slash-separated paths
            local reload_path = mod_name:gsub("%.", "/")
            send(session_id, "  " .. mod_name .. "\r\n")
            reload(reload_path)
        end
        send(session_id, "Done. Check server log for results.\r\n")
    else
        -- Single module reload (existing behavior)
        local module_name = input
        send(session_id, "\r\nReloading '" .. module_name .. "'...\r\n")
        reload(module_name)
        send(session_id, "Reload request sent. Check server log for result.\r\n")
    end
end

return M
