# Oxigeon

**Oxigeon** is a modern MUD driver written in Rust, using LuaJIT as its game scripting engine.

It provides the infrastructure — networking, protocol handling, sessions, databases — so you can focus on writing the game in Lua.

## Quick Start

```bash
# Build and run
cargo run

# Connect with Mudlet or any telnet client
telnet localhost 4000

# Serve documentation locally
mdbook serve docs/

# Generate Rust API docs
cargo doc --no-deps --open
```

## Design Philosophy

- **Lua does the game. Rust does the plumbing.**
- **Configuration over code** — change behavior via TOML files, not recompilation.
- **Liskov-substitutable components** — swap any subsystem via traits.
- **LuaJIT (5.1 API)** — fast, simple, accessible to non-coder creators.

## Three-Layer Architecture

```
┌────────────────────────────────────────────────────┐
│  Layer 3: Mudlib  (Lua)                            │
│  Your game — rooms, items, NPCs, combat, commands  │
├────────────────────────────────────────────────────┤
│  Layer 2: domain  (Rust — creator-facing)          │
│  Database models, configuration, trait impls       │
├────────────────────────────────────────────────────┤
│  Layer 1: Core  (Rust — framework internals)       │
│  Telnet, sessions, Lua VM, efuns, protocols        │
└────────────────────────────────────────────────────┘
```

See the [Architecture Overview](./architecture.md) for a deep dive.
