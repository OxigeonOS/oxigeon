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

All of them live in `mudlib/daemons/`. Anything substantial has its own page;
this table is the directory.

### Infrastructure

| Daemon | Key | Purpose |
|--------|-----|---------|
| `journal_d` | `DAEMON.journal` | Structured logging to server console and log files. See [Observability](./observability.md). |
| `audit_d` | `DAEMON.audit` | Security and compliance audit trail for player/admin actions. |
| `ticker_d` | `DAEMON.ticker` | Timer scheduler — Lua callbacks for Tokio-backed async timers. |
| `event_d` | `DAEMON.event` | Signal system — named channels with subscribe/emit. See [Signals](./signals.md). |
| `task_d` | `DAEMON.task` | Named periodic work — `schedule`, `pause`, `resume`, `run_now`, `cancel`, `list`. A raw ticker is anonymous and fire-and-forget; a task has an id an operator can act on. |
| `cache_d` | `DAEMON.cache` | Tiered game state: memory, write-behind, write-through. See [State Cache](./state-cache.md). |

### World

| Daemon | Key | Purpose |
|--------|-----|---------|
| `room_d` | `DAEMON.room` | Room creation — data-oriented (`from_data`, `load_area`, `merge`) and builder pattern. |
| `world_d` | `DAEMON.world` | Room registry, character locations, movement, virtual providers, area metadata. |
| `item_d` | `DAEMON.items` | Item templates **and instances**: spawn, move, destroy, and what is on a floor or in a container. See [Items](./items.md). |
| `tag_d` | `DAEMON.tag` | The reverse index over tags — "which rooms are outdoors" without walking the world. |
| `shop_d` | `DAEMON.shop` | Stock, prices, restocking and a ledger. See [Shops](./shops.md). |
| `mob_d` | `DAEMON.mobs` | Creature templates, instances, room occupancy, respawn. See [Combat](./combat.md). |

### Characters

| Daemon | Key | Purpose |
|--------|-----|---------|
| `character_d` | `DAEMON.character` | In-memory character state cache with DB persistence. See [Character Data](./character-data.md). |
| `trait_d` | `DAEMON.trait` | Attributes, derived values and regeneration. See [Traits](./traits.md). |
| `effect_d` | `DAEMON.effect` | Buffs, debuffs and the event pipeline. See [Effects](./effects.md). |
| `cooldown_d` | `DAEMON.cooldown` | "Not yet" gates, stored as expiry. See [State Cache](./state-cache.md). |
| `combat_d` | `DAEMON.combat` | Engagement and rounds. See [Combat](./combat.md). |
| `death_d` | `DAEMON.death` | Death handling, respawn and what death costs. Where the dead reappear comes from `game.respawn_room`, not from a constant in this layer. |

### Interface

| Daemon | Key | Purpose |
|--------|-----|---------|
| `prompt_d` | `DAEMON.prompt` | Per-player prompt templates with variable substitution. |
| `channel_d` | `DAEMON.channel` | Chat channels and subscriptions. |
| `gmcp_d` | `DAEMON.gmcp` | GMCP, both ways — pushes `Char.Vitals`, `Char.Status`, `Char.Effects` and `Room.Info`; dispatches `Core.Supports.Set`, `Core.Hello`, `Core.Ping` and whatever a game registers. |
| `pager_d` | `DAEMON.pager` | Paged output for long text. |
| `snoop_d` | `DAEMON.snoop` | Admin session snooping. |

### Building

| Daemon | Key | Purpose |
|--------|-----|---------|
| `codegen_d` | `DAEMON.codegen` | Reads and writes an area's data files. Decides *where* files go and what a file is made of; `lib/serialize.lua` decides how a value is written and `lib/schema.lua` which fields exist. |
| `olc_d` | `DAEMON.olc` | What a builder is working on: the area, the cursor, and the unsaved drafts. |
| `verify_d` | `DAEMON.verify` | The content linter. Reads areas **from disk**, never the registry, and reports without changing anything. |
| `adopt_d` | `DAEMON.adopt` | Brings a hand-authored area under OLC. Reports first; `--confirm` writes. Never parses Lua source. |
| `editor_d` | `DAEMON.editor` | A line editor, shaped like `pager_d` and intercepted just after it. What makes a six-line room description typable. |
| `fs_d` | `DAEMON.fs` | Where each session is standing in the file tree, for `ls`/`cd`/`pwd`/`cat`. Separate from `olc_d` because `cd` outlives a build session. |

See [OLC](./olc.md) for how they fit together.

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
- See `CLAUDE.md` in the repository root for the full error handling policy.
  (Not linked: it is a contributor file rather than a page of this book, and a
  link out of the rendered book would go nowhere.)
