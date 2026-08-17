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

> [!TIP]
> You generally do not need to call `send_prompt()` directly from commands or room actions. The command dispatcher calls `DAEMON.prompt.render(session_id)` automatically after every command, which resolves the player's customizable prompt template. Use `send_prompt()` only in low-level code (login flow, raw protocol handlers).

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

### `send_rich(session_id, parts, opts?) → boolean`
Send a line built from **parts** rather than from a string. On a client that
negotiated [MXP](../protocols/mxp.md) it renders as markup — a clickable
command, a room-name line tag; on every other client it renders as the same
prose with the affordance dropped.

```lua
send_rich(session_id, {
    "The baker offers ",
    { send = "buy bread", hint = "A fresh loaf — 3 copper", text = item.short },
    " for three copper.",
})
```

Two things to know before writing the first call:

- **The driver escapes every string you pass**, and it is the only thing in the
  system that writes a `<`. So `item.short` can be whatever a player named it
  and cannot become markup. There is deliberately no `mxp_escape` helper — it
  would exist only to be forgotten at the one call site that mattered.
- **A rich line terminates itself.** Unlike `send`, do not append `\r\n`: the
  line boundary is where an MXP mode reverts, so the renderer needs it as a
  property rather than as a character. Pass `opts.newline = false` for a prompt.

Each element of `parts` is a string (literal text) or a table:

| Key | Meaning |
|---|---|
| `text` | Literal content. Mutually exclusive with `parts`. |
| `parts` | Nested array, for a group inside a group. |
| `send` | Command to run when clicked. An array is a popup menu. |
| `hint` | Mouse-over text. With a menu, `[1]` is the caption and the rest label the items. |
| `href` | A URL. `http://`, `https://` or `mailto:` only. |
| `prompt` | Put the command on the input line instead of running it. |
| `expire` | Name this link, for a later `mxp_expire`. |
| `var` | Also store the rendered text in a client variable of this name. |
| `br` | A hard line break. Ignores every other key. |

`opts` takes `mode` (`"secure"` by default, or `"open"`/`"locked"`), `line`
(`"room_name"`, `"room_desc"`, `"room_exits"`, `"welcome"`, or a number 20-99)
and `newline`.

```lua
-- A popup menu: three commands behind one word.
send_rich(session_id, {
    { send = { "buy bread", "buy cake" },
      hint = { "Shop", "Bread — 3cp", "Cake — 8cp" },
      text = "the counter" },
})

-- Client-side room tagging, for a client with an automapper.
send_rich(session_id, { room.name }, { line = "room_name" })
```

Returns `false` if the session is gone or its output channel is full.
**Raises** on an author error — a command containing `|` (which separates menu
items and has no escape), a `javascript:` URL, a tree deeper than 32 — with the
offending field named, the same convention `lua_to_json` and the `db_*` efuns
follow.

### `mxp_var(session_id, name, value) → boolean`
Set a client-side variable **and** display the value: `<VAR hp>40</VAR>`. That
is the difference between `<VAR>` and `<!ENTITY>`, which this driver does not
implement. Session-scoped, so there is nothing to replay on reconnect. The name
may hold only letters, digits and underscores, because it goes into the tag
itself.

```lua
mxp_var(session_id, "hp", char.hp)
```

### `mxp_expire(session_id, name?) → boolean`
Retire the links tagged with `name`, or every named link if omitted — a room's
exits stop being clickable once the player has left the room. Links that never
carried an `expire` name never expire.

```lua
send_rich(session_id, { { send = "north", expire = "exits", text = "N" } })
-- …later, on leaving:
mxp_expire(session_id, "exits")
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
| `window_width` | integer? | Columns, from NAWS or a WebSocket `hello` |
| `window_height` | integer? | Rows, same source |
| `terminal_type` | string? | From TTYPE, e.g. `"xterm-256color"` |
| `gmcp_supported` | boolean | The client negotiated [GMCP](../protocols/gmcp.md) |
| `gmcp_packages` | string[]? | Present only if the client listed any |
| `mxp_supported` | boolean | The client negotiated [MXP](../protocols/mxp.md) |
| `mxp_version` | string? | MXP spec level from its `<VERSION>` reply, e.g. `"0.4"` |
| `mxp_client` | string? | Client name and version, e.g. `"mushclient 5.06"` |
| `mxp_supports` | string[]? | `+tag` / `-tag` from its `<SUPPORTS>` reply |
| `dropped_output` | integer | Messages this session has lost to a full channel |

```lua
local session = get_session(session_id)
if session and session.state == "playing" then
    local char = get_character(session.character_id)
