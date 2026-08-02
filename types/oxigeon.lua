--- Oxigeon MUD Driver — Efun Definitions for LuaLS
--- These are engine-provided global functions injected by the Rust driver.
--- This file is NOT executed at runtime; it exists solely for IDE type-checking.
---@meta

-- ═══════════════════════════════════════════════════════════════════════════════
-- I/O — Sending output to players
-- ═══════════════════════════════════════════════════════════════════════════════

--- Send text to a session. Raw output, no color processing or word wrap.
--- Prefer player:send() in command code; this is the low-level primitive.
---@param session_id string  The session to send to
---@param text string        The text to send (include \r\n yourself)
function send(session_id, text) end

--- Send prompt text to a session (no trailing newline appended).
---@param session_id string
---@param text string
function send_prompt(session_id, text) end

--- Send text to ALL connected sessions.
---@param text string  The text to broadcast
function broadcast(text) end

--- Send text to all sessions that have a specific permission.
---@param permission string  The permission to filter by
---@param text string        The text to send
---@return integer count     Number of recipients
function broadcast_to_perm(permission, text) end

--- Disconnect a session.
---@param session_id string
function disconnect(session_id) end

--- Send a GMCP message to a session.
---@param session_id string
---@param package string     GMCP package name (e.g. "Char.Vitals")
---@param data string        JSON-encoded data
function send_gmcp(session_id, package, data) end

--- Enable local echo on the client (e.g. after password input).
---@param session_id string
function start_echo(session_id) end

--- Suppress local echo on the client (e.g. for password input).
---@param session_id string
function stop_echo(session_id) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Sessions — Session state management
-- ═══════════════════════════════════════════════════════════════════════════════

--- Get the current session ID (set by the engine before calling handlers).
---@return string|nil session_id
function this_session() end

--- Get session info as a table.
---@param session_id string
---@return SessionInfo|nil
function get_session(session_id) end

---@class SessionInfo
---@field id string
---@field state string           "authenticating"|"authenticated"|"playing"
---@field account_id integer|nil
---@field character_id integer|nil
---@field ip string|nil
---@field window_width integer|nil
---@field window_height integer|nil

--- Get an array of all connected session IDs.
---@return string[]
function all_sessions() end

--- Set the state of a session ("authenticating", "authenticated", "playing").
---@param session_id string
---@param state string
function set_session_state(session_id, state) end

--- Authenticate a session (bind it to an account, transition to "authenticated").
---@param session_id string
---@param account_id integer
function authenticate_session(session_id, account_id) end

--- Transition a session to "playing" and bind a character.
---@param session_id string
---@param account_id integer
---@param character_id integer
function enter_game_session(session_id, account_id, character_id) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Accounts & Characters — Database operations
-- ═══════════════════════════════════════════════════════════════════════════════

--- Authenticate username/password. Returns account_id or nil.
---@param username string
---@param password string
---@return integer|nil account_id
function authenticate(username, password) end

--- Create a new account. Returns account_id or nil + error message.
---@param username string
---@param password string
---@return integer|nil account_id
---@return string|nil error
function create_account(username, password) end

--- Get account info by account_id.
---@param account_id integer
---@return AccountInfo|nil
function get_account(account_id) end

---@class AccountInfo
---@field id integer
---@field username string
---@field is_admin boolean

--- Set admin status on an account.
---@param account_id integer
---@param is_admin boolean
function set_admin(account_id, is_admin) end

--- Create a character for an account.
---@param account_id integer
---@param name string
---@return integer|nil character_id
---@return string|nil error
function create_character(account_id, name) end

--- Get all characters for an account.
---@param account_id integer
---@return CharacterInfo[]
function get_characters(account_id) end

---@class CharacterInfo
---@field id integer
---@field name string
---@field account_id integer

--- Get a single character by ID.
---@param character_id integer
---@return CharacterInfo|nil
function get_character(character_id) end

--- Save character data (JSON blob) to the database.
---@param character_id integer
---@param data string  JSON-encoded string
---@return boolean success
function save_character_data(character_id, data) end

--- Load character data from the database.
---@param character_id integer
---@return string|nil  JSON-encoded string, or nil if none saved
function load_character_data(character_id) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Logging & Observability
-- ═══════════════════════════════════════════════════════════════════════════════

--- Log a message to the server console (Rust tracing).
---@param level string   "error"|"warn"|"info"|"debug"|"trace"
---@param message string
function log(level, message) end

--- Get the current Unix timestamp (seconds since epoch).
---@return number
function time() end

--- Get server configuration values from server.toml.
---@param key string  Dot-separated config key (e.g. "game.name", "sessions.max_connections")
---@return any        The config value, or nil if not found
function config(key) end

--- Get server info (name, version, uptime, etc).
---@return ServerInfo|nil
function server_info() end

---@class ServerInfo
---@field name string
---@field version string
---@field started_at string
---@field uptime_seconds number
---@field connected_sessions integer
---@field lua_memory_kb number

--- Write a structured journal entry.
---@param level string    "error"|"warn"|"info"|"debug"
---@param message string
---@param meta? string    JSON-encoded metadata
---@return boolean
function journal_write(level, message, meta) end

--- Write an audit trail entry.
---@param action string   Action identifier (e.g. "cmd.ban")
---@param success boolean
---@param reason? string
---@return boolean
function audit_write(action, success, reason) end

--- Read recent journal entries.
---@param limit? integer   Max entries (default 20)
---@param level? string    Filter by level
---@return string[]        Array of raw JSON entry strings
function journal_read(limit, level) end

