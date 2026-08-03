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
    
    -- Through :stat() rather than the raw table, so a client's health bar
    -- reflects buffs and regeneration the same way the prompt does.
    local data = {
        hp = player:stat("hp"),
        maxhp = player:stat("max_hp"),
        mp = player:stat("mp"),
        maxmp = player:stat("max_mp")
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
    
    -- xp and gold live on the Player, not in stats. Reading them from
    -- `player.stats` meant this reported 0 for every character, always.
    local data = {
        level = player:stat("level"),
        xp = player.xp or 0,
        gold = player.gold or 0
    }
    
    local send_ok, err = pcall(function() send_gmcp(session_id, "Char.Status", data) end)
    if not send_ok then log_error("gmcp_d send_status error: " .. tostring(err)) end
    return true
end

--- Everything currently affecting the character, for a client that wants to
--- draw buff icons.
function M.send_effects(session_id)
    local ok, sess = pcall(function() return get_session(session_id) end)
    if not ok or not sess or not sess.gmcp_supported then return false end
    if not (DAEMON and DAEMON.effect) then return false end

    local pok, player = pcall(function() return get_player(session_id) end)
    if not pok or not player then return false end

    local now = os_time()
    local list = {}
    local aok, active = pcall(DAEMON.effect.active, player)
    if not aok then return false end
    for _, e in ipairs(active) do
        list[#list + 1] = {
            id = e.inst.def,
            label = e.def.label or e.inst.def,
            remaining = e.inst.expires and math.max(0, math.floor(e.inst.expires - now)) or -1,
            stacks = e.inst.stacks or 1,
        }
    end

    local send_ok, err = pcall(function() send_gmcp(session_id, "Char.Effects", list) end)
    if not send_ok then log_error("gmcp_d send_effects error: " .. tostring(err)) end
    return true
end

function M.send_all(session_id)
    M.send_vitals(session_id)
    M.send_status(session_id)
    M.send_effects(session_id)
    M.send_room(session_id)
end

log("info", "gmcp_d loaded")
return M
