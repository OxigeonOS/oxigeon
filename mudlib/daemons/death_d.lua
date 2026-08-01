local M = {}

M._config = {
    respawn_room = "wizard_workshop.entrance",
    respawn_delay = 5,
    hp_on_respawn = 0.25,
    xp_penalty = 0.0,
    drop_gold = false
}

local function log_error(msg)
    log("error", msg)
    if DAEMON and DAEMON.journal then
        local ok = pcall(function() DAEMON.journal.error(msg) end)
    end
end

function M.configure(overrides)
    if not overrides then return end
    for k, v in pairs(overrides) do
        M._config[k] = v
    end
end

function M.handle_death(data)
    local char_id = data.char_id
    local session_id = data.session_id

    local ok, player = pcall(function() return get_player(session_id) end)
    if not ok or not player then return end

    local send_ok = pcall(function()
        player:send_lines({"You have died.", "Respawning in " .. M._config.respawn_delay .. " seconds..."})
        player:message_room(player.name .. " has died.")
    end)
    if not send_ok then log_error("death_d send messages error") end

    if M._config.xp_penalty > 0 and player.stats and player.stats.xp then
        player.stats.xp = math.max(0, math.floor(player.stats.xp * (1.0 - M._config.xp_penalty)))
    end

    if DAEMON and DAEMON.ticker then
        local timer_ok, timer_err = pcall(function()
            DAEMON.ticker.after(M._config.respawn_delay, "respawn_" .. char_id, function()
                M.handle_respawn(char_id, session_id)
            end)
        end)
        if not timer_ok then log_error("death_d timer error: " .. tostring(timer_err)) end
    end
end

function M.handle_respawn(char_id, session_id)
    local ok, player = pcall(function() return get_player(session_id) end)
    if not ok or not player then return end

    if player.stats and player.stats.max_hp then
        player.stats.hp = math.floor(player.stats.max_hp * M._config.hp_on_respawn)
    end

    local move_ok, move_err = pcall(function()
        player:move_to(M._config.respawn_room)
    end)
    if not move_ok then log_error("death_d move error: " .. tostring(move_err)) end

    local send_ok = pcall(function()
        player:send("You have respawned.\r\n")
        player:message_room(player.name .. " appears in a flash of light.")
    end)

    if DAEMON and DAEMON.event then
        local ev_ok, ev_err = pcall(function()
            DAEMON.event.emit("player.respawn", {char_id = char_id, session_id = session_id})
        end)
        if not ev_ok then log_error("death_d emit error: " .. tostring(ev_err)) end
    end
end

if DAEMON and DAEMON.event then
    local ok, err = pcall(function()
        DAEMON.event.on("player.death", "death_d.handler", function(data) M.handle_death(data) end)
    end)
    if not ok then log_error("death_d init error: " .. tostring(err)) end
end

log("info", "death_d loaded")
return M
