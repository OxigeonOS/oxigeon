-- mudlib/daemons/ticker_d.lua — Timer/scheduler daemon
-- Manages Lua-side callbacks for the Tokio-backed timer system.
-- The driver provides three efuns:
--   schedule_timer(id, delay_secs)      — one-shot timer
--   schedule_repeating(id, interval_secs) — repeating timer
--   cancel_timer(id)                     — cancel a timer
-- When a timer fires, the driver calls on_timer(id) which dispatches
-- to the callback registered here.
--
-- Usage:
--   DAEMON.ticker.after(10, "puzzle.reset", function() ... end)
--   DAEMON.ticker.every(15, "mob.guard.echo", function() ... end)
--   DAEMON.ticker.remove("mob.guard.echo")

local M = {}

-- ─── Internal state ──────────────────────────────────────────────────────────

-- Registered callbacks: id → { func = fn, once = bool }
M._callbacks = {}

-- ─── Helpers ─────────────────────────────────────────────────────────────────

--- Log an error to both log() and journal_d (if available).
local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        DAEMON.journal.error(message)
    end
end

-- ─── Public API ──────────────────────────────────────────────────────────────

--- Schedule a one-shot timer. The callback fires once after `delay` seconds.
-- @param delay number     Seconds to wait before firing
-- @param id string        Unique identifier for this timer
-- @param func function    Callback to invoke when the timer fires
function M.after(delay, id, func)
    if type(delay) ~= "number" or delay <= 0 then
        log("warn", "TICKER_D.after: invalid delay " .. tostring(delay)
            .. " for timer '" .. tostring(id) .. "'")
        return false
    end
    if type(id) ~= "string" or id == "" then
        log("warn", "TICKER_D.after: timer ID must be a non-empty string")
        return false
    end
    if type(func) ~= "function" then
        log("warn", "TICKER_D.after: callback must be a function for timer '"
            .. id .. "'")
        return false
    end

    -- Cancel any existing timer with this ID
    if M._callbacks[id] then
        M.remove(id)
    end

    M._callbacks[id] = { func = func, once = true }
    schedule_timer(id, delay)
    log("debug", "TICKER_D: Scheduled one-shot timer '" .. id
        .. "' in " .. delay .. "s")
    return true
end

--- Schedule a repeating timer. The callback fires every `interval` seconds.
-- @param interval number  Seconds between each firing
-- @param id string        Unique identifier for this timer
-- @param func function    Callback to invoke each time the timer fires
function M.every(interval, id, func)
    if type(interval) ~= "number" or interval <= 0 then
        log("warn", "TICKER_D.every: invalid interval " .. tostring(interval)
            .. " for timer '" .. tostring(id) .. "'")
        return false
    end
    if type(id) ~= "string" or id == "" then
        log("warn", "TICKER_D.every: timer ID must be a non-empty string")
        return false
    end
    if type(func) ~= "function" then
        log("warn", "TICKER_D.every: callback must be a function for timer '"
            .. id .. "'")
        return false
    end

    -- Cancel any existing timer with this ID
    if M._callbacks[id] then
        M.remove(id)
    end

    M._callbacks[id] = { func = func, once = false }
    schedule_repeating(id, interval)
    log("debug", "TICKER_D: Scheduled repeating timer '" .. id
        .. "' every " .. interval .. "s")
    return true
end

--- Cancel a timer by ID. Stops both the driver-side timer and removes the callback.
-- @param id string  The timer ID to cancel
-- @return boolean   true if a timer was found and cancelled
function M.remove(id)
    if not M._callbacks[id] then
        return false
    end
    M._callbacks[id] = nil
    local ok, err = pcall(cancel_timer, id)
    if not ok then
        log("warn", "TICKER_D: cancel_timer efun failed for '" .. id
            .. "': " .. tostring(err))
    end
    log("debug", "TICKER_D: Cancelled timer '" .. id .. "'")
    return true
end

--- Cancel every timer whose ID starts with `prefix`.
---
--- The cleanup half of the per-player convention: a timer scheduled for one
--- character is named `player.<char_id>.<what>`, and this is what disposes of
--- the lot when they log out. `character_d.unload` has called this since it was
--- written — the function simply did not exist, so the call raised into a pcall
--- that logged at debug level and every per-player timer leaked.
--- @param prefix string
--- @return number  how many were cancelled
function M.remove_by_prefix(prefix)
    if type(prefix) ~= "string" or #prefix == 0 then
        log("warn", "TICKER_D.remove_by_prefix: prefix must be a non-empty string")
        return 0
    end

    -- Collect first: removing while iterating `_callbacks` would be modifying
    -- a table mid-traversal.
    local doomed = {}
    for id in pairs(M._callbacks) do
        if id:sub(1, #prefix) == prefix then
            doomed[#doomed + 1] = id
        end
    end

    local n = 0
    for _, id in ipairs(doomed) do
        if M.remove(id) then n = n + 1 end
    end
    if n > 0 then
        log("debug", "TICKER_D: cancelled " .. n .. " timer(s) matching '" .. prefix .. "'")
    end
    return n
end

--- Called by the engine (via on_timer) when a timer fires.
-- Looks up the registered callback and executes it.
-- @param id string  The timer ID that fired
function M.fire(id)
    local entry = M._callbacks[id]
    if not entry then
        -- Timer fired but no callback registered (may have been removed)
        return
    end

    local ok, err = pcall(entry.func)
    if not ok then
        log_error("TICKER_D: Timer '" .. id .. "' callback failed: "
            .. tostring(err))
    end

    -- Clean up one-shot timers
    if entry.once then
        M._callbacks[id] = nil
    end
end

--- Check if a timer is currently registered.
-- @param id string  The timer ID
-- @return boolean
function M.is_active(id)
    return M._callbacks[id] ~= nil
end

--- List all active timer IDs.
-- @return table  Array of timer ID strings
function M.list()
    local ids = {}
    for id, _ in pairs(M._callbacks) do
        ids[#ids + 1] = id
    end
    return ids
end

--- Cancel all active timers. Used during shutdown or full reset.
function M.clear_all()
    for id, _ in pairs(M._callbacks) do
        pcall(cancel_timer, id)
    end
    M._callbacks = {}
    log("debug", "TICKER_D: All timers cleared")
end

log("info", "ticker_d daemon loaded")

return M
