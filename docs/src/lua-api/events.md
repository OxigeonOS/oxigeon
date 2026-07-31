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
