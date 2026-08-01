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
M.permission = "daemon.audit_d.read"

-- Permissions for the management subcommands (checked inline)
local MANAGE_PERM = "daemon.audit_d.manage"

--- Format a raw audit JSON line for display.
local function format_entry(raw)
    local ts      = raw:match('"ts"%s*:%s*"([^"]+)"')     or "?"
    local char    = raw:match('"char"%s*:%s*"([^"]*)"')   or ""
    local action  = raw:match('"action"%s*:%s*"([^"]+)"') or "?"
    local success = raw:match('"success"%s*:%s*(%a+)')    or "?"
    local reason  = raw:match('"reason"%s*:%s*"([^"]*)"')

    local when = ts:match("T(.-)Z") or ts
    local status = success == "true" and "\u{2713}" or "\u{2717}"
    local who = char ~= "" and ("[" .. char .. "] ") or ""
    local note = reason and (" — " .. reason) or ""
    return string.format("  %s %s %s%s%s", status, when, who, action, note)
end

function M.execute(session_id, args_str, args)
    local sub = args[1] and args[1]:lower() or nil

    -- ─── audit list ──────────────────────────────────────────────────────
    if sub == "list" then
        send(session_id, "\r\n=== Audit Watch List ===\r\n")
        if not (DAEMON and DAEMON.audit) then
            send(session_id, "  audit_d daemon not loaded.\r\n")
        
            return
        end
        local wl = DAEMON.audit.watch_list()
        local count = 0
        for verb, cond in pairs(wl) do
            send(session_id, string.format("  %-20s %s\r\n", verb, cond))
            count = count + 1
        end
        if count == 0 then
            send(session_id, "  (no commands being watched)\r\n")
        end
        send(session_id, "\r\nType 'audit add <cmd> <success|fail|all>' to add.\r\n")
    
        return
    end

    -- ─── audit add <cmd> <condition> ─────────────────────────────────────
    if sub == "add" then
        -- Requires higher permission
        if type(has_permission) == "function" and not has_permission(session_id, MANAGE_PERM) then
            send(session_id, "\r\nPermission denied. Requires: " .. MANAGE_PERM .. "\r\n")
        
            return
        end
        local verb = args[2]
        local cond = args[3] and args[3]:lower()
        if not verb or not cond then
            send(session_id, "\r\nUsage: audit add <command> <success|fail|all>\r\n")
        
            return
        end
        if not DAEMON or not DAEMON.audit then
            send(session_id, "\r\naudit_d daemon not loaded.\r\n")
        
            return
        end
        local ok, err = DAEMON.audit.watch(verb, cond)
        if ok then
            send(session_id, string.format(
                "\r\nNow auditing '%s' on: %s\r\n", verb, cond))
        else
            send(session_id, "\r\nError: " .. (err or "unknown") .. "\r\n")
        end
    
        return
    end

    -- ─── audit rm <cmd> ──────────────────────────────────────────────────
    if sub == "rm" or sub == "remove" then
        if type(has_permission) == "function" and not has_permission(session_id, MANAGE_PERM) then
            send(session_id, "\r\nPermission denied. Requires: " .. MANAGE_PERM .. "\r\n")
        
            return
        end
        local verb = args[2]
        if not verb then
            send(session_id, "\r\nUsage: audit rm <command>\r\n")
        
            return
        end
        if not DAEMON or not DAEMON.audit then
            send(session_id, "\r\naudit_d daemon not loaded.\r\n")
        
            return
        end
        local removed = DAEMON.audit.unwatch(verb)
        if removed then
            send(session_id, "\r\nNo longer auditing '" .. verb .. "'.\r\n")
        else
            send(session_id, "\r\n'" .. verb .. "' was not in the watch list.\r\n")
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
        send(session_id, "\r\naudit_d not available.\r\n")
    
        return
    end

    send(session_id, string.format("\r\n=== Audit Log (last %d) ===\r\n", #entries))
    if #entries == 0 then
        send(session_id, "  (no entries)\r\n")
    else
        for _, raw in ipairs(entries) do
            send(session_id, format_entry(raw) .. "\r\n")
        end
    end
    send(session_id, "\r\nTip: 'audit list' shows the command watch list.\r\n")

end

return M
