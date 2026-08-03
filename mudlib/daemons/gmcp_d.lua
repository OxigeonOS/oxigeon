-- mudlib/daemons/gmcp_d.lua — What the client is told, and what it may say back.
--
-- Outbound has always worked: `Char.Vitals`, `Char.Status`, `Char.Effects` and
-- `Room.Info` are pushed on the events that change them. **Inbound did not** —
-- `on_gmcp` logged the package name and returned, so a client could negotiate
-- GMCP, announce what it supports and send `Core.Hello`, and the game never
-- read a word of it.
--
-- This is the inbound half. Packages are dispatched by name to handlers a game
-- can add to, because which custom packages exist is content: `Game.Quest` is
-- this game's, and a different game wants a different one rather than a
-- configuration option.
--
-- ─── Why a table rather than an if-chain ─────────────────────────────────────
--
-- A game adding `Game.Craft` should not have to edit a mudlib file. Registering
-- a handler is one line and the dispatcher never changes, which is the same
-- reason `COMPUTE_HANDLERS` is a list.

local M = {}

--- package name -> function(session_id, data). Lowercased on both sides,
--- because clients disagree about capitalisation and the spec does not care.
M._handlers = {}

--- What each session said it supports, from `Core.Supports.Set`. Memory tier:
--- it arrives again on the next connection, and a client that reconnects is a
--- client whose support list may have changed anyway.
M._supports = {}

local function log_error(msg)
    log("error", msg)
    if DAEMON and DAEMON.journal then
        local ok = pcall(function() DAEMON.journal.error(msg) end)
    end
end

function M.send_vitals(session_id)
    local ok, sess = pcall(function() return get_session(session_id) end)
    if not ok or not sess or not sess.gmcp_supported then return false end
    -- A client that never asked for this module should not be sent it. Before
    -- `Core.Supports.Set` was read, everyone got everything.
    if not M.wants(session_id, "Char.Vitals") then return false end
    
    local pok, player = pcall(function() return get_player(session_id) end)
    if not pok or not player or not player.stats then return false end
    
    -- Through :trait() rather than the raw table, so a client's health bar
    -- reflects buffs and regeneration the same way the prompt does.
    local data = {
        hp = player:trait("hp"),
        maxhp = player:trait("max_hp"),
        mp = player:trait("mp"),
        maxmp = player:trait("max_mp")
    }
    
    local send_ok, err = pcall(function() send_gmcp(session_id, "Char.Vitals", data) end)
    if not send_ok then log_error("gmcp_d send_vitals error: " .. tostring(err)) end
    return true
end