--- Read recent audit entries.
---@param limit? integer   Max entries (default 20)
---@return string[]        Array of raw JSON entry strings
function audit_read(limit) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- File I/O — Sandboxed file operations within mudlib/
-- ═══════════════════════════════════════════════════════════════════════════════

--- Read a file's contents. Path is relative to the mudlib root.
---@param path string
---@return string|nil contents
---@return string|nil error
function read_file(path) end

--- Write content to a file (creates or overwrites). Path is jailed to mudlib.
---@param path string
---@param content string
---@return boolean success
---@return string|nil error
function write_file(path, content) end

--- Append content to a file (creates if missing). Path is jailed to mudlib.
---@param path string
---@param content string
---@return boolean success
---@return string|nil error
function append_file(path, content) end

--- Check if a file exists. Path is relative to the mudlib root.
---@param path string
---@return boolean
function file_exists(path) end

--- List files in a directory. Returns basenames without extensions.
---@param path string  Directory path relative to mudlib root
---@return string[]    Array of file/directory names
function list_dir(path) end

--- Delete a file. Path is jailed to mudlib.
---@param path string
---@return boolean success
---@return string|nil error
function delete_file(path) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Timers
-- ═══════════════════════════════════════════════════════════════════════════════

--- Schedule a one-shot timer. Fires on_timer(id) after delay_secs.
---@param delay_secs number
---@param id string          Timer identifier (passed to on_timer)
---@return boolean
function schedule_timer(delay_secs, id) end

--- Schedule a repeating timer. Fires on_timer(id) every interval_secs.
---@param interval_secs number
---@param id string
---@return boolean
function schedule_repeating(interval_secs, id) end

--- Cancel a scheduled timer by ID.
---@param id string
---@return boolean
function cancel_timer(id) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Object State — Key/value state attached to game objects (rooms, items, etc.)
-- ═══════════════════════════════════════════════════════════════════════════════

--- Set a state key on an object.
---@param object_id string
---@param key string
---@param value any
function set_object_state(object_id, key, value) end

--- Get a state key from an object.
---@param object_id string
---@param key string
---@return any
function get_object_state(object_id, key) end

--- Get all state keys for an object as a table.
---@param object_id string
---@return table<string, any>
function get_all_object_state(object_id) end

--- Clear all state for an object.
---@param object_id string
function clear_object_state(object_id) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Persistent Store — Key/value persistence across server restarts
-- ═══════════════════════════════════════════════════════════════════════════════

--- Set a persistent key/value pair.
---@param key string
---@param value any
function set_persistent(key, value) end

--- Get a persistent value by key.
---@param key string
---@return any
function get_persistent(key) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- RBAC — Role-Based Access Control
-- ═══════════════════════════════════════════════════════════════════════════════

--- Check if a session has a permission.
---@param session_id string
---@param permission string
---@return boolean
function has_permission(session_id, permission) end

--- Refresh cached permissions for a session from the database.
---@param session_id string
function refresh_permissions(session_id) end

--- Create a new role.
---@param role_name string
---@return boolean success
function create_role(role_name) end

--- Delete a role.
---@param role_name string
---@return boolean success
function delete_role(role_name) end

--- List all roles, optionally with their permissions.
---@return RoleInfo[]
function list_roles() end

---@class RoleInfo
---@field id integer
---@field name string
---@field permissions string[]|nil

--- Assign a role to an account.
---@param account_id integer
---@param role_name string
---@return boolean success
function assign_role(account_id, role_name) end

--- Revoke a role from an account.
---@param account_id integer
---@param role_name string
---@return boolean success
function revoke_role(account_id, role_name) end

--- Get all roles for an account.
---@param account_id integer
---@return string[]  Array of role names
function get_roles(account_id) end

--- Grant a permission to a role.
---@param role_name string
---@param permission string
---@return boolean success
function grant_permission(role_name, permission) end

--- Revoke a permission from a role.
---@param role_name string
---@param permission string
---@return boolean success
function revoke_permission(role_name, permission) end

--- Get all permissions for a role.
---@param role_name string
---@return string[]
function get_permissions(role_name) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Hot Reload & Verification
-- ═══════════════════════════════════════════════════════════════════════════════

--- Hot-reload a Lua module. The engine re-reads the file and updates package.loaded.
---@param module_path string  Slash-separated path (e.g. "cmds/look", "daemons/world_d")
function reload(module_path) end

--- Compile-check a Lua file without executing it.
---@param path string  Path relative to mudlib root
---@return boolean ok
---@return string|nil error
function verify_file(path) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Time — Safe os-level time functions
-- ═══════════════════════════════════════════════════════════════════════════════

--- Get a high-resolution monotonic clock value (seconds, fractional).
---@return number
function os_clock() end

--- Get the current Unix timestamp (integer seconds since epoch).
---@return integer
function os_time() end

--- Format the current local time using strftime format codes.
---@param format string  strftime format string (e.g. "%Y-%m-%d %H:%M:%S")
---@return string
function os_date(format) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Mudlib-defined globals (from init.lua, available everywhere)
-- ═══════════════════════════════════════════════════════════════════════════════

--- Global daemon registry. All daemons attach themselves here on load.
---@type table<string, table>
DAEMON = {}

--- Get the Player object for a session.
--- Wraps the session → character → Player lookup.
---@param session_id string
---@return Player|nil
function get_player(session_id) end
