# Event Hooks

Event hooks are Lua functions defined globally in your mudlib that the Oxigeon driver calls when specific events occur.

You define them in `mudlib/init.lua` (or a file it requires).

## Required Hooks

### `on_connect(session_id)`
Called when a new client connects. Always called **before** any input is received.

```lua
function on_connect(session_id)
    log("info", "New connection: " .. session_id)
    set_session_state(session_id, "authenticating")
    send(session_id, "\r\nWelcome! Please enter your username: ")
end
```

**Note:** At this point, the session is in the `"connected"` state. You should immediately transition it to `"authenticating"` or start a login flow.

### `on_input(session_id, text)`
Called when the player sends a line of input. `text` is the decoded, trimmed line.

```lua
function on_input(session_id, text)
    local session = get_session(session_id)
    if not session then return end

    if session.state == "authenticating" then
        -- handle login
    elseif session.state == "playing" then
        -- handle commands
    end
end
```

**Note:** ECHO masking (`start_echo`/`stop_echo`) is controlled by your Lua code.

### `on_disconnect(session_id)`
Called when a client disconnects (gracefully or via network error). **Session data is still available** during this call but will be removed immediately after.

```lua
function on_disconnect(session_id)
    local session = get_session(session_id)
    if session and session.character_id then
        local char = get_character(session.character_id)
        if char then
            broadcast("\n" .. char.name .. " has left the game.\n")
        end
    end
end
```

---

## Optional Hooks

### `on_gmcp(session_id, package, data)`
Called when a GMCP message is received from a client.

- `package` — string like `"Core.Hello"`, `"Char.Login"`, etc.
- `data` — JSON string of the payload (parse with your own JSON library or ignore)

```lua
function on_gmcp(session_id, package, data)
    if package == "Core.Hello" then
        -- Client identified itself — could check client name/version
        send_gmcp(session_id, "Core.Goodbye", {})
    end
end
```

### `on_mxp_ready(session_id)`
Called once per session, when the client has finished the
[MXP](../protocols/mxp.md) handshake and will parse markup.

The reason this exists rather than only a capability field: MXP is offered in
the opening negotiation burst and the client's `<VERSION>` reply can arrive
several round trips later — **after `on_connect`**. So
`get_session().mxp_supported` answers "is it there" but not "is it there *now*",
which is the question anything that wants to greet the player with a clickable
line has to ask.

```lua
function on_mxp_ready(session_id)
    local s = get_session(session_id)
    log("info", "MXP ready for " .. (s.mxp_client or "an unnamed client"))
    send_rich(session_id, {
        "Type ", { send = "help mxp", text = "help mxp" }, " to see what your client can do.",
    })
end
```

You do not need this to *use* `send_rich` — a rich line sent to a client without
MXP simply renders as plain text. It is for the case where the timing matters.

### `on_load(module_name)`
Called **after** a module is hot-reloaded successfully.

```lua
function on_load(module_name)
    log("info", "Module reloaded: " .. module_name)
    -- Re-register anything that was cleared
end
```

### `on_unload(module_name)`
Called **before** a module is cleared for hot-reload.

```lua
function on_unload(module_name)
    -- Save any state you need to persist
    set_persistent("my_key", current_value)
end
```

### `on_timer(id)`
Called when a Tokio-backed timer fires. The `id` is the string identifier
passed to `schedule_timer()` or `schedule_repeating()`.

Typically you don't implement this yourself — the mudlib's default implementation
dispatches to `DAEMON.ticker.fire(id)`, which runs the registered callback.

```lua
function on_timer(id)
    if DAEMON and DAEMON.ticker then
        DAEMON.ticker.fire(id)
    end
end
```

### `on_shutdown()`
Called once, before the Lua VM stops, when the server is shutting down cleanly
(Ctrl+C). This is the last chance to write anything the game is holding in
memory.

It matters because `CHARACTER_D` is a write-back cache: a player's progress
reaches the database on an autosave tick, on disconnect, or here. Without this
hook a clean restart discards everything changed since the last tick.

```lua
function on_shutdown()
    log("info", "Shutdown: flushing game state")
    local ok, err = pcall(function() require('tasks.autosave').run() end)
    if not ok then log("error", "Shutdown autosave failed: " .. tostring(err)) end
end
```

Three things to know:

- **The driver waits for it**, so it must return. The wait is bounded by
  `game.shutdown_timeout_seconds` (default 30); when that expires the server
  logs an error and exits regardless, losing whatever had not been written.
- **It runs with the engine's identity**, like a timer tick, so permission-gated
  efuns are available to it.
- **It only runs on a clean shutdown.** A kill, a power cut, or a panic reaches
  no Lua at all — which is what `autosave_seconds` is for.

---

## Event Dispatch Order

For each input:

```
TCP read → Telnet parser → IAC handling → on_input(session_id, text)
```

For connections:

```
TCP accept → Initial negotiations (SGA, GMCP, MCCP2 offers) → on_connect(session_id)
```

For disconnects:

```
TCP close → on_disconnect(session_id) → session removed from registry
```

For a clean shutdown:

```
Ctrl+C → broadcast goodbye → on_shutdown() → driver waits for the Lua thread → exit
```

---

## Error Handling

If your event hook throws a Lua error, it is caught by the driver and logged at the `error` level. The session remains connected.

```lua
function on_input(session_id, text)
    local ok, err = pcall(function()
        -- wrap risky code in pcall for graceful handling
        process_command(session_id, text)
    end)
    if not ok then
        log("error", "Command error: " .. tostring(err))
        send(session_id, "\nAn error occurred. Please report this.\n")
    end
end
```
