-- game/daemons/olc_d.lua — OLC session manager daemon
-- Tracks which sessions are in OLC (building) mode and for which area.
-- State is transient — not persisted across restarts.

local M = {}

-- session_id → { area_name = "...", entered_at = N }
M._sessions = {}

--- Enter OLC mode for an area.
-- @param session_id string
-- @param area_name string
-- @return boolean
function M.start(session_id, area_name)
    M._sessions[session_id] = {
        area_name = area_name,
        entered_at = os_time(),
    }
    log("info", "OLC_D: Session " .. tostring(session_id)
        .. " entered OLC mode for area '" .. area_name .. "'")
    return true
end

--- Exit OLC mode.
-- @param session_id string
-- @return boolean
function M.stop(session_id)
    if M._sessions[session_id] then
        local area = M._sessions[session_id].area_name
        M._sessions[session_id] = nil
        log("info", "OLC_D: Session " .. tostring(session_id)
            .. " exited OLC mode for area '" .. tostring(area) .. "'")
    end
    return true
end

--- Get the OLC state for a session.
-- @param session_id string
-- @return table|nil  { area_name, entered_at } or nil
function M.get_state(session_id)
    return M._sessions[session_id]
end

--- Check if a session is in OLC mode.
-- @param session_id string
-- @return boolean
function M.is_active(session_id)
    return M._sessions[session_id] ~= nil
end

--- Cleanup on disconnect. Called from on_disconnect handler.
-- @param session_id string
function M.cleanup(session_id)
    if M._sessions[session_id] then
        M.stop(session_id)
    end
end

log("debug", "OLC_D: daemon loaded")

return M
