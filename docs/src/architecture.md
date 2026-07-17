# Architecture Overview

Oxigeon is structured in three layers. Each layer has a specific responsibility and knows nothing about the layers above it.

## Layer Diagram

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 3: Mudlib (Lua)                                      │
│                                                             │
│  mudlib/init.lua        → on_connect, on_input, on_disconnect│
│  mudlib/login.lua       → login/registration flow           │
│  mudlib/lib/*.lua       → shared utilities                  │
│  mudlib/areas/*.lua     → game world                        │
│  mudlib/systems/*.lua   → game mechanics (combat, etc.)     │
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
Your game. The driver only calls three things from Lua:
- `on_connect(session_id)`
- `on_input(session_id, text)`
- `on_disconnect(session_id)`

Everything else is up to you.

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
