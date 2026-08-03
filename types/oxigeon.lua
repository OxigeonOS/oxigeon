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

--- Verify a username and password. Asynchronous: returns nothing immediately.
--- Argon2 runs on a worker pool, and the result arrives at the global
--- `on_auth_result(session_id, "authenticate", account, err)`.
---@param session_id string
---@param username string
---@param password string
function authenticate(session_id, username, password) end

--- Create a new account. Asynchronous, like `authenticate`; the result arrives
--- at `on_auth_result(session_id, "create_account", account, err)`.
---@param session_id string
---@param username string
---@param password string
function create_account(session_id, username, password) end

--- Called by the driver when an asynchronous `authenticate` or
--- `create_account` finishes. Exactly one of `account` and `err` is set.
---@param session_id string
---@param kind "authenticate"|"create_account"
---@param account AccountInfo|nil
---@param err string|nil  a message safe to show the player
function on_auth_result(session_id, kind, account, err) end

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

--- Save character data to the database.
--- Takes a Lua **table**, not a string — the driver serializes it. Raises if
--- the table cannot be represented as JSON: a table that is both a list and a
--- map, a cycle, NaN/infinity, or a function value. Callers should pcall.
---@param character_id integer
---@param data table  serialized by the driver
---@return boolean success
function save_character_data(character_id, data) end

--- Load character data from the database.
--- Returns a Lua **table**, not a string.
---@param character_id integer
---@return table|nil  the saved data, or nil if none saved
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
---@field started_at string           ISO 8601, captured at startup
---@field uptime_secs number          NOT `uptime_seconds` — that name was in
---                                   this stub for a while and `mudstatus.lua`
---                                   trusted it, so it printed "0s" uptime for
---                                   as long as the typo lived here.
---@field dropped_output number       output lost to full session channels
---@field compute ComputeInfo|nil     absent when [compute] is disabled

---@class ComputeInfo
---@field workers integer
---@field queue_depth integer
---@field instruction_limit integer
---@field in_flight integer
---@field running integer
---@field submitted integer
---@field completed integer
---@field failed integer
---@field timed_out integer
---@field refused integer
---@field cancelled integer
---@field wedged integer              non-zero means a worker is gone for good

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
-- Reload-Surviving Store — Key/value state that outlives a hot reload
--
-- NOT persisted to disk. This is a table living in the Lua VM, so it survives
-- `reload()` (which is its job — daemons use it to keep state across a reload)
-- but is gone on restart. For real persistence use save_character_data for
-- per-character data.
-- ═══════════════════════════════════════════════════════════════════════════════

--- Set a key/value pair that survives hot reload. Lost on restart.
---@param key string
---@param value any
function set_persistent(key, value) end

--- Get a value set by set_persistent. Lost on restart.
---@param key string
---@return any
function get_persistent(key) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Document Store — persisting anything, with no Rust
--
-- One generic table serves every collection. Reads return a DocumentRecord
-- envelope, so it is rec.data.field, not rec.field.
-- See docs/src/lua-api/document-store.md.
-- ═══════════════════════════════════════════════════════════════════════════════

---@class DocumentRecord
---@field collection string
---@field id string
---@field data table        what you stored
---@field created_at string RFC 3339, set on first write
---@field updated_at string RFC 3339, moves on every write

---@class DocumentQueryOpts
---@field limit integer?
---@field offset integer?
---@field sort string?   "id"|"created_at"|"updated_at" or a dotted JSON path
---@field order string?  "asc"|"desc"

--- Insert or replace a document. `created_at` survives an overwrite.
---@param collection string  lowercase letters, digits, underscores
---@param id string
---@param doc table
---@return string id
function db_put(collection, id, doc) end

--- Insert under a generated id.
---@param collection string
---@param doc table
---@return string id
function db_insert(collection, doc) end

---@param collection string
---@param id string
---@return DocumentRecord|nil
function db_get(collection, id) end

---@param collection string
---@param id string
---@return boolean
function db_exists(collection, id) end

---@param collection string
---@param id string
---@return boolean removed
function db_delete(collection, id) end

--- Query a collection. Always returns an array, never nil.
--- A query with no `limit` that matches more than [documents] max_results
--- RAISES rather than silently truncating.
---@param collection string
---@param filter table?  { field = value } or { field = { [op] = value } }
---@param opts DocumentQueryOpts?
---@return DocumentRecord[]
function db_find(collection, filter, opts) end

---@param collection string
---@param filter table?
---@return integer
function db_count(collection, filter) end

--- Recursive merge (RFC 7396). Objects merge key by key; arrays are replaced
--- wholesale. Atomic.
---@param collection string
---@param id string
---@param patch table
---@return boolean existed
function db_update(collection, id, patch) end

