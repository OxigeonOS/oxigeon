-- mudlib/cmds/journal.lua — Read server journal entries
-- Displays recent journal entries from logs/journal.log.
-- Requires daemon.journal_d.read permission.

local M = {}

M.name       = "journal"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Read server journal. Usage: journal [count] [level]"
M.permission = "daemon.journal_d.read"

-- Map level argument to the filter string used by journal_read
local LEVEL_ALIASES = {
    e = "error", err = "error",  error = "error",
    w = "warn",  warn = "warn",  warning = "warn",
    i = "info",  info = "info",
    d = "debug", debug = "debug",
    t = "trace", trace = "trace",
}

function M.execute(session_id, args_str, args)
    -- Parse args: "journal [count] [level]" or "journal [level] [count]"
    local count = 20
    local level = nil

    for _, tok in ipairs(args) do
        local n = tonumber(tok)
        if n then
            count = math.min(math.max(1, math.floor(n)), 200)
        else
            local lv = LEVEL_ALIASES[tok:lower()]
            if lv then level = lv end
        end
    end

    -- Fetch entries via DAEMON or directly via efun
    local entries
    if DAEMON and DAEMON.journal then
        entries = DAEMON.journal.recent(count, level)
    elseif type(journal_read) == "function" then
        entries = journal_read(count, level)
    else
        send(session_id, "\r\njournal_d not available.\r\n")
    
        return
    end

    send(session_id, "\r\n")
    if #entries == 0 then
        local filter_str = level and (" [" .. level .. "]") or ""
        send(session_id, "No journal entries" .. filter_str .. ".\r\n")
    else
        local header = string.format("=== Journal (last %d", #entries)
        if level then header = header .. ", level=" .. level end
        header = header .. ") ===\r\n"
        send(session_id, header)

        for _, raw in ipairs(entries) do
            local line
            if DAEMON and DAEMON.journal then
                line = DAEMON.journal.format_entry(raw)
            else
                -- Minimal parse fallback
                local lvl = raw:match('"level"%s*:%s*"([^"]+)"') or "?"
                local msg = raw:match('"msg"%s*:%s*"([^"]*)"') or raw
                line = "[" .. lvl:upper():sub(1,5) .. "] " .. msg
            end
            send(session_id, "  " .. line .. "\r\n")
        end
    end


end

return M
