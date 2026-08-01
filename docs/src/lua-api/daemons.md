# Daemons — Service Layer

In Oxigeon, a **Daemon** is a persistent service or singleton object that runs in the background and manages a specific domain of engine or game logic. 

Daemons sit in the `daemons/` directories (either `mudlib/daemons/` for core systems or `game/daemons/` for game content) and are registered globally.

## The DAEMON Global

To make daemons easily accessible from anywhere in the codebase without complex require paths, the engine provides a global `DAEMON` table.

Daemons are registered into this table during initialization (`mudlib/init.lua` and `game/init.lua`).

```lua
-- Accessing a daemon from anywhere
DAEMON.journal.log("info", "Server startup complete.")
```

## Core Daemons

### Mudlib Layer (`mudlib/daemons/`)

| Daemon | Key | Purpose |
|--------|-----|---------|
| `journal_d` | `DAEMON.journal` | Structured logging to server console and log files. |
| `audit_d` | `DAEMON.audit` | Security and compliance audit trail for player/admin actions. |
| `ticker_d` | `DAEMON.ticker` | Timer scheduler — manages Lua callbacks for Tokio-backed async timers. |
| `event_d` | `DAEMON.event` | Signal/event system — Godot-style named event channels with subscribe/emit. |
| `prompt_d` | `DAEMON.prompt` | Prompt template engine — per-player customizable prompt rendering with variable substitution. |

### Game Layer (`game/daemons/`)

| Daemon | Key | Purpose |
|--------|-----|---------|
| `room_d` | `DAEMON.room` | Room creation — data-oriented (`from_data`, `load_area`, `merge`) and builder pattern. |
| `character_d` | `DAEMON.character` | In-memory character state cache with DB persistence. |
| `world_d` | `DAEMON.world` | Room registry, character locations, movement, virtual providers, area metadata. |
| `codegen_d` | `DAEMON.codegen` | Code generation for OLC — produces clean Lua data files for rooms and area metadata. |
| `olc_d` | `DAEMON.olc` | Online Creation session manager — tracks per-session OLC state (area, room, mode). |

### journal_d vs audit_d

These serve distinct purposes:

- **journal_d** (`DAEMON.journal`) — "What went wrong?" Operational events: errors, warnings, daemon load/unload, module reloads.
- **audit_d** (`DAEMON.audit`) — "Who did this?" Security trail: admin commands, permission denials, privileged actions.

## Creating Your Own Daemon

Creating a daemon is as simple as defining a Lua module and registering it. By convention, daemon files are suffixed with `_d.lua`.

**1. Create the Daemon (`game/daemons/weather_d.lua`)**
```lua
local M = {}

local current_weather = "sunny"

function M.get_weather()
    return current_weather
end

function M.set_weather(weather)
    current_weather = weather
    log("info", "Weather changed to: " .. weather)
end

return M
```

**2. Register the Daemon (`game/init.lua`)**
```lua
ok, err = pcall(function() DAEMON.weather = require('daemons.weather_d') end)
if not ok then log("error", "Failed to load weather_d: " .. tostring(err)) end
```

**3. Use the Daemon**
```lua
-- In a data-oriented room description (lfun)
{
    id = "field.open",
    short = "Open Field",
    description = function(room)
        if DAEMON.weather.get_weather() == "raining" then
            return "You stand in a muddy field. Rain pours down on you."
        else
            return "You stand in a grassy field under a clear sky."
        end
    end,
}
```

## Daemon Best Practices

- Wrap every `require()` in `pcall` during init so a broken daemon doesn't crash the layer.
- Log critical failures to both `log()` and `DAEMON.journal`.
- Validate inputs — return clear failure values rather than crashing.
- See [GEMINI.md](../../GEMINI.md) for the full error handling policy.
