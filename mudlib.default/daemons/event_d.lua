-- mudlib/daemons/event_d.lua — Signal/event system daemon
-- Provides Godot-style signals: named event channels with dynamic subscribe/unsubscribe.
-- All handlers fire synchronously on emit. Each handler is pcall-wrapped so a
-- failing listener never breaks the chain.
--
-- Usage:
--   DAEMON.event.on("mob.died", "guard.enrage", function(data) ... end)
--   DAEMON.event.emit("mob.died", { mob_id = "mob.guard_1", room_id = "town.gate" })
--   DAEMON.event.off("mob.died", "guard.enrage")

local M = {}

-- ─── Internal state ──────────────────────────────────────────────────────────

-- Registered listeners: event_name → { listener_id → { func, priority } }
local _listeners = {}

-- Sorted cache: event_name → sorted array of { id, func, priority }
-- Invalidated when listeners for an event change.
local _sorted_cache = {}

-- ─── Helpers ─────────────────────────────────────────────────────────────────

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        DAEMON.journal.error(message)
    end
end

--- Invalidate the sorted cache for an event so it's rebuilt on next emit.
local function invalidate_cache(event)
    _sorted_cache[event] = nil
end

--- Build the sorted listener list for an event (sorted by priority, lower first).
local function get_sorted_listeners(event)
    if _sorted_cache[event] then
        return _sorted_cache[event]
    end

    local entries = _listeners[event]
    if not entries then
        return {}
    end

    local sorted = {}
    for id, entry in pairs(entries) do
        sorted[#sorted + 1] = { id = id, func = entry.func, priority = entry.priority }
    end

    table.sort(sorted, function(a, b)
        if a.priority ~= b.priority then
            return a.priority < b.priority
        end
        return a.id < b.id  -- stable sort by ID for same priority
    end)

    _sorted_cache[event] = sorted
    return sorted
end

-- ─── Public API ──────────────────────────────────────────────────────────────

--- Subscribe to an event.
-- @param event string       The event name (e.g. "mob.died", "area.dungeon.alarm")
-- @param listener_id string Unique ID for this listener (used for unsubscribe)
-- @param callback function  Called with (data) when the event fires
-- @param priority number    Optional priority (default 0). Lower fires first.
-- @return boolean           true if registered successfully
function M.on(event, listener_id, callback, priority)
    if type(event) ~= "string" or event == "" then
        log("warn", "EVENT_D.on: event name must be a non-empty string")
        return false
    end
    if type(listener_id) ~= "string" or listener_id == "" then
        log("warn", "EVENT_D.on: listener ID must be a non-empty string")
        return false
    end
    if type(callback) ~= "function" then
        log("warn", "EVENT_D.on: callback must be a function for listener '"
            .. listener_id .. "' on event '" .. event .. "'")
        return false
    end

    priority = priority or 0

    if not _listeners[event] then
        _listeners[event] = {}
    end

    _listeners[event][listener_id] = {
        func     = callback,
        priority = priority,
    }

    invalidate_cache(event)
    log("debug", "EVENT_D: '" .. listener_id .. "' subscribed to '" .. event
        .. "' (priority " .. priority .. ")")
    return true
end

--- Unsubscribe a specific listener from an event.
-- @param event string       The event name
-- @param listener_id string The listener to remove
-- @return boolean           true if the listener was found and removed
function M.off(event, listener_id)
    if not _listeners[event] then
        return false
    end
    if not _listeners[event][listener_id] then
        return false
    end

    _listeners[event][listener_id] = nil
    invalidate_cache(event)

    -- Clean up empty event tables
    if not next(_listeners[event]) then
        _listeners[event] = nil
    end

    log("debug", "EVENT_D: '" .. listener_id .. "' unsubscribed from '" .. event .. "'")
    return true
end

--- Remove all listeners for a specific event.
-- Useful for area unloads or system resets.
-- @param event string  The event name to clear
-- @return number       Number of listeners removed
function M.off_all(event)
    local entries = _listeners[event]
    if not entries then
        return 0
    end

    local count = 0
    for _ in pairs(entries) do
        count = count + 1
    end

    _listeners[event] = nil
    invalidate_cache(event)

    log("debug", "EVENT_D: Cleared " .. count .. " listener(s) from '" .. event .. "'")
    return count
end

--- Remove all listeners whose ID starts with a given prefix.
-- Useful for cleanup when a mob dies, an area unloads, etc.
-- @param prefix string  The listener ID prefix to match
-- @return number        Number of listeners removed
function M.off_by_prefix(prefix)
    if type(prefix) ~= "string" or prefix == "" then
        return 0
    end

    local count = 0
    local prefix_len = #prefix

    for event, entries in pairs(_listeners) do
        local removed_any = false
        for listener_id, _ in pairs(entries) do
            if listener_id:sub(1, prefix_len) == prefix then
                entries[listener_id] = nil
                removed_any = true
                count = count + 1
            end
        end
        if removed_any then
            invalidate_cache(event)
            -- Clean up empty event tables
            if not next(entries) then
                _listeners[event] = nil
            end
        end
    end

    if count > 0 then
        log("debug", "EVENT_D: Removed " .. count .. " listener(s) with prefix '"
            .. prefix .. "'")
    end
    return count
end

--- Emit an event, calling all registered listeners in priority order.
-- Each listener is pcall-wrapped: a failure in one never stops the others.
-- @param event string   The event name to emit
-- @param data table     Context data passed to each listener callback
-- @return number        Number of listeners that were called
function M.emit(event, data)
    local sorted = get_sorted_listeners(event)
    if #sorted == 0 then
        return 0
    end

    local count = 0
    for _, entry in ipairs(sorted) do
        local ok, err = pcall(entry.func, data)
        if not ok then
            log_error("EVENT_D: Listener '" .. entry.id .. "' failed on event '"
                .. event .. "': " .. tostring(err))
        end
        count = count + 1
    end

    return count
end

--- Emit an event after a delay, using DAEMON.ticker.
-- Avoids re-entrancy issues (e.g., a death handler emitting another death event).
-- @param event string   The event name
-- @param data table     Context data
-- @param delay number   Seconds to wait (default 0.01 — effectively "next tick")
function M.defer(event, data, delay)
    delay = delay or 0.01
    local timer_id = "event.deferred." .. event .. "." .. tostring(time())
    if DAEMON and DAEMON.ticker then
        DAEMON.ticker.after(delay, timer_id, function()
            M.emit(event, data)
        end)
    else
        -- Fallback: emit immediately if ticker isn't loaded
        log("warn", "EVENT_D.defer: DAEMON.ticker not available, emitting immediately")
        M.emit(event, data)
    end
end

-- ─── Introspection ───────────────────────────────────────────────────────────

--- Check if any listeners are registered for an event.
-- @param event string  The event name
-- @return boolean
function M.has_listeners(event)
    return _listeners[event] ~= nil and next(_listeners[event]) ~= nil
end

--- Count listeners for a specific event.
-- @param event string  The event name
-- @return number
function M.count(event)
    if not _listeners[event] then
        return 0
    end
    local n = 0
    for _ in pairs(_listeners[event]) do
        n = n + 1
    end
    return n
end

--- List all listener IDs for a specific event.
-- @param event string  The event name
-- @return table        Array of listener ID strings
function M.listeners(event)
    local ids = {}
    if _listeners[event] then
        for id, _ in pairs(_listeners[event]) do
            ids[#ids + 1] = id
        end
    end
    return ids
end

--- List all event names that have at least one listener.
-- @return table  Array of event name strings
function M.events()
    local names = {}
    for event, entries in pairs(_listeners) do
        if next(entries) then
            names[#names + 1] = event
        end
    end
    return names
end

--- Clear all listeners for all events. Use for full system reset.
function M.clear_all()
    _listeners = {}
    _sorted_cache = {}
    log("debug", "EVENT_D: All events and listeners cleared")
end

log("info", "event_d daemon loaded")

return M