end
```

The capability fields are what a transport *discovered*, copied onto the session
— see `publish_capabilities`. Branch on the booleans: `mxp_client` and
`gmcp_packages` are extras a client may never volunteer, and treating their
absence as "the protocol is not there" disables the feature for the majority
that simply did not answer.

`mxp_client` is **not** `terminal_type`. TTYPE answers a different question and
a client is entitled to give two different answers.

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
-- `account` here comes from on_auth_result; see Account Management below.
local kicked = authenticate_session(session_id, account.id)
if kicked then
    log("info", "Kicked old session: " .. kicked)
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

> [!IMPORTANT]
> `authenticate` and `create_account` are **asynchronous**. They hash with
> Argon2, which costs a few hundred milliseconds, and the whole game runs on
> one Lua thread — doing it inline froze the world on every login, before
> authentication, so anyone able to open a socket could freeze it on demand.
> Both efuns hand the work to a worker pool and return nothing; the answer
> arrives at the global `on_auth_result`.

### `authenticate(session_id, username, password)`
Verify credentials against the database. Returns nothing. The result is delivered to `on_auth_result`.

### `create_account(session_id, username, password)`
Create a new account. Returns nothing. The result is delivered to `on_auth_result`.

### `on_auth_result(session_id, kind, account, err)` *(mudlib hook)*
Called by the driver when either of the above finishes. `kind` is `"authenticate"` or `"create_account"`. Exactly one of `account` and `err` is set; `err` is a message safe to show the player.

```lua
function on_auth_result(session_id, kind, account, err)
    if not account then
        send(session_id, (err or "Login failed.") .. "\r\n")
        return
    end
    send(session_id, "Welcome back, " .. account.username .. "!\r\n")
    authenticate_session(session_id, account.id)
end
```

**Account table fields:** `id`, `username`, `is_admin`, `created_at`

A request can be refused before any hashing happens — the queue is bounded, and an address is locked out for 30 seconds after 5 consecutive failures. Refusals come back through `on_auth_result` like any other failure, so there is only one place a login can finish.

While a hash is in flight, `mudlib/login.lua` ignores further input from that session. Queueing it would let one connection stack up Argon2 work, which is the denial of service this design exists to prevent.

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

### `save_character_data(char_id, data_table) → boolean`
Serialize a Lua table to JSON and save it to the character's persistent data column.
Returns `true` on success, `false` on failure.

### `load_character_data(char_id) → table|nil`  
Load the character's persistent data from the database, deserializing the JSON into a Lua table.
Returns `nil` if the character doesn't exist.

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
Read a server configuration value by **dotted path**. Any key in `server.toml`
is readable — this walks the parsed configuration rather than consulting a list
of keys someone remembered to add.

```lua
local name = config("game.name")
send(session_id, "Welcome to " .. name .. "!
")

config("game.command_paths")         --> { "cmds" }   (a table)
config("limits.lua_memory_mb")       --> 64
config("sessions.multisession_mode") --> "single"
config("nonsense.key")               --> nil          (never an error)
```

An unknown key is `nil`, not an error: Lua reads config with `or <fallback>`
throughout, and raising would turn every typo into a dead daemon rather than a
default.

**`[game]` accepts keys the driver has no opinion about.** They are captured
rather than rejected, and readable the same way:

```toml
[game]
respawn_room = "thornhollow.square"
shop_restock_seconds = 600
```

```lua
local room = config("game.respawn_room")
```

That is why `death_d` no longer has one game's room hardcoded in the *mudlib*
layer. Before this was generic, every game-layer setting needed a Rust edit
first, and that pressure is what put `wizard_workshop.entrance` in a driver file.

Keys with a driver default answer with it when unset — `game.autosave_seconds`
is 300, `game.combat_round_seconds` is 3, and so on. Those defaults live in one
table rather than at each call site, because a default repeated in two places is
a default two places can disagree about.

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

---

## Object State

In-memory key/value storage scoped to any object ID (rooms, items, mobs).
Survives hot-reloads but not server restarts.
See [World Building — Room State](./world-building.md#room-state) for examples.

### `set_object_state(id, key, value)`
Set a key/value pair on an object's state.

```lua
set_object_state("dungeon.cell", "door_open", true)
set_object_state("mob.guard_1", "alert_level", 3)
```

### `get_object_state(id, key) → any`
Get a single value from an object's state. Returns `nil` if the key or object has no state.

```lua
local open = get_object_state("dungeon.cell", "door_open")
```

### `get_all_object_state(id) → table|nil`
Get the entire state table for an object, or `nil` if no state has been set.

```lua
local state = get_all_object_state("dungeon.cell")
if state then
    for k, v in pairs(state) do
        log("debug", k .. " = " .. tostring(v))
    end
end
```

### `clear_object_state(id)`
Remove all state for an object.

```lua
clear_object_state("dungeon.cell")
```

---

## Timers

Tokio-backed timers that sleep asynchronously and fire precisely when due.
Zero polling — each timer is an independent async task.
See [TICKER_D](./world-building.md#tickers) for the Lua-side API.

### `schedule_timer(id, delay_seconds)`
Schedule a one-shot timer. When it fires, `on_timer(id)` is called.

```lua
schedule_timer("puzzle.reset", 10.0)
```

### `schedule_repeating(id, interval_seconds)`
Schedule a repeating timer. Fires `on_timer(id)` every interval.

```lua
schedule_repeating("mob.guard.echo", 15.0)
```

### `cancel_timer(id) → boolean`
Cancel a scheduled timer. Returns `true` if found and cancelled.

```lua
cancel_timer("mob.guard.echo")
```

> [!NOTE]
> Timer efuns are low-level. Use `DAEMON.ticker` for the high-level API:
> `DAEMON.ticker.after(delay, id, func)` and `DAEMON.ticker.every(interval, id, func)`.
