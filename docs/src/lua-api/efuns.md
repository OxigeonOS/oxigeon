# Efuns — Driver Functions

Efuns (external functions) are Rust functions exposed to the Lua mudlib.
They are registered globally and available from any Lua file.

---

## Output & Communication

### `send(session_id, text)`
Send a text message to a specific session. Newlines (`\n`) are automatically converted to `\r\n`.

```lua
send(session_id, "Welcome to the game!\n")
```

### `send_prompt(session_id, text)`
Send raw text **without** a trailing newline — intended for command prompts where you want
the cursor to remain on the same line as the prompt.

```lua
send_prompt(session_id, "> ")
```

### `broadcast(text)`
Send a text message to **all** connected sessions.

```lua
broadcast("\nThe world trembles. An earthquake strikes!\n")
```

### `disconnect(session_id)`
Disconnect a session immediately.

```lua
disconnect(session_id)
```

### `send_gmcp(session_id, package, data_table)`
Send a GMCP message to a session. `data_table` is a Lua table that will be JSON-encoded.

```lua
send_gmcp(session_id, "Char.Vitals", { hp = 100, max_hp = 100 })
send_gmcp(session_id, "Room.Info", { name = "The Void", exits = {} })
```

### `start_echo(session_id)`
Start ECHO masking — the player's input is hidden (for password entry).
Sends `IAC WILL ECHO` to the client.

```lua
start_echo(session_id)
send(session_id, "Password: ")
```

### `stop_echo(session_id)`
Stop ECHO masking — resumes normal input display.
Sends `IAC WONT ECHO` to the client.

```lua
-- After player typed password:
stop_echo(session_id)
send(session_id, "\n")
```

---

## Session Management

### `this_session() → string|nil`
Returns the session ID of the currently-executing session. Available from within event handlers.

```lua
function on_input(session_id, text)
    -- These are equivalent:
    local sid = this_session()  -- same as session_id in this context
end
```

### `get_session(session_id) → table|nil`
Returns a table of session properties, or `nil` if not found.

**Fields:**
| Field | Type | Description |
|-------|------|-------------|
| `id` | string | UUID session identifier |
| `protocol` | string | `"telnet"`, `"websocket"`, etc. |
| `address` | string | Remote IP:port |
| `state` | string | `"connected"`, `"authenticating"`, `"authenticated"`, `"playing"` |
| `account_id` | integer? | Set when state is `"authenticated"` or `"playing"` |
| `character_id` | integer? | Set when state is `"playing"` |

```lua
local session = get_session(session_id)
if session and session.state == "playing" then
    local char = get_character(session.character_id)
end
```

### `all_sessions() → table`
Returns an array of all active session IDs (strings).

```lua
local sessions = all_sessions()
send(session_id, "Players online: " .. #sessions .. "\n")
```

### `set_session_state(session_id, state_name)`
Manually set session state by name. Only valid for the simple states:
`"connected"` and `"authenticating"`.

For the authenticated and playing states, use `authenticate_session()` and `enter_game_session()`
instead — they carry the required structured data (account/character IDs).

```lua
set_session_state(session_id, "authenticating")
```

### `authenticate_session(session_id, account_id) → string|nil`
Transitions the session to the `"authenticated"` state and records the account ID.
In `"single"` multisession mode, this kicks any existing session for the same account and
returns its session ID. Returns `nil` if no session was kicked.

```lua
local account = authenticate(username, password)
if account then
    local kicked = authenticate_session(session_id, account.id)
    if kicked then
        log("info", "Kicked old session: " .. kicked)
    end
end
```

### `enter_game_session(session_id, account_id, character_id)`
Transitions the session to the `"playing"` state with the given account and character.
After this call, `get_session()` will return `state = "playing"`, `account_id`, and `character_id`.

```lua
-- Typical login flow:
authenticate_session(session_id, account.id)   -- → "authenticated"
enter_game_session(session_id, account.id, char.id)  -- → "playing"
```

---

## Account Management

### `authenticate(username, password) → table|nil`
Verify credentials against the database. Returns an account table on success, `nil` on failure.

```lua
local account = authenticate(username, password)
if account then
    send(session_id, "Welcome back, " .. account.username .. "!\n")
else
    send(session_id, "Invalid credentials.\n")
end
```

**Account table fields:** `id`, `username`, `is_admin`, `created_at`

### `create_account(username, password) → table|nil`
Create a new account. Returns account table or `nil` on failure (name taken, password too short, etc.).

```lua
local account = create_account(username, password)
if not account then
    send(session_id, "Could not create account.\n")
end
```

### `get_account(id) → table|nil`
Look up an account by its integer ID.

```lua
local account = get_account(session.account_id)
```

---

## Character Management

### `create_character(account_id, name) → table|nil`
Create a new character for an account. Subject to the `max_characters_per_account` limit.

```lua
local char = create_character(account.id, "Aldric")
```

**Character table fields:** `id`, `account_id`, `name`, `created_at`

### `get_characters(account_id) → table`
Get all characters for an account (array of character tables).

```lua
local chars = get_characters(account.id)
for i, char in ipairs(chars) do
    send(session_id, i .. ". " .. char.name .. "\n")
end
```

### `get_character(id) → table|nil`
Look up a character by its integer ID.

```lua
local char = get_character(session.character_id)
```

---

## Utility

### `log(level, message)`
Write to the server log. Levels: `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`.

```lua
log("info", "Player logged in: " .. username)
log("warn", "Unusual activity from session " .. session_id)
```

### `time() → number`
Returns the current Unix timestamp as a float (seconds since epoch).

```lua
local now = time()
send(session_id, "Server time: " .. now .. "\n")
```

### `config(key) → any`
Read a server configuration value.

| Key | Type | Example |
|-----|------|---------|
| `"game.name"` | string | `"My MUD"` |
| `"game.mudlib_path"` | string | `"./mudlib"` |
| `"accounts.allow_creation"` | bool | `true` |
| `"accounts.max_characters_per_account"` | integer | `1` |

```lua
local name = config("game.name")
send(session_id, "Welcome to " .. name .. "!\n")
```

---

## Hot-Reload & Persistence

### `reload(module_name)`
Hot-reload a Lua module. The old module version is evicted from `package.loaded`,
the file is re-read from disk, and the new version is compiled and installed.
The `on_unload` and `on_load` hooks are called around the reload.

```lua
-- Admin command handler:
if cmd == "reload" and is_admin then
    reload("areas.town")
end
```

> [!NOTE]
> If the reload fails (Lua syntax error), the **old version remains active** — the engine
> does not crash. Check the server log for the error.

### `set_persistent(key, value)`
Store a value that persists across module hot-reloads.

```lua
set_persistent("player_count", 0)
```

### `get_persistent(key) → any`
Retrieve a previously stored persistent value.

```lua
local count = get_persistent("player_count") or 0
```

> [!NOTE]
> Persistent storage lives in the Lua VM's memory. It survives `reload()` calls but is
> cleared if the server restarts. For cross-restart persistence, write to a file or database.
