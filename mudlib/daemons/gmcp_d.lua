local M = {}

local function log_error(msg)
    log("error", msg)
    if DAEMON and DAEMON.journal then
        local ok = pcall(function() DAEMON.journal.error(msg) end)
    end
end

function M.send_vitals(session_id)
    local ok, sess = pcall(function() return get_session(session_id) end)
    if not ok or not sess or not sess.gmcp_supported then return false end
    
    local pok, player = pcall(function() return get_player(session_id) end)
    if not pok or not player or not player.stats then return false end
    
    local data = {
        hp = player.stats.hp or 0,
        maxhp = player.stats.max_hp or 0,
        mp = player.stats.mp or 0,
        maxmp = player.stats.max_mp or 0
    }
    
    local send_ok, err = pcall(function() send_gmcp(session_id, "Char.Vitals", data) end)
    if not send_ok then log_error("gmcp_d send_vitals error: " .. tostring(err)) end
    return true
end

function M.send_room(session_id)
    local ok, sess = pcall(function() return get_session(session_id) end)
    if not ok or not sess or not sess.gmcp_supported then return false end
    
    if not DAEMON or not DAEMON.world then return false end
    
    local char_id = sess.character_id
    if not char_id then return false end
    
    local room = nil
    local room_ok, err = pcall(function()
        room = DAEMON.world.get_room(DAEMON.world.get_character_room(char_id))
    end)
    
    if not room_ok or not room then return false end
    
    local exits = {}
    if room.exits then
        for dir, _ in pairs(room.exits) do
            table.insert(exits, dir)
        end
    end
    
    local data = {
        id = room.id or "",
        name = room.name or "A room",
        area = room.area or "Unknown",
        exits = exits
    }
    
    local send_ok, err2 = pcall(function() send_gmcp(session_id, "Room.Info", data) end)
    if not send_ok then log_error("gmcp_d send_room error: " .. tostring(err2)) end
    return true
end

function M.send_status(session_id)
    local ok, sess = pcall(function() return get_session(session_id) end)
    if not ok or not sess or not sess.gmcp_supported then return false end
    
    local pok, player = pcall(function() return get_player(session_id) end)
    if not pok or not player or not player.stats then return false end
    
    local data = {
        level = player.stats.level or 1,
        xp = player.stats.xp or 0,
        gold = player.stats.gold or 0
    }
    
    local send_ok, err = pcall(function() send_gmcp(session_id, "Char.Status", data) end)
    if not send_ok then log_error("gmcp_d send_status error: " .. tostring(err)) end
    return true
end

function M.send_all(session_id)
    M.send_vitals(session_id)
    M.send_status(session_id)
    M.send_room(session_id)
end

log("info", "gmcp_d loaded")
return M
