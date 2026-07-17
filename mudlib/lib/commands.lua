-- mudlib/lib/commands.lua — Command dispatcher and registry
-- Provides lazy-loading of command modules from mudlib/cmds/<verb>.lua

local M = {}

-- registry maps canonical command name → module table
local _registry = {}
-- aliases maps alias string → canonical name
local _aliases  = {}

--- Register a command module into the registry and alias table
local function register(mod)
    _registry[mod.name] = mod
    for _, a in ipairs(mod.aliases or {}) do
        _aliases[a] = mod.name
    end
end

--- Lazy-load a command by verb name.
-- Tries require("cmds.<verb>"). On success, registers and returns the module.
-- Returns nil if the command doesn't exist or is malformed.
local function lazy_load(verb)
    if _registry[verb] then return _registry[verb] end
    local ok, mod = pcall(require, "cmds." .. verb)
    if ok and type(mod) == "table" and type(mod.execute) == "function" then
        register(mod)
        return mod
    end
    return nil
end

--- Parse a line of text into (verb, args_str, args_table).
-- verb     : lowercase first word
-- args_str : everything after the verb (raw, trimmed)
-- args     : whitespace-split tokens from args_str
-- Returns nil, "", {} for empty input.
function M.parse(text)
    local verb, rest = text:match("^(%S+)%s*(.*)")
    if not verb then return nil, "", {} end
    local args = {}
    for tok in rest:gmatch("%S+") do
        args[#args + 1] = tok
    end
    return verb:lower(), rest, args
end

--- Return a copy of the command registry (for help/introspection).
-- Keys are canonical command names, values are module tables.
function M.registry()
    return _registry
end

--- Dispatch a line of player input to the appropriate command.
-- session_id : the session that typed the input
-- text       : the raw input line (will be trimmed)
function M.dispatch(session_id, text)
    text = text:gsub("^%s+", ""):gsub("%s+$", "")

    if text == "" then
        send_prompt(session_id, "\r\n> ")
        return
    end

    local verb, args_str, args = M.parse(text)
    -- Resolve alias to canonical name
    verb = _aliases[verb] or verb

    local mod = lazy_load(verb)
    if not mod then
        send(session_id, "\r\nUnknown command: '" .. verb .. "'. Type 'help' for a list.\r\n")
        send_prompt(session_id, "> ")
        return
    end

    -- Permission check — has_permission is only available after Phase 2.
    -- Guard with a nil check so Phase 1 works without the efun present.
    if mod.permission and type(has_permission) == "function" then
        if not has_permission(session_id, mod.permission) then
            send(session_id, "\r\nYou don't have permission to do that.\r\n")
            send_prompt(session_id, "> ")
            -- Audit the denial
            if DAEMON and DAEMON.audit then
                DAEMON.audit.after_command(verb, session_id, args_str, false,
                    "permission denied: " .. mod.permission)
            end
            return
        end
    end

    local ok, err = pcall(mod.execute, session_id, args_str, args)
    if not ok then
        log("error", "Command '" .. verb .. "' error: " .. tostring(err))
        send(session_id, "\r\nAn error occurred processing that command.\r\n")
        send_prompt(session_id, "> ")
    end

    -- Notify audit daemon (it will check the watch list internally)
    if DAEMON and DAEMON.audit then
        DAEMON.audit.after_command(verb, session_id, args_str, ok, err)
    end
end

return M
