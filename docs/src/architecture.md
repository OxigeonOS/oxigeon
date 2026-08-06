# Architecture Overview

Oxigeon is structured in three layers. Each layer has a specific responsibility and knows nothing about the layers above it.

## Layer Diagram

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 3: Mudlib (Lua)                                      │
│                                                             │
│  mudlib/                → Core system layer                 │
│  ├── init.lua           → setup daemons, event hooks        │
│  ├── daemons/           → journal_d, audit_d, ticker_d,     │
│  │                        event_d, prompt_d, cache_d,       │
│  │                        trait_d, effect_d, cooldown_d,    │
│  │                        world_d, room_d, character_d,     │
│  │                        item_d, mob_d, combat_d, ...      │
│  ├── lib/               → object, item, weapon, armor,      │
│  │                        mobile, player, room, traits,     │
│  │                        effects, jsonsafe, persist        │
│  └── cmds/              → the command set                   │
│                                                             │
│  game/                  → Game content layer (data only)    │
│  ├── init.lua           → register traits, effects, areas   │
│  ├── traits/            → attribute definitions             │
│  ├── effects/           → buff and debuff definitions       │
│  └── areas/             → rooms, items, mobs (data)         │
└─────────────────────────────────────────────────────────────┘
         │ require(), efuns calls
┌─────────────────────────────────────────────────────────────┐
│  Layer 2: domain (Rust — creator-facing)                    │
│                                                             │
│  src/domain/db/         → Diesel pool, schema               │
│  src/domain/models/     → Account, Character (Diesel ORM)   │
│  src/config/            → driver.toml, server.toml parsing  │
│                                                             │
│  This is the layer creators MUST touch for database models, │
│  config changes, and schema additions.                      │
└─────────────────────────────────────────────────────────────┘
         │ Arc<T>, channels
┌─────────────────────────────────────────────────────────────┐
│  Layer 1: Core (Rust — driver internals)                    │
│                                                             │
│  src/core/network/      → Telnet, GMCP, MCCP2, ECHO         │
│  src/core/session/      → Session, SessionHandler           │
│  src/core/scripting/    → Lua VM, efuns, sandbox            │
│  src/driver.rs          → Coordinator, main loop            │
└─────────────────────────────────────────────────────────────┘
```

## Component Responsibilities

### Core Layer (Rust)
The core layer knows nothing about the game. It speaks TCP/Telnet, manages sessions, and runs the Lua VM.

| Component | File | Purpose |
|-----------|------|---------|
| `TelnetListener` | `core/network/telnet/mod.rs` | Accept TCP connections |
| `TelnetParser` | `core/network/telnet/parser.rs` | Byte-level Telnet FSM |
| `OptionNegotiator` | `core/network/telnet/option.rs` | RFC 1143 Q Method |
| `TelnetCodec` | `core/network/telnet/codec.rs` | CR/LF, IAC, GMCP encode/decode |
| `TelnetConnection` | `core/network/telnet/connection.rs` | Per-connection writer |
| `Session` | `core/session/session.rs` | Non-persistent connection state |
| `SessionHandler` | `core/session/handler.rs` | Session registry + multisession policy |
| `ScriptEngine` | `core/scripting/engine.rs` | Lua VM thread, hot-reload, event dispatch |
| `EfunContext` | `core/scripting/efuns.rs` | All Lua efun registrations |
| `Driver` | `driver.rs` | Coordinator — wires everything together |

### domain Layer (Rust)
The domain layer contains things creators will need to touch: database models, schema, config types.

| Component | File | Purpose |
|-----------|------|---------|
| `AnyPool` | `domain/db/connection.rs` | Runtime DB backend selection |
| `schema` | `domain/db/schema.rs` | Diesel table definitions |
| `Account` + `DieselAccountStore` | `domain/models/account.rs` | Account model + Argon2 passwords |
| `Character` + `DieselCharacterStore` | `domain/models/character.rs` | Character model |
| `AccountStore`, `CharacterStore` | `domain/traits.rs` | Swappable backend traits |
| `DriverConfig` | `config/driver_config.rs` | Infrastructure config |
| `ServerConfig` | `config/server_config.rs` | Game config |

### Mudlib Layer (Lua)
Your game, in two parts, and the line between them is the one to hold on to:

- **`mudlib/`** — anything a *second* game would want unchanged. Login, command
  dispatch, the daemons, the object model, items and equipment, shops, traits
  and effects, the state cache.
- **`game/`** — this game. Rooms, creatures, items, spells, quests, and the
  *policy* decisions the driver deliberately has no view on: whether an
  aggressive creature attacks, whether it rains, which roles exist, what a
  quest is.

The test of which side something goes on is not size or subject, it is: would
another game want this one unchanged, or would it want a different file? A
pathfinder is mudlib; the `navigate` command that decides what to do with a
route is game. `Mobile.aggressive` is mudlib; `aggro_d`, which reads it, is not.

The driver calls these global Lua functions (defined in `mudlib/init.lua`):
- `on_connect(session_id)`
- `on_input(session_id, text)`
- `on_disconnect(session_id)` — a chain of independently protected cleanup steps
- `on_gmcp(session_id, package, data)` — dispatched by package name
- `on_compute_result(id, ok, value, err, meta)` — one per job that was accepted
- `on_timer(id)` — fired by the Tokio timer system
- `on_shutdown()` — last dispatch before the VM stops on a clean shutdown; the driver waits for it
- `on_load(module_name)` / `on_unload(module_name)` — hot-reload hooks

#### Object Hierarchy
All MUD objects inherit from a shared `Object` base class (`mudlib/lib/object.lua`) which provides:
- `resolve()` — lfun pattern (string or function → string)
- State access — `get_state(key)`, `set_state(key, value)` wrapping driver efuns
- Common fields — `id`, `short`, `description`

The full inheritance tree:

```
Object (mudlib/lib/object.lua)
├── Room   (mudlib/lib/room.lua)
├── Item   (mudlib/lib/item.lua)
└── Mobile (mudlib/lib/mobile.lua)
    └── Player (mudlib/lib/player.lua)
