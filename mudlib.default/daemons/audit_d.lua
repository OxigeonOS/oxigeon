-- mudlib/daemons/audit_d.lua
-- Audit Daemon — tracks privileged command runs and efun permission events.
--
-- Exposes:
--   DAEMON.audit.log(action, success, reason, extra)
--   DAEMON.audit.after_command(verb, session_id, args_str, ok, err)
--   DAEMON.audit.watch(verb, condition)      -- "success"|"fail"|"all"
--   DAEMON.audit.unwatch(verb)
--   DAEMON.audit.watch_list()               -- returns current watch table
--   DAEMON.audit.load_watch()               -- load from logs/audit_watch.json
--   DAEMON.audit.save_watch()               -- persist to logs/audit_watch.json
--   DAEMON.audit.recent(n)                  -- last n entries (formatted strings)

local M = {}

-- ─── Watch table ─────────────────────────────────────────────────────────────
-- verb -> "success" | "fail" | "all"
local _watch = {}

-- Rooted explicitly. The file efuns are jailed to two trees, and an unprefixed
-- *write* stays in the mudlib while an unprefixed *read* prefers the game layer
-- — so a stray `game/logs/audit_watch.json` would shadow the one this daemon
-- writes, for ever, with nothing reporting it. A file a daemon owns should say
-- which tree it lives in rather than rely on the defaults agreeing.
local WATCH_FILE = "mudlib:logs/audit_watch.json"

-- Helpers for resolving char name from session
local function char_name_for(session_id)
    if not session_id or session_id == "" then return "" end
    local s = get_session(session_id)
    if not s or not s.character_id then return "" end
    local char = get_character(s.character_id)
    return char and char.name or tostring(s.character_id)
end

-- ─── Core log function ───────────────────────────────────────────────────────

--- Write an audit entry. Uses the audit_write efun (backed by Rust GameLogger).
--- @param action  string   e.g. "cmd.spawn", "efun.reload"
--- @param success boolean
--- @param reason  string|nil  optional human-readable reason
function M.log(action, success, reason)
    if type(audit_write) == "function" then
        audit_write(action, success == true, reason)
    else
        -- Fallback: write to journal
        local msg = string.format("[AUDIT] action=%s success=%s reason=%s",
            action, tostring(success), reason or "nil")
        if type(journal_write) == "function" then
            journal_write("info", msg)
        end
    end
end

-- ─── Command audit hook ───────────────────────────────────────────────────────

--- Called by commands.lua after every command execution.
--- @param verb       string   the command verb
--- @param session_id string
--- @param args_str   string   raw argument string (may be truncated in log)
--- @param ok         boolean  pcall success
--- @param err        any      error if not ok
function M.after_command(verb, session_id, args_str, ok, err)
    local cond = _watch[verb]
    if not cond then return end  -- not being watched

    local should_log = false
    if cond == "all" then
        should_log = true
    elseif cond == "success" and ok then
        should_log = true
    elseif cond == "fail" and not ok then
        should_log = true
    end

    if not should_log then return end

    local char = char_name_for(session_id)
    local reason = nil
    if not ok and err then
        reason = tostring(err):sub(1, 200)  -- cap at 200 chars
    end

    local extra_json = string.format(
        '{"verb":"%s","char":"%s","args":"%s"}',
        verb,
        char:gsub('"', '\\"'),
        (args_str or ""):sub(1, 80):gsub('"', '\\"')
    )

    -- Use audit_write directly; the Rust side will fill in session_id from
    -- the current session context. We write the action as "cmd.<verb>".
    if type(audit_write) == "function" then
        audit_write("cmd." .. verb, ok == true, reason)
    end

    -- Also log to journal for searchability
    if type(journal_write) == "function" then
        journal_write(
            ok and "info" or "warn",
            string.format("[audit:cmd] %s %s %s", char, verb, ok and "ok" or "FAIL"),
            extra_json
        )
    end
end

-- ─── Watch table management ──────────────────────────────────────────────────

--- Add a command to the audit watch list.
--- @param verb      string
--- @param condition string  "success"|"fail"|"all"
function M.watch(verb, condition)
    local valid = {success=true, fail=true, all=true}
    if not valid[condition] then
        return false, "condition must be 'success', 'fail', or 'all'"
    end
    _watch[verb] = condition
    M.save_watch()
    M.log("audit_d.watch.add", true, verb .. "=" .. condition)
    return true
end

--- Remove a command from the audit watch list.
function M.unwatch(verb)
    if not _watch[verb] then return false end
    _watch[verb] = nil
    M.save_watch()
    M.log("audit_d.watch.rm", true, verb)
    return true
end

--- Return a copy of the current watch table.
function M.watch_list()
    local out = {}
    for verb, cond in pairs(_watch) do
        out[verb] = cond
    end
    return out
end

-- ─── Persistence ─────────────────────────────────────────────────────────────

--- Load watch table from logs/audit_watch.json.
function M.load_watch()
    if type(read_file) ~= "function" then return end
    local content = read_file(WATCH_FILE)
    if not content or content == "" then return end

    -- Simple JSON object parse for {"verb":"condition",...}
    -- We use a hand-rolled parser since we don't have a JSON library by default.
    -- Format is guaranteed to be flat key-value strings from our save_watch.
    for verb, cond in content:gmatch('"([^"]+)"%s*:%s*"([^"]+)"') do
        _watch[verb] = cond
    end
end

--- Persist the watch table to logs/audit_watch.json.
---
--- `write_file` *returns* failure rather than raising it, so the result has to
--- be read. A watch list that silently stopped persisting would go unnoticed
--- until a restart dropped every watch somebody had set.
--- @return boolean ok
function M.save_watch()
    if type(write_file) ~= "function" then return false end
    local parts = {}
    for verb, cond in pairs(_watch) do
        parts[#parts+1] = string.format('  "%s": "%s"', verb, cond)
    end
    local json = "{\n" .. table.concat(parts, ",\n") .. "\n}"

    local ok, err = write_file(WATCH_FILE, json)
    if not ok then
        local message = "AUDIT_D: could not save the watch list to "
            .. WATCH_FILE .. ": " .. tostring(err)
        log("error", message)
        if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
    end
    return ok and true or false
end

-- ─── Reading entries ─────────────────────────────────────────────────────────

--- Read the last n audit entries. Returns array of raw JSON strings.
--- Requires efun.audit_read (enforced by the audit_read efun).
--- @param n number  default 20
function M.recent(n)
    if type(audit_read) ~= "function" then return {} end
    return audit_read(n or 20)
end

-- ─── Init ────────────────────────────────────────────────────────────────────

-- Load watch table from disk on module load
M.load_watch()

log("info", "audit_d daemon loaded (watching " .. (function()
    local c = 0
    for _ in pairs(_watch) do c = c + 1 end
    return c
end)() .. " commands)")

return M
