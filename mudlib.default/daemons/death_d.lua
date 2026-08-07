-- mudlib/daemons/death_d.lua — What happens when a character dies.
--
-- Where the dead reappear is a *game* decision, so it comes from
-- `game.respawn_room` in server.toml rather than from a constant in the mudlib.
-- It was `wizard_workshop.entrance` here for a long time — a driver-layer file
-- naming one specific game's room, which meant a second game could not use this
-- daemon without editing it. `game.start_room` is the fallback, because a game
-- that says where characters begin has already answered this question well
-- enough to boot.

local M = {}

--- The config key, with the two fallbacks, resolved fresh rather than cached:
--- a hot reload of this file should pick up an edited server.toml, and this is
--- read once per death rather than once per command.
local function configured_respawn_room()
    if type(config) ~= "function" then return nil end
    local ok, room = pcall(config, "game.respawn_room")
    if ok and type(room) == "string" and #room > 0 then return room end
    local sok, start = pcall(config, "game.start_room")
    if sok and type(start) == "string" and #start > 0 then return start end
    return nil
end

M._config = {
    -- nil means "ask the config". `configure{ respawn_room = ... }` still wins,
    -- so a game daemon can override it at runtime without touching a file.
    respawn_room = nil,
    respawn_delay = 5,
    hp_on_respawn = 0.25,
    xp_penalty = 0.0,
    drop_gold = false
}

--- Where a corpse gets up. Explicit override, then config, then the last-resort
--- literal — which is only reachable on a server that has configured no start
--- room at all, and is logged when it is used so it never becomes the answer by
--- accident.
--- @return string
function M.respawn_room()
    if type(M._config.respawn_room) == "string" and #M._config.respawn_room > 0 then
        return M._config.respawn_room
    end
    local configured = configured_respawn_room()
    if configured then return configured end

    -- No fallback room id. There used to be one — `wizard_workshop.entrance` —
    -- and it was a *mudlib* file naming a room in one particular game: a second
    -- game inherited it, silently, and only found out when somebody died.
    --
    -- Returning nil is the honest answer to "where does this game respawn
    -- people", and it is loud: the caller sends the player nowhere rather than
    -- to a room that may not exist.
    log("error", "DEATH_D: neither game.respawn_room nor game.start_room is set "
        .. "in server.toml; there is nowhere to respawn")
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.error, "DEATH_D: no respawn room configured — "
            .. "set game.respawn_room or game.start_room in server.toml")
    end
    return nil
end

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

    -- XP lives on the Player, not in stats. This read `player.stats.xp`,
    -- which has never existed, so the penalty was dead code.
    if M._config.xp_penalty > 0 and type(player.xp) == "number" then
        player.xp = math.max(0, math.floor(player.xp * (1.0 - M._config.xp_penalty)))
    end

    -- Death ends most effects. A definition can opt out with
    -- `survives_death = true` — a long curse should outlast dying.
    if DAEMON and DAEMON.effect then
        local eff_ok, eff_err = pcall(DAEMON.effect.clear, player,
            { keep_survivors = true, reason = "death" })
        if not eff_ok then log_error("death_d effect clear error: " .. tostring(eff_err)) end
    end

    if DAEMON and DAEMON.ticker then
        local timer_ok, timer_err = pcall(function()
            DAEMON.ticker.after(M._config.respawn_delay, "player." .. char_id .. ".respawn", function()
                M.handle_respawn(char_id, session_id)
            end)
        end)
        if not timer_ok then log_error("death_d timer error: " .. tostring(timer_err)) end
    end
end

function M.handle_respawn(char_id, session_id)
    local ok, player = pcall(function() return get_player(session_id) end)
    if not ok or not player then return end

    -- Through TRAIT_D rather than writing player.stats.hp directly: a raw
    -- write would leave the regeneration anchor pointing at the moment before
    -- they died, and they would be handed all the intervening seconds of
    -- healing the instant they respawned.
    if DAEMON and DAEMON.trait and DAEMON.trait.get_def and DAEMON.trait.get_def("hp") then
        local max_hp = player:trait("max_hp")
        DAEMON.trait.set_cur(player, "hp", math.floor(max_hp * M._config.hp_on_respawn))
    elseif player.stats and player.stats.max_hp then
        player.stats.hp = math.floor(player.stats.max_hp * M._config.hp_on_respawn)
    end

    local room = M.respawn_room()
    if room then
        local move_ok, move_err = pcall(function() player:move_to(room) end)
        if not move_ok then log_error("death_d move error: " .. tostring(move_err)) end
    else
        -- `respawn_room` has already said why, loudly. Leaving them where they
        -- fell is wrong, but it is visibly wrong, which a guess at another
        -- game's room id is not.
        log_error("death_d: nowhere to respawn char " .. tostring(player.char_id))
    end

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