```

Item *roles* are components rather than subclasses. `Weapon{...}` and
`Armor{...}` are archetypes that build an `Item` carrying `item.weapon` /
`item.armour`; behaviour lives in the matching system module. See
[Object Hierarchy](./lua-api/object-hierarchy.md).

#### Daemons
Services and singletons are managed via a global `DAEMON` table for easy access across both `mudlib` and `game` layers (e.g., `DAEMON.world`, `DAEMON.journal`, `DAEMON.ticker`, `DAEMON.event`, `DAEMON.cache`, `DAEMON.trait`, `DAEMON.effect`, `DAEMON.combat`). See [Daemons](./lua-api/daemons.md) for the full list.

## Data Flow

### New Connection
```
TCP accept
  → TelnetListener.accept()
  → TelnetConnection (writer)
  → Session::new() → SessionHandler::connect()
  → send_initial_negotiations() (IAC offers)
  → LuaCommand::OnConnect → ScriptEngine thread
  → Lua: on_connect(session_id)
```

### Player Input
```
TCP read
  → TelnetParser::feed_bytes()
  → TelnetEvent::Data / TelnetEvent::Negotiate / TelnetEvent::Subnegotiation
  → Data: TelnetCodec::decode_line()
  → LuaCommand::OnInput → ScriptEngine thread
  → Lua: on_input(session_id, text)
```

### Lua Efun Call
```
Lua: send(session_id, "Hello!")
  → Efun closure (Rust)
  → SessionHandler::read().get(id)
  → session.output_tx.send(SessionOutput::Text)
  → Connection task → TelnetCodec::encode_text()
  → TCP write
```

## Concurrency Model

```
┌─────────────────────────────────┐
│  Tokio async runtime            │
│                                 │
│  Connection tasks (async)       │
│  ┌──────────┐  ┌──────────┐    │
│  │ conn 1   │  │ conn 2   │    │
│  └──────────┘  └──────────┘    │
│         │            │          │
│         ▼            ▼          │
│  LuaCommand (UnboundedSender)  │
└────────────────┬────────────────┘
                 │ channel
┌────────────────▼────────────────┐        ┌──────────────────────────┐
│  Lua thread (dedicated OS thread)│  pipe  │  oxigeon-compute (process)│
│                                 │◄──────►│  a LuaJIT VM, no efuns    │
│  ScriptEngine (the Lua VM)      │        │  started only when        │
│  Processes commands sequentially│        │  [compute] enabled = true │
│  No async — blocking recv()     │        └──────────────────────────┘
└─────────────────────────────────┘
```

The Lua VM runs on **one dedicated thread**. All events are processed sequentially — no concurrency issues within Lua. Each connection task sends events via an unbounded channel.

Two things qualify that, both opt-in:

- **A dispatch can be suspended.** On a Lua 5.5 build with
  `[servers.debug] stop_the_world = false`, a breakpoint parks that one command
  as a coroutine and the loop carries on serving everyone else. Sequential
  becomes *interleaved*, which is why module-level guards in the mudlib are keyed
  per entity rather than per process — see `tests/interleaving.rs`.
- **Compute runs elsewhere.** `compute()` hands a job to an `oxigeon-compute`
  child process with its own LuaJIT VM and no efuns at all, and the answer comes
  back through `on_compute_result`. It is a separate binary because it links a
  different Lua from the server; see
  [Compute — Off-Thread Lua](./lua-api/compute.md).

## Database

Diesel 2.x with SQLite (default) or PostgreSQL (future). The `AnyPool` enum wraps the backend selection at runtime based on `driver.toml`.

Migrations are embedded at compile time via `embed_migrations!("migrations")` and applied on startup.

## Hot-Reload

```
Reload request → LuaCommand::Reload { module_name }
  → ScriptEngine thread receives it
  → on_unload(module_name) called
  → package.loaded[module_name] = nil
  → File read → lua.load() → pcall()
  → if ok: package.loaded[module_name] = new_module
  → on_load(module_name) called
```

If the reload fails (Lua syntax error), the old version remains active.

## Timer System

Timers are backed by Tokio async tasks — no heartbeat polling.

```
Lua: DAEMON.ticker.after(10, "puzzle.reset", fn)
  → schedule_timer("puzzle.reset", 10) efun
  → Rust: tokio::spawn(async { sleep(10s).await; send TimerFired })
  →   ... 10 seconds pass (zero CPU) ...
  → LuaCommand::TimerFired { id: "puzzle.reset" }
  → ScriptEngine thread: on_timer("puzzle.reset")
  → DAEMON.ticker.fire("puzzle.reset") → runs callback
```

Repeating timers use `tokio::time::interval`. Cancellation uses `AbortHandle::abort()`.
