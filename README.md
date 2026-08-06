# Oxigeon

**A modern MUD driver** written in Rust with Lua scripting.

Oxigeon provides the infrastructure — networking, Telnet protocol, session management, database — so you can focus on building your game in Lua.

## Quick Start

```bash
# Build (the first build compiles Lua from source — takes a few minutes)
cargo build --release

# Optional: the compute worker, only needed with [compute] enabled = true.
# A separate binary because it links LuaJIT while the server links Lua 5.5.
cargo build --release -p oxigeon-compute

# Run (uses config/driver.toml and config/server.toml)
cargo run

# Connect from Mudlet or any telnet client
telnet localhost 4000

# Serve documentation locally on port 3000
mdbook serve docs/ --port 3000
```

## Project Layout

```
oxigeon/
├── config/
│   ├── driver.toml        # Infrastructure: servers, database, logging
│   └── server.toml        # Game: name, accounts, session limits
│
├── mudlib/                # Your game — written in Lua
│   ├── init.lua           # Entry point: on_connect, on_input, on_disconnect
│   ├── login.lua          # Login/registration flow
│   └── lib/
│       ├── strings.lua    # String utilities
│       └── tables.lua     # Table utilities
│
├── migrations/            # Diesel database migrations
│
├── docs/                  # mdbook documentation
│   ├── book.toml
│   └── src/
│       ├── SUMMARY.md
│       └── ...
│
└── src/                   # Rust driver source
    ├── main.rs            # Entry point
    ├── driver.rs          # Coordinator
    ├── error.rs           # Error types
    ├── config/            # Config parsing
    ├── core/
    │   ├── network/
    │   │   └── telnet/    # Telnet state machine, Q Method, codec
    │   ├── session/       # Session registry + multisession policy
    │   └── scripting/     # Lua engine, efuns, sandbox
    └── domain/
        ├── db/            # Diesel pool + schema
        ├── models/        # Account + Character models
        └── traits.rs      # Swappable component traits
```

## Architecture

Lua does the game. Rust does the plumbing.

```
Layer 3: Mudlib (Lua)   — game logic, rooms, commands
Layer 2: Domain (Rust)  — models, traits, custom efuns (creators touch this)
Layer 1: Core (Rust)    — networking, sessions, VM (driver internals)
```

## Supported Protocols

- **Telnet** (RFC 854) — full IAC state machine, RFC 1143 Q Method
- **GMCP** (option 201) — `send_gmcp()`/`on_gmcp()` for rich data
- **ECHO** (option 1) — `start_echo()`/`stop_echo()` for password masking
- **MCCP2** (option 86) — negotiation done, compression stream coming soon

## Testing

```bash
# Unit tests (fast, in-process)
cargo test --lib

# All tests including integration tests (account store, character store, sandbox, hot-reload)
# NOTE: use --test-threads=1 to avoid argon2 pool exhaustion in debug builds
cargo test -- --test-threads=1
```

Tests cover:
- **Telnet parser FSM** — IAC, WILL/WONT/DO/DONT, SB/SE subnegotiation
- **RFC 1143 Q Method** — all 24 state transitions per direction
- **CR/LF codec** — encoding, decoding, IAC escaping, GMCP framing
- **Session handler** — connect/disconnect, multisession modes, max-connections
- **Lua sandbox** — io/os/debug blocked, path traversal prevention, binary bytecode rejection
- **Account store** — CRUD, argon2 authentication, duplicate rejection, password length
- **Character store** — CRUD, per-account limits, name uniqueness
- **Hot-reload** — module reload, error resilience, multiple reload cycles
- **Event dispatch** — on_connect, on_input, on_disconnect

> **Performance note**: Argon2 hashing in `--debug` mode takes ~3-4 seconds per hash.
> Integration tests run significantly faster with `--release`. The `--test-threads=1`
> flag prevents parallel pool exhaustion while waiting for argon2 operations.

## Documentation

```bash
mdbook serve docs/ --port 3000
# Open: http://localhost:3000
```

Or generate Rust API docs:
```bash
cargo doc --no-deps --open
```

## Configuration

| File | Purpose |
|------|---------|
| `config/driver.toml` | Servers, database, logging |
| `config/server.toml` | Game name, accounts, Lua limits |

## License

MIT
