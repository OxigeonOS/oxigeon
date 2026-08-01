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
│  │                        event_d, prompt_d                 │
│  └── lib/               → object, item, weapon, armor,      │
│                           mobile, player, commands, utils    │
│                                                             │
│  game/                  → Game content layer                │
│  ├── init.lua           → load game daemons and areas       │
│  ├── daemons/           → world_d, room_d, character_d,     │
│  │                        codegen_d, olc_d                  │
│  ├── lib/               → room                             │
│  └── areas/             → game world definitions (data)     │
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
│  src/core/scripting/    → LuaJIT VM, efuns, sandbox         │
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
Your game. It is divided into two parts:
- **`mudlib/`**: The core system layer (login, command dispatch, base daemons).
- **`game/`**: The game content layer (rooms, areas, game-specific daemons like `ROOM_D` and `CHARACTER_D`).

The driver calls these global Lua functions (defined in `mudlib/init.lua`):
- `on_connect(session_id)`
- `on_input(session_id, text)`
- `on_disconnect(session_id)`
- `on_gmcp(session_id, package, data)`
- `on_timer(id)` — fired by the Tokio timer system
- `on_load(module_name)` / `on_unload(module_name)` — hot-reload hooks

#### Object Hierarchy
All MUD objects inherit from a shared `Object` base class (`mudlib/lib/object.lua`) which provides:
- `resolve()` — lfun pattern (string or function → string)
- State access — `get_state(key)`, `set_state(key, value)` wrapping driver efuns
- Common fields — `id`, `short`, `description`

The full inheritance tree:

```
Object (mudlib/lib/object.lua)
├── Room   (game/lib/room.lua)
├── Item   (mudlib/lib/item.lua)
│   ├── Weapon (mudlib/lib/weapon.lua)
│   └── Armor  (mudlib/lib/armor.lua)
└── Mobile (mudlib/lib/mobile.lua)
    └── Player (mudlib/lib/player.lua)
```

#### Daemons
Services and singletons are managed via a global `DAEMON` table for easy access across both `mudlib` and `game` layers (e.g., `DAEMON.world`, `DAEMON.journal`, `DAEMON.ticker`, `DAEMON.event`, `DAEMON.prompt`, `DAEMON.codegen`, `DAEMON.olc`).

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
┌────────────────▼────────────────┐
│  Lua thread (dedicated OS thread)│
│                                 │
│  ScriptEngine (LuaJIT VM)       │
│  Processes commands sequentially│
│  No async — blocking recv()     │
└─────────────────────────────────┘
```

The Lua VM runs on **one dedicated thread**. All events are processed sequentially — no concurrency issues within Lua. Each connection task sends events via an unbounded channel.

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