function M.send_room(session_id)
    local ok, sess = pcall(function() return get_session(session_id) end)
    if not ok or not sess or not sess.gmcp_supported then return false end
    -- A client that never asked for this module should not be sent it. Before
    -- `Core.Supports.Set` was read, everyone got everything.
    if not M.wants(session_id, "Room.Info") then return false end
    
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
    -- A client that never asked for this module should not be sent it. Before
    -- `Core.Supports.Set` was read, everyone got everything.
    if not M.wants(session_id, "Char.Status") then return false end
    
    local pok, player = pcall(function() return get_player(session_id) end)
    if not pok or not player or not player.stats then return false end
    
    -- xp and gold live on the Player, not in stats. Reading them from
    -- `player.stats` meant this reported 0 for every character, always.
    local data = {
        level = player:trait("level"),
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
    -- A client that never asked for this module should not be sent it. Before
    -- `Core.Supports.Set` was read, everyone got everything.
    if not M.wants(session_id, "Char.Effects") then return false end
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

-- ─── Inbound ─────────────────────────────────────────────────────────────────

--- Register a handler for one package.
--- @param package string    e.g. "Game.Quest"
--- @param fn function       function(session_id, data)
--- @return boolean
function M.on(package, fn)
    if type(package) ~= "string" or type(fn) ~= "function" then
        log_error("GMCP_D.on: needs a package name and a function")
        return false
    end
    M._handlers[package:lower()] = fn
    return true
end

--- What this session told us it supports.
--- @param session_id string
--- @return table  package name -> version
function M.supports(session_id)
    return M._supports[session_id] or {}
end

--- Does the client want this package? A client that did not ask for
--- `Char.Effects` should not be sent forty of them a minute.
--- @return boolean
function M.wants(session_id, package)
    local list = M._supports[session_id]
    -- No `Core.Supports.Set` at all means an older client that negotiated GMCP
    -- and said nothing else. Sending it everything is the friendlier guess, and
    -- it is what the game did before any of this existed.
    if not list or next(list) == nil then return true end

    -- `Char.Vitals` is covered by a client supporting `Char`, which is how the
    -- convention works: modules are announced, not individual packages.
    local module = package:match("^([^%.]+)")
    return list[package:lower()] ~= nil or list[(module or ""):lower()] ~= nil
end

--- Dispatch one inbound message. Called from `on_gmcp`.
--- @param session_id string
--- @param package string
--- @param data any
--- @return boolean  whether anything handled it
function M.receive(session_id, package, data)
    if type(package) ~= "string" then return false end

    local handler = M._handlers[package:lower()]
    if not handler then
        -- Logged rather than ignored: a package nobody handles is either a
        -- client feature nobody has written yet or a typo in a handler
        -- registration, and both are worth being able to see.
        log("debug", "GMCP: unhandled package '" .. package .. "' from " .. tostring(session_id))
        return false
    end

    local ok, err = pcall(handler, session_id, data)
    if not ok then
        log_error("GMCP_D: handler for '" .. package .. "' raised: " .. tostring(err))
        return false
    end
    return true
end

--- Forget a session's support list. From `on_disconnect`, for the reason every
--- other per-session table is cleaned up there: session ids are not reused and
--- a table keyed on them grows forever.
function M.forget(session_id)
    M._supports[session_id] = nil
end

-- ─── The standard packages ───────────────────────────────────────────────────

--- `Core.Supports.Set` — an array of "Module version" strings.
---
--- The one inbound package every GMCP client sends, and the one the game had
--- never read. Knowing what a client supports is what makes it reasonable to
--- push `Char.Effects` on every effect change: to a client that did not ask,
--- that is forty messages a minute it will throw away.
M.on("Core.Supports.Set", function(session_id, data)
    if type(data) ~= "table" then return end

    local list = {}
    for _, entry in ipairs(data) do
        if type(entry) == "string" then
            -- "Char 1" -> char = 1. The version is kept because a client
            -- announcing version 2 of a package may expect a different shape,
            -- and throwing it away now would mean re-negotiating later.
            local name, version = entry:match("^(%S+)%s*(%d*)$")
            if name then list[name:lower()] = tonumber(version) or 1 end
        end
    end
    M._supports[session_id] = list

    log("debug", "GMCP: " .. tostring(session_id) .. " supports "
        .. tostring(#data) .. " module(s)")

    -- Answering immediately is the point: a client that has just said what it
    -- can draw should get something to draw.
    M.send_all(session_id)
end)

--- `Core.Supports.Add` / `.Remove` — the same list, edited.
M.on("Core.Supports.Add", function(session_id, data)
    if type(data) ~= "table" then return end
    local list = M._supports[session_id] or {}
    for _, entry in ipairs(data) do
        if type(entry) == "string" then
            local name, version = entry:match("^(%S+)%s*(%d*)$")
            if name then list[name:lower()] = tonumber(version) or 1 end
        end
    end
    M._supports[session_id] = list
end)

M.on("Core.Supports.Remove", function(session_id, data)
    if type(data) ~= "table" then return end
    local list = M._supports[session_id]
    if not list then return end
    for _, entry in ipairs(data) do
        if type(entry) == "string" then
            local name = entry:match("^(%S+)")
            if name then list[name:lower()] = nil end
        end
    end
end)

--- `Core.Hello` — the client naming itself. Worth keeping for the same reason
--- `terminal_type` is: when somebody reports a rendering bug, the first
--- question is what they were using.
M.on("Core.Hello", function(session_id, data)
    if type(data) ~= "table" then return end
    local client = tostring(data.client or "?") .. " " .. tostring(data.version or "?")
    log("info", "GMCP: " .. tostring(session_id) .. " is " .. client)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.info, "GMCP client: " .. client)
    end
end)

--- `Core.Ping` — answered, because a client that pings and hears nothing will
--- conclude the connection is dead.
M.on("Core.Ping", function(session_id, data)
    pcall(send_gmcp, session_id, "Core.Ping", data)
end)

log("info", "gmcp_d loaded")
return M
