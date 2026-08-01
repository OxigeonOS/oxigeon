-- mudlib/init.lua — Oxigeon Mudlib Entry Point
-- Loaded by the Oxigeon driver at startup.
-- Defines on_connect, on_input, on_disconnect, on_gmcp, on_load, on_unload globally.

-- ─── Daemon registry ─────────────────────────────────────────────────────────
-- DAEMON is a global table; all daemons attach themselves here on load.
DAEMON = {}

-- Load core daemons (order matters: auditd and journald are foundational)
local ok, err

ok, err = pcall(function() DAEMON.journal = require("daemons.journald") end)
if not ok then log("warn", "Failed to load journald daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.audit   = require("daemons.auditd") end)
if not ok then log("warn", "Failed to load auditd daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.ticker  = require("daemons.ticker_d") end)
if not ok then log("warn", "Failed to load ticker_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.event   = require("daemons.event_d") end)
if not ok then log("warn", "Failed to load event_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.prompt  = require("daemons.prompt_d") end)
if not ok then log("warn", "Failed to load prompt_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.channel = require("daemons.channel_d") end)
if not ok then log("warn", "Failed to load channel_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.death   = require("daemons.death_d") end)
if not ok then log("warn", "Failed to load death_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.task    = require("daemons.task_d") end)
if not ok then log("warn", "Failed to load task_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.gmcp    = require("daemons.gmcp_d") end)
if not ok then log("warn", "Failed to load gmcp_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.pager   = require("daemons.pager_d") end)
if not ok then log("warn", "Failed to load pager_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.snoop   = require("daemons.snoop_d") end)
if not ok then log("warn", "Failed to load snoop_d daemon: " .. tostring(err)) end

-- ─── Global utility functions ────────────────────────────────────────────────
-- These are available to all Lua code (mudlib and game) without require().

--- Get the Player object for a session.
-- Wraps the session → character → Player lookup so area/game code never
-- needs to know about session infrastructure.
-- @param session_id string
-- @return Player|nil  The Player object, or nil if not logged in
function get_player(session_id)
    local session = get_session(session_id)
    if not session or not session.character_id then return nil end
    if DAEMON and DAEMON.character then
        return DAEMON.character.get(session.character_id)
    end
    return nil
end

-- ─── Command dispatcher ──────────────────────────────────────────────────────
local login    = require("login")
local commands = require("lib.commands")

--- Called when a new client connects (before authentication)
function on_connect(session_id)
    log("debug", "New connection: " .. session_id)
    set_session_state(session_id, "authenticating")
    login.greet(session_id)
end

--- Called when a player types a line of input
function on_input(session_id, text)
    local session = get_session(session_id)
    if not session then return end

    if session.state == "authenticating" then
        login.handle_input(session_id, text)
    elseif session.state == "playing" then
        commands.dispatch(session_id, text)
    end
end

--- Called when a client disconnects
function on_disconnect(session_id)
    log("debug", "Disconnected: " .. session_id)

    -- Save and unload character data, then remove from world.
    -- Each step is individually protected so a failure in one doesn't
    -- prevent cleanup in subsequent steps.
    local session = get_session(session_id)
    if session and session.character_id then
        local char_id = session.character_id

        -- Save persisted character data before cleanup
        if DAEMON and DAEMON.character then
            local ok, err = pcall(DAEMON.character.unload, char_id)
            if not ok then
                log("error", "Failed to unload character data for "
                    .. tostring(char_id) .. ": " .. tostring(err))
                if DAEMON.journal then
                    DAEMON.journal.error("CHARACTER_D unload failed on disconnect for char "
                        .. tostring(char_id) .. ": " .. tostring(err))
                end
            end
        end

        -- Remove character from the world
        if DAEMON and DAEMON.world then
            local ok, err = pcall(DAEMON.world.remove_character, char_id)
            if not ok then
                log("error", "Failed to remove character "
                    .. tostring(char_id) .. " from world: " .. tostring(err))
            end
        end
    end

    -- Clean up OLC session if active
    if DAEMON and DAEMON.olc then
        local ok, err = pcall(DAEMON.olc.cleanup, session_id)
        if not ok then
            log("error", "Failed to cleanup OLC session: " .. tostring(err))
        end
    end

    -- Clean up snoop relationships
    if DAEMON and DAEMON.snoop then
        local ok, err = pcall(DAEMON.snoop.cleanup, session_id)
        if not ok then
            log("error", "Failed to cleanup snoop session: " .. tostring(err))
        end
    end

    -- Clean up pager state
    if DAEMON and DAEMON.pager then
        local ok, err = pcall(DAEMON.pager.stop, session_id)
        if not ok then
            log("error", "Failed to cleanup pager session: " .. tostring(err))
        end
    end

    login.cleanup(session_id)
end

--- Called when a GMCP message is received
function on_gmcp(session_id, package, data)
    log("debug", "GMCP from " .. session_id .. ": " .. package)
end

--- Called when a driver-side timer fires
function on_timer(id)
    if DAEMON and DAEMON.ticker then
        DAEMON.ticker.fire(id)
    end
end

--- Called before a module is hot-reloaded
function on_unload(module_name)
    log("info", "Unloading module: " .. module_name)
    if DAEMON.journal then
        DAEMON.journal.info("Module unloading: " .. module_name)
    end
end

--- Called after a module is hot-reloaded
function on_load(module_name)
    log("info", "Loaded module: " .. module_name)
    if DAEMON.journal then
        DAEMON.journal.info("Module reloaded: " .. module_name)
    end

    -- Re-bind DAEMON references so they point to the newly loaded module.
    -- The hot-reload system updates package.loaded, but DAEMON.x still
    -- points to the old table unless we reassign here.
    local daemon_map = {
        ["daemons.journald"]     = "journal",
        ["daemons.auditd"]       = "audit",
        ["daemons.ticker_d"]     = "ticker",
        ["daemons.event_d"]      = "event",
        ["daemons.prompt_d"]     = "prompt",
        ["daemons.channel_d"]    = "channel",
        ["daemons.death_d"]      = "death",
        ["daemons.task_d"]       = "task",
        ["daemons.gmcp_d"]       = "gmcp",
        ["daemons.pager_d"]      = "pager",
        ["daemons.snoop_d"]      = "snoop",
        ["daemons.room_d"]       = "room",
        ["daemons.character_d"]  = "character",
        ["daemons.world_d"]      = "world",
        ["daemons.codegen_d"]    = "codegen",
        ["daemons.olc_d"]        = "olc",
        ["daemons.item_d"]       = "items",
    }

    -- Convert slash-separated path to dot-separated require path
    local require_path = module_name:gsub("/", "."):gsub("\\", ".")
    local key = daemon_map[require_path]
    if key then
        local loaded = package.loaded[require_path]
        if loaded then
            DAEMON[key] = loaded
            log("info", "Re-bound DAEMON." .. key .. " after reload")
        end
    end

    -- Flush command cache so reloaded command modules are picked up
    local commands = package.loaded["lib.commands"]
    if commands and commands.flush_cache then
        commands.flush_cache()
    end
end