--- Remove one field, including a nested one ("target.area").
---@param collection string
---@param id string
---@param field string
---@return boolean
function db_unset(collection, id, field) end

--- Atomic increment. Creates the document if missing, so a counter needs no
--- bootstrap. Raises if the field holds something that is not a number.
---@param collection string
---@param id string
---@param field string
---@param delta number?  defaults to 1
---@return number new_value
function db_incr(collection, id, field, delta) end

---@return { name: string, count: integer }[]
function db_collections() end

--- Delete a whole collection. Gated by efun.db.clear.
---@param collection string
---@return integer deleted
function db_clear(collection) end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Compute — running long Lua on a worker thread
--
-- Worker VMs have NO efuns. A job receives arguments and returns a value; it
-- cannot see sessions, the world, the database or object state.
-- See docs/src/lua-api/compute.md.
-- ═══════════════════════════════════════════════════════════════════════════════

---@class ComputeOpts
---@field tag any?           echoed back untouched in meta.tag
---@field deadline_ms integer?  overrides [compute] default_deadline_ms

---@class ComputeMeta
---@field kind "ok"|"error"|"load_error"|"timeout"|"cancelled"|"budget"|"refused"
---@field tag any
---@field module string
---@field fn string
---@field queued_ms number
---@field run_ms number

--- Queue `module.fn_name(args)` on a compute worker. Returns immediately.
--- If an id comes back, exactly one on_compute_result fires for it; if nil
--- comes back, none does.
---@param module string     under a [compute] root, e.g. "compute.pathfind"
---@param fn_name string
---@param args any          copied, not shared; functions are refused
---@param opts ComputeOpts?
---@return string|nil id
---@return string|nil err
function compute(module, fn_name, args, opts) end

--- Ask a job to stop. A running job only stops if [compute] instruction_limit
--- is set, or if it polls compute_cancelled() itself.
---@param id string
---@return boolean was_live
function compute_cancel(id) end

--- Called when a compute job finishes, whatever the outcome.
---@param id string
---@param ok boolean
---@param value any
---@param err string|nil
---@param meta ComputeMeta
function on_compute_result(id, ok, value, err, meta) end

-- ─── Available only inside a compute job ─────────────────────────────────────

--- Buffered and written to the journal when the job finishes. The only way to
--- see inside a job — a debug adapter cannot attach to a worker VM.
---@param level string
---@param message string
function compute_log(level, message) end

--- Milliseconds left before the deadline, so a job can return a partial answer.
---@return number
function compute_deadline_ms() end

--- Whether compute_cancel() has been called for this job.
---@return boolean
function compute_cancelled() end

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
-- Debugging / tracing
-- ═══════════════════════════════════════════════════════════════════════════════

---@class TraceStatus
---@field mode         string    "off" | "time" | "calls" | "lines"
---@field armed        boolean   whether the Lua hook is currently installed
---@field all_sessions boolean
---@field sessions     string[]  opted-in session ids
---@field records      integer   trace records currently buffered
---@field capacity     integer   trace ring capacity
---@field timings      integer   command timings currently buffered
---@field dropped      integer   records evicted because the ring was full

--- Enable or disable execution tracing.
--- Installs a Lua debug hook, which forces the traced code onto the interpreter
--- (no JIT) — enable only while investigating, and turn it off afterwards.
---@param mode  string       "off" | "time" | "calls" | "lines"
---@param scope string|nil   nil = calling session, "all" = every session, or a session id
---@return boolean ok
---@return string|nil err
function trace_set(mode, scope) end

--- Current trace settings and buffer usage.
---@return TraceStatus
function trace_status() end

--- Most recent trace records, oldest first, preformatted as plain text.
---@param limit integer|nil  default 40
---@return string[]
function trace_show(limit) end

--- Most recent per-command timings, preformatted as plain text with a header row.
---@param limit integer|nil  default 20
---@return string[]
function trace_timings(limit) end

--- Empty both the trace and timing ring buffers.
function trace_clear() end

-- ═══════════════════════════════════════════════════════════════════════════════
-- Mudlib-defined globals (from init.lua, available everywhere)
-- ═══════════════════════════════════════════════════════════════════════════════

--- Global daemon registry. All daemons attach themselves here on load.
---@type table<string, table>
DAEMON = {}

--- Called once by the driver before the Lua VM stops, on a clean shutdown.
--- Runs with the engine's own identity (gated efuns are permitted), and the
--- driver waits for it to return — bounded by `game.shutdown_timeout_seconds`,
--- after which the server exits regardless. Flush anything held in memory here.
function on_shutdown() end

--- Get the Player object for a session.
--- Wraps the session → character → Player lookup.
---@param session_id string
---@return Player|nil
function get_player(session_id) end
