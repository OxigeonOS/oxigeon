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

--- Take whatever a command module returned and register what is in it.
---
--- Two shapes. One table with an `execute` is the ordinary case — one file, one
--- command. A table with a `commands` array is one file declaring several,
--- which is what `cmds/directions.lua` is: twelve verbs that differ by one
--- string each, and twelve near-identical files to maintain otherwise.
--- @return number  how many were registered
local function register_module(mod, module_path)
    if type(mod) ~= "table" then return 0 end

    if type(mod.execute) == "function" then
        register(mod)
        return 1
    end

    if type(mod.commands) == "table" then
        local n = 0
        for _, cmd in ipairs(mod.commands) do
            if type(cmd) == "table" and type(cmd.execute) == "function"
                and type(cmd.name) == "string" then
                register(cmd)
                n = n + 1
            else
                log("error", "COMMANDS: '" .. module_path
                    .. "' has an entry with no name or execute")
            end
        end
        return n
    end

    return 0
end

--- Eagerly load ALL command modules from configured paths.
-- Called once on first dispatch (or after flush_cache).
-- Uses list_dir() efun to discover .lua files, and recurses into
-- subdirectories so `cmds/admin/` is a category rather than a silence.
local _loaded_all = false

--- A symlink loop inside the jail would otherwise hang the game thread.
local MAX_DEPTH = 8

local function load_dir(prefix, dir_path, seen, depth)
    if depth > MAX_DEPTH then
        log("error", "COMMANDS: giving up at depth " .. depth .. " in '" .. dir_path .. "'")
        return
    end

    local ok, entries = pcall(list_dir, dir_path)
    if not ok then
        log("error", "COMMANDS: list_dir('" .. dir_path .. "') failed: " .. tostring(entries))
        return
    end
    if type(entries) ~= "table" then return end

    -- `list_dir` returns { name, is_dir, size } entries and has done for as
    -- long as the docs have described it. It used to return bare module stems
    -- here, because a second, unjailed copy of the efun was overwriting the
    -- real one — so this loop was written against a contract that only existed
    -- by accident. It also merges the game and mudlib roots, so a game-layer
    -- `cmds/admin/` is found by the same walk.
    for _, entry in ipairs(entries) do
        local file = type(entry) == "table" and entry.name or nil
        if file then
            if entry.is_dir then
                load_dir(prefix .. "." .. file, dir_path .. "/" .. file, seen, depth + 1)
            else
                local name = file:match("^(.+)%.lua$")
                -- `init.lua` is reachable as the directory itself; requiring it
                -- again under its own name would load it twice.
                if name and name ~= "init" then
                    local module_path = prefix .. "." .. name
                    -- Keyed on the module, not on the command name. The old
                    -- guard used the *file stem*, which assumed the stem was
                    -- the verb — false the moment one file declares twelve.
                    if not seen[module_path] then
                        seen[module_path] = true
                        local rok, mod = pcall(require, module_path)
                        if rok then
                            register_module(mod, module_path)
                        else
                            log("error", "COMMANDS: failed to load '" .. module_path
                                .. "': " .. tostring(mod))
                            if DAEMON and DAEMON.journal then
                                pcall(DAEMON.journal.error, "COMMANDS: failed to load '"
                                    .. module_path .. "': " .. tostring(mod))
                            end
                        end
                    end
                end
            end
        end
    end
end

local function load_all_commands()
    if _loaded_all then return end
    _loaded_all = true
    if type(list_dir) ~= "function" then return end

    local seen = {}
    for _, prefix in ipairs(get_cmd_prefixes()) do
        -- Convert dot prefix back to slash path for list_dir
        load_dir(prefix, prefix:gsub("%.", "/"), seen, 1)
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
    local verb, rest

    -- A punctuation alias attaches to what follows it. `'hello` is `say hello`
    -- and `:grins` is `emote grins` — which is how everyone types them, and
    -- neither worked: splitting on whitespace made the verb `'hello`, which
    -- resolves to nothing. So a leading punctuation character is split off
    -- **when it is a registered verb**, and only then: `:-)` should not become
    -- an emote of `-)` on a mudlib that has no `:` command.
    local punct, tail = text:match("^([^%w%s])(.*)$")
    if punct and M.resolve(punct) then
        verb, rest = punct, tail:gsub("^%s+", "")
    else
        verb, rest = text:match("^(%S+)%s*(.*)")
    end

    if not verb then return nil, "", {} end
    local args = {}
    for tok in rest:gmatch("%S+") do
        args[#args + 1] = tok
    end
    return verb:lower(), rest, args
end

--- Return the command registry (for help/introspection).
-- Keys are canonical command names, values are module tables.
--
-- Loads every command first. Without that this answered with whatever had
-- happened to be dispatched so far, which for `help` meant a list that grew as
-- you used the game — the one case where a lazy registry is exactly wrong.
function M.registry()
    if not _loaded_all then load_all_commands() end
    return _registry
end

--- The canonical name a verb or alias resolves to, or nil.
--- @param verb string
--- @return string|nil
function M.resolve(verb)
    if type(verb) ~= "string" then return nil end
    if not _loaded_all then load_all_commands() end
    if _registry[verb] then return verb end
    return _aliases[verb]
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
    --- add this check to satisfy linter
    if not verb then
        render_prompt(session_id)
        return
    end
    -- Resolve alias to canonical name (for system commands)
    local resolved_verb = _aliases[verb] or verb
    local session = get_session(session_id)

    -- ── 1. Room actions ──────────────────────────────────────────────────
    -- Check if the verb matches a room-scoped action on the player's
    -- current room. Room actions take priority over system commands.
    if DAEMON and DAEMON.world then
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
