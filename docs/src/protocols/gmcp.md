# GMCP — Generic MUD Communication Protocol

GMCP (Telnet option 201) provides a structured, JSON-based channel for rich game data alongside normal text.

## Negotiation

```
Server → Client: IAC WILL GMCP
Client → Server: IAC DO GMCP
```

After negotiation, GMCP messages flow as subnegotiations:

```
IAC SB GMCP "Package.Message" {json} IAC SE
```

## Lua Interface

### Sending GMCP

```lua
-- In any event handler:
send_gmcp(session_id, "Char.Vitals", {
    hp = 100,
    max_hp = 100,
    mp = 50,
    max_mp = 50
})

send_gmcp(session_id, "Room.Info", {
    name = "The Market Square",
    area = "Midgaard",
    exits = { "north", "east", "south" }
})
```

### Receiving GMCP

```lua
function on_gmcp(session_id, package, data)
    if package == "Core.Hello" then
        -- data is a JSON string; parse it with your JSON library
        log("debug", "Client identified: " .. data)
    elseif package == "Char.Login" then
        -- Some clients send login info via GMCP
    end
end
```

## Common GMCP Packages

These are conventions followed by popular MUD clients (Mudlet, MUSH-Client):

| Package | Direction | Description |
|---------|-----------|-------------|
| `Core.Hello` | Client→Server | Client identification |
| `Core.KeepAlive` | Client→Server | Ping |
| `Core.Goodbye` | Server→Client | Server says goodbye |
| `Char.Vitals` | Server→Client | HP, MP, etc. |
| `Char.Status` | Server→Client | Character info |
| `Char.Login` | Client→Server | Login credentials (some clients) |
| `Room.Info` | Server→Client | Current room details |
| `Room.Players` | Server→Client | Other players in room |
| `Comm.Channel` | Server→Client | Channel messages |
