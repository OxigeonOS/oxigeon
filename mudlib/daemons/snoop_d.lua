local M = {}

local _snoops = {}
local _reverse = {}

local function log_error(msg)
    log("error", msg)
    if DAEMON and DAEMON.journal then
        local ok = pcall(function() DAEMON.journal.error(msg) end)
    end
end

function M.start(snooper_sid, target_sid)
    if snooper_sid == target_sid then
        return false, "You cannot snoop yourself."
    end
    
    if _snoops[target_sid] == snooper_sid then
        return false, "Cannot chain snoops (they are snooping you)."
    end
    
    _snoops[snooper_sid] = target_sid
    
    if not _reverse[target_sid] then
        _reverse[target_sid] = {}
    end
    table.insert(_reverse[target_sid], snooper_sid)
    
    return true
end

function M.stop(snooper_sid)
    local target_sid = _snoops[snooper_sid]
    if not target_sid then return false end
    
    _snoops[snooper_sid] = nil
    
    if _reverse[target_sid] then
        for i, sid in ipairs(_reverse[target_sid]) do
            if sid == snooper_sid then
                table.remove(_reverse[target_sid], i)
                break
            end
        end
        if #_reverse[target_sid] == 0 then
            _reverse[target_sid] = nil
        end
    end
    
    return true
end

function M.get_target(snooper_sid)
    return _snoops[snooper_sid]
end

function M.get_snoopers(target_sid)
    return _reverse[target_sid] or {}
end

function M.is_snooped(target_sid)
    return _reverse[target_sid] ~= nil and #_reverse[target_sid] > 0
end

function M.cleanup(session_id)
    M.stop(session_id)
    
    if _reverse[session_id] then
        local snoopers = {}
        for _, snooper_sid in ipairs(_reverse[session_id]) do
            table.insert(snoopers, snooper_sid)
        end
        for _, snooper_sid in ipairs(snoopers) do
            M.stop(snooper_sid)
        end
    end
end

log("info", "snoop_d loaded")
return M
