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

--- Get the list of command search path prefixes from config.
-- Cached after first call. Defaults to {"cmds"}.
local _cmd_prefixes = nil
local function get_cmd_prefixes()
    if _cmd_prefixes then return _cmd_prefixes end
    local ok, paths = pcall(config, "game.command_paths")
    if ok and type(paths) == "table" then
        _cmd_prefixes = {}
        for _, p in ipairs(paths) do
            -- Convert slashes to dots for require() module names
            _cmd_prefixes[#_cmd_prefixes + 1] = p:gsub("/", ".")
        end
    else
        _cmd_prefixes = { "cmds" }
    end
    return _cmd_prefixes
end

--- Eagerly load ALL command modules from configured paths.
-- Called once on first dispatch (or after flush_cache).
-- Uses list_dir() efun to discover .lua files.
local _loaded_all = false
local function load_all_commands()
    if _loaded_all then return end
    _loaded_all = true
    for _, prefix in ipairs(get_cmd_prefixes()) do
        -- Convert dot prefix back to slash path for list_dir
        local dir_path = prefix:gsub("%.", "/")
        if type(list_dir) == "function" then
            local ok, files = pcall(list_dir, dir_path)
            if ok and type(files) == "table" then
                for _, name in ipairs(files) do
                    if not _registry[name] then
                        local rok, mod = pcall(require, prefix .. "." .. name)
                        if rok and type(mod) == "table" and type(mod.execute) == "function" then
                            register(mod)
                        end
                    end
                end
            end
        end
    end
end

--- Look up a command by verb name.
-- On first call, eagerly loads all commands so aliases are available.
-- Returns nil if the command doesn't exist.
local function lazy_load(verb)
    if not _loaded_all then load_all_commands() end
    -- Check canonical name first, then aliases
    if _registry[verb] then return _registry[verb] end
    local canonical = _aliases[verb]
    if canonical and _registry[canonical] then return _registry[canonical] end
    return nil
end

--- Send the prompt to a session.
-- Uses DAEMON.prompt if available, otherwise falls back to "> ".
local function render_prompt(session_id)
    if DAEMON and DAEMON.prompt then
        DAEMON.prompt.render(session_id)
    else
        send_prompt(session_id, "> ")
    end
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

--- Flush the command cache so reloaded modules are picked up.
-- Called by on_load when a command module is hot-reloaded.
function M.flush_cache()
    _registry = {}
    _aliases  = {}
    _cmd_prefixes = nil
    _loaded_all = false
end

--- Dispatch a line of player input to the appropriate command.
-- Resolution order: room actions → system commands.
-- Future: guild actions → item actions → room actions → system commands.
-- session_id : the session that typed the input
-- text       : the raw input line (will be trimmed)
function M.dispatch(session_id, text)
    text = text:gsub("^%s+", ""):gsub("%s+$", "")

    -- Pager interception: if the player is mid-page, consume input
    if DAEMON and DAEMON.pager and DAEMON.pager.is_paging(session_id) then
        DAEMON.pager.handle_input(session_id, text)
        return
    end

    if text == "" then
        render_prompt(session_id)
        return
    end

    local verb, args_str, args = M.parse(text)
    -- Resolve alias to canonical name (for system commands)
    local resolved_verb = _aliases[verb] or verb

    -- ── 1. Room actions ──────────────────────────────────────────────────
    -- Check if the verb matches a room-scoped action on the player's
    -- current room. Room actions take priority over system commands.
    if DAEMON and DAEMON.world then
        local session = get_session(session_id)
        if session and session.character_id then
            local room = DAEMON.world.get_character_room_obj(session.character_id)
            if room then
                local action = room:get_action(verb)
                if action then
                    local ok, err = pcall(action.func, session_id, args_str, args)
                    if not ok then
                        log("error", "Room action '" .. verb .. "' error: " .. tostring(err))
                        send(session_id, "\r\nAn error occurred.\r\n")
                    end
                    render_prompt(session_id)
                    return  -- handled by room action
                end
            end
        end
    end

    -- ── 2. Channel shortcut ──────────────────────────────────────────────
    -- If the verb matches a channel name the player is subscribed to,
    -- treat the rest of the input as a message to that channel.
    if DAEMON and DAEMON.channel then
        local session = session or get_session(session_id)
        if session and session.character_id then
            if DAEMON.channel.is_subscribed(verb, session.character_id) then
                if args_str == "" then
                    send(session_id, "\r\nSay what on " .. verb .. "?\r\n")
                else
                    DAEMON.channel.send(verb, session.character_id, args_str)
                end
                render_prompt(session_id)
                return
            end
        end
    end

    -- ── 3. System commands ───────────────────────────────────────────────
    local mod = lazy_load(resolved_verb)
    if not mod then
        send(session_id, "\r\nUnknown command: '" .. verb .. "'. Type 'help' for a list.\r\n")
        render_prompt(session_id)
        return
    end

    -- Permission check
    if mod.permission and type(has_permission) == "function" then
        if not has_permission(session_id, mod.permission) then
            send(session_id, "\r\nYou don't have permission to do that.\r\n")
            render_prompt(session_id)
            -- Audit the denial
            if DAEMON and DAEMON.audit then
                DAEMON.audit.after_command(resolved_verb, session_id, args_str, false,
                    "permission denied: " .. mod.permission)
            end
            return
        end
    end

    local ok, err = pcall(mod.execute, session_id, args_str, args)
    if not ok then
        log("error", "Command '" .. resolved_verb .. "' error: " .. tostring(err))
        send(session_id, "\r\nAn error occurred processing that command.\r\n")
    end

    -- Render prompt after every command (the command itself no longer needs to)
    render_prompt(session_id)

    -- Notify audit daemon (it will check the watch list internally)
    if DAEMON and DAEMON.audit then
        DAEMON.audit.after_command(resolved_verb, session_id, args_str, ok, err)
    end
end

return M
