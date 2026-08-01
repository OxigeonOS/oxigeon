-- mudlib/daemons/journal_d.lua
-- Journal Daemon — general server log for info/warn/error messages.
--
-- Any code can call DAEMON.journal.info/warn/error to write structured
-- entries to the server journal (logs/journal.log via Rust GameLogger).
--
-- Exposes:
--   DAEMON.journal.trace(msg, meta)
--   DAEMON.journal.debug(msg, meta)
--   DAEMON.journal.info(msg, meta)
--   DAEMON.journal.warn(msg, meta)
--   DAEMON.journal.error(msg, meta)
--   DAEMON.journal.write(level, msg, meta)
--   DAEMON.journal.recent(n, level)   -- last n entries, optional level filter

local M = {}

--- Write a journal entry at the given level.
--- @param level  string  "trace"|"debug"|"info"|"warn"|"error"
--- @param msg    string
--- @param meta   string|nil  optional JSON string for structured metadata
function M.write(level, msg, meta)
    if type(journal_write) ~= "function" then
        -- Fallback to log() if efun not yet available
        if type(log) == "function" then
            log(level, "[journal] " .. tostring(msg))
        end
        return false
    end
    return journal_write(level, tostring(msg), meta)
end

function M.trace(msg, meta) return M.write("trace", msg, meta) end
function M.debug(msg, meta) return M.write("debug", msg, meta) end
function M.info(msg, meta)  return M.write("info",  msg, meta) end
function M.warn(msg, meta)  return M.write("warn",  msg, meta) end
function M.error(msg, meta) return M.write("error", msg, meta) end

--- Read recent journal entries.
--- Requires daemon.journal_d.read permission (enforced by journal_read efun).
--- @param n      number  default 20
--- @param level  string|nil  optional level filter ("error", "warn", etc.)
--- @return array of raw JSON strings
function M.recent(n, level)
    if type(journal_read) ~= "function" then return {} end
    return journal_read(n or 20, level)
end

--- Format a journal entry (raw JSON string) for human-readable display.
--- Returns a single line: "[LEVEL] source: message"
function M.format_entry(json_str)
    -- Quick parse without a full JSON library
    local ts    = json_str:match('"ts"%s*:%s*"([^"]+)"')    or "?"
    local level = json_str:match('"level"%s*:%s*"([^"]+)"') or "?"
    local src   = json_str:match('"source"%s*:%s*"([^"]+)"') or ""
    local msg   = json_str:match('"msg"%s*:%s*"([^"]*)"')   or json_str

    level = level:upper():sub(1, 5)
    local when = ts:match("T(.-)Z") or ts  -- extract HH:MM:SS part
    if src ~= "" then
        return string.format("[%s] %s %s: %s", level, when, src, msg)
    else
        return string.format("[%s] %s %s", level, when, msg)
    end
end

log("info", "journal_d daemon loaded")

return M
