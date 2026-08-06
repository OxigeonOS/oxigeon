-- mudlib/cmds/audit.lua — Audit log management
--
-- Subcommands:
--   audit [n]                     Read last n audit entries (default 20)
--   audit list                    Show the command watch list
--   audit add <cmd> <condition>   Watch a command (success|fail|all)
--   audit rm <cmd>                Remove a command from the watch list

local M = {}

M.name       = "audit"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Audit log: read entries and manage the command watch list."
M.permission = "cmd.audit"

-- Permissions for the management subcommands (checked inline)
local MANAGE_PERM = "cmd.audit.manage"

--- Format a raw audit JSON line for display.
local function format_entry(raw)
    local ts      = raw:match('"ts"%s*:%s*"([^"]+)"')     or "?"
    local char    = raw:match('"char"%s*:%s*"([^"]*)"')   or ""
    local action  = raw:match('"action"%s*:%s*"([^"]+)"') or "?"
    local success = raw:match('"success"%s*:%s*(%a+)')    or "?"
    local reason  = raw:match('"reason"%s*:%s*"([^"]*)"')

    local when = ts:match("T(.-)Z") or ts
    local status = success == "true" and "{green}✓{/}" or "{red}✗{/}"
    local who = char ~= "" and ("[{cyan}" .. char .. "{/}] ") or ""
    local note = reason and (" — {yellow}" .. reason .. "{/}") or ""
    return string.format("  %s %s %s%s%s", status, when, who, action, note)
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    
    local sub = args[1] and args[1]:lower() or nil

    -- ─── audit list ──────────────────────────────────────────────────────
    if sub == "list" then
        local lines = {}
        table.insert(lines, "{bold}=== Audit Watch List ==={/}")
        if not (DAEMON and DAEMON.audit) then
            table.insert(lines, "  {red}audit_d daemon not loaded.{/}")
            player:send(table.concat(lines, "\r\n"))
            return
        end
        local wl = DAEMON.audit.watch_list()
        local count = 0
        for verb, cond in pairs(wl) do
            table.insert(lines, string.format("  {cyan}%-20s{/} %s", verb, cond))
            count = count + 1
        end
        if count == 0 then
            table.insert(lines, "  (no commands being watched)")
        end
        table.insert(lines, "")
        table.insert(lines, "Type 'audit add <cmd> <success|fail|all>' to add.")
        player:send(table.concat(lines, "\r\n"))
        return
    end

    -- ─── audit add <cmd> <condition> ─────────────────────────────────────
    if sub == "add" then
        -- Requires higher permission
        if type(has_permission) == "function" and not has_permission(session_id, MANAGE_PERM) then
            player:send("{red}Permission denied. Requires: " .. MANAGE_PERM .. "{/}")
            return
        end
        local verb = args[2]
        local cond = args[3] and args[3]:lower()
        if not verb or not cond then
            player:send("Usage: audit add <command> <success|fail|all>")
            return
        end
        if not DAEMON or not DAEMON.audit then
            player:send("{red}audit_d daemon not loaded.{/}")
            return
        end
        local ok, err = DAEMON.audit.watch(verb, cond)
        if ok then
            player:send(string.format("{green}Now auditing '{cyan}%s{green}' on: {yellow}%s{/}", verb, cond))
        else
            player:send("{red}Error: " .. (err or "unknown") .. "{/}")
        end
        return
    end

    -- ─── audit rm <cmd> ──────────────────────────────────────────────────
    if sub == "rm" or sub == "remove" then
        if type(has_permission) == "function" and not has_permission(session_id, MANAGE_PERM) then
            player:send("{red}Permission denied. Requires: " .. MANAGE_PERM .. "{/}")
            return
        end
        local verb = args[2]
        if not verb then
            player:send("Usage: audit rm <command>")
            return
        end
        if not DAEMON or not DAEMON.audit then
            player:send("{red}audit_d daemon not loaded.{/}")
            return
        end
        local removed = DAEMON.audit.unwatch(verb)
        if removed then
            player:send("{green}No longer auditing '{cyan}" .. verb .. "{green}'.{/}")
        else
            player:send("{yellow}'" .. verb .. "' was not in the watch list.{/}")
        end
        return
    end

    -- ─── audit [n] — read recent entries ─────────────────────────────────
    local count = 20
    if sub then
        local n = tonumber(sub)
        if n then
            count = math.min(math.max(1, math.floor(n)), 200)
        end
    end

    local entries
    if DAEMON and DAEMON.audit then
        entries = DAEMON.audit.recent(count)
    elseif type(audit_read) == "function" then
        entries = audit_read(count)
    else
        player:send("{red}audit_d not available.{/}")
        return
    end

    local lines = {}
    table.insert(lines, string.format("{bold}=== Audit Log (last %d) ==={/}", #entries))
    if #entries == 0 then
        table.insert(lines, "  (no entries)")
    else
        for _, raw in ipairs(entries) do
            table.insert(lines, format_entry(raw))
        end
    end
    table.insert(lines, "")
    table.insert(lines, "Tip: 'audit list' shows the command watch list.")
    player:send(table.concat(lines, "\r\n"))
end

return M
