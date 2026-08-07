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
M.permission = "cmd.reload"

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
    local player = get_player(session_id)
    if not player then return end

    if not args[1] then
        local lines = {}
        table.insert(lines, "Usage: reload <module|pattern>")
        table.insert(lines, "Examples:")
        table.insert(lines, "  reload login          — reload a single module")
        table.insert(lines, "  reload lib/*          — reload all lib modules")
        table.insert(lines, "  reload daemons/*_d    — reload matching daemons")
        player:send(table.concat(lines, "\r\n"))
        return
    end

    if type(reload) ~= "function" then
        player:send("{red}Reload efun not available.{/}")
        return
    end

    -- Normalize slashes for display consistency
    local input = args[1]:gsub("\\", "/")

    -- Check for glob pattern
    if input:find("*") then
        local matches = expand_glob(input)
        if #matches == 0 then
            player:send("{yellow}No loaded modules match '" .. input .. "'.{/}")
            return
        end

        local lines = {}
        table.insert(lines, "{cyan}Reloading " .. #matches .. " module(s) matching '" .. input .. "':{/}")
        for _, mod_name in ipairs(matches) do
            -- The reload efun expects slash-separated paths
            local reload_path = mod_name:gsub("%.", "/")
            table.insert(lines, "  " .. mod_name)
            reload(reload_path)
        end
        table.insert(lines, "{green}Done.{/} Check server log for results.")
        player:send(table.concat(lines, "\r\n"))
    else
        -- Single module reload (existing behavior)
        local module_name = input
        player:send("{cyan}Reloading '" .. module_name .. "'...{/}")
        reload(module_name)
        player:send("{green}Reload request sent.{/} Check server log for result.")
    end
end

return M
