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

## Inbound — what a client may say back

`on_gmcp` used to log the package name and return, so a client could negotiate
GMCP, announce what it supports and send `Core.Hello`, and the game never read a
word of it. It dispatches now, by package name, to handlers a game can add to.

```lua
DAEMON.gmcp.on("Game.Quest.Request", function(session_id, data)
    send_gmcp(session_id, "Game.Quest", build_quest_list(session_id))
end)
```

| Package | |
|---|---|
| `Core.Supports.Set` | an array of `"Module version"` strings; **read**, and it gates what is pushed |
| `Core.Supports.Add` / `.Remove` | the same list, edited |
| `Core.Hello` | the client naming itself — logged, because it is the first question asked when somebody reports a rendering bug |
| `Core.Ping` | echoed, because a client that pings and hears nothing concludes the connection is dead |

Case is ignored on both sides: clients disagree about capitalisation and the
spec does not care. An unhandled package is a debug line rather than an error —
a client sending something the game has never heard of must not break the
connection — and a handler that raises is contained the same way.

### `Core.Supports.Set` gates the outbound side

```lua
DAEMON.gmcp.wants(session_id, "Char.Effects")   --> boolean
```

A client that never asked for `Char` should not be sent forty `Char.Effects`
messages a minute. A module covers its packages, which is how the convention
works: supporting `Char` gets you `Char.Vitals`. A client that sent **no**
support list at all gets everything, which is the friendlier guess for an older
client and is what the game did before any of this existed.

### Custom packages

`Char`, `Room` and `Core` are conventions every client knows. Anything else is
yours, and belongs in the game layer — see `game/daemons/gmcp_game_d.lua`, which
registers `Game.Quest`, `Game.Quest.Request` and `Game.Quest.Track` without the
mudlib's dispatcher changing at all.

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
