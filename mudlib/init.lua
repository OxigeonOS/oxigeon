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
    login.cleanup(session_id)
end

--- Called when a GMCP message is received
function on_gmcp(session_id, package, data)
    log("debug", "GMCP from " .. session_id .. ": " .. package)
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
end
