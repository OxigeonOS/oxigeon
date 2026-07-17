# ECHO — Password Masking

The Telnet ECHO option (option 1) is used to mask sensitive input such as passwords.

## How It Works

By default, Telnet clients echo the player's typed characters locally. When the server sends `IAC WILL ECHO`, it tells the client "I will handle echoing" — and the client stops showing what the player types.

Oxigeon takes advantage of this: clients like Mudlet replace typed characters with `****` during ECHO masking.

## Lua Interface

```lua
-- At a password prompt:
start_echo(session_id)
send(session_id, "Password: ")

-- In on_input, after receiving the password:
function on_input(session_id, text)
    if state.step == "password" then
        stop_echo(session_id)       -- MUST stop echo before any other output
        send(session_id, "\r\n")    -- Newline after the hidden input
        -- Now process the password
    end
end
```

## Sequence

```
Server → Client: IAC WILL ECHO    ← start_echo()
Player types password (hidden)
Client → Server: (password text)
Server receives: on_input(session_id, "password")
Server → Client: IAC WONT ECHO    ← stop_echo()
Normal echoing resumes
```

> [!IMPORTANT]
> Always call `stop_echo()` before sending any visible text after a password prompt. Forgetting to stop ECHO will leave the player's subsequent input masked.

## Supported Clients

| Client | ECHO Masking |
|--------|-------------|
| Mudlet | ✅ Shows `****` |
| MUSH-Client | ✅ Hides input |
| TinyFugue | ✅ |
| Raw telnet | ⚠️ No visual masking, but input is not echoed |
