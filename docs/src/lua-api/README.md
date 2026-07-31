# Lua API Reference

This section documents all Lua functions (efuns) provided by the Oxigeon driver,
plus the daemon APIs and event hooks available to your mudlib.

Efuns are Rust functions exposed to Lua — they form the bridge between your mudlib and the driver's subsystems.

## Categories

- **[Daemons — Service Layer](./daemons.md)** — `DAEMON.journal`, `DAEMON.audit`, `DAEMON.ticker`, `DAEMON.event`, `DAEMON.prompt`, `DAEMON.world`, `DAEMON.room`, `DAEMON.character`, `DAEMON.codegen`, `DAEMON.olc`
- **[Object Hierarchy](./object-hierarchy.md)** — `Object` → `Room`, `Item` (→ `Weapon`, `Armor`), `Mobile` (→ `Player`): fields, methods, inheritance
- **[World Building — Rooms & Areas](./world-building.md)** — Data-oriented rooms, virtual providers, area metadata, multi-file areas, object state, tickers
- **[Signals & Events (EVENT_D)](./signals.md)** — Godot-style signals: subscribe, emit, priority, deferred events, bulk cleanup
- **[Character Data & Persistence](./character-data.md)** — `CHARACTER_D`, `save_character_data()`, `load_character_data()`
- **[Efuns — Driver Functions](./efuns.md)** — `send()`, `send_prompt()`, `broadcast()`, `authenticate_session()`, `set_object_state()`, `schedule_timer()`, `reload()`, etc.
- **[Event Hooks](./events.md)** — `on_connect`, `on_input`, `on_disconnect`, `on_gmcp`, `on_timer`, `on_load`, `on_unload`
- **[Observability & Logging](./observability.md)** — journald, auditd, server info
- **[Permissions & Roles](./permissions.md)** — RBAC system, role management, permission checks
- **[File & System Access](./file-access.md)** — `read_file()`, `write_file()`, `list_dir()`, `os_time()`, `os_date()`, etc.
- **[Sandboxing & Security](./sandboxing.md)** — What is and isn't available, and why.

## Lua Version

Oxigeon uses **LuaJIT (API compatible with Lua 5.1)**. This means:

- Lua 5.1 standard library (string, table, math, coroutine)
- `setfenv`/`getfenv` available (removed in Lua 5.2+)
- **No** Lua 5.2+ features: `goto`, bitwise operators, integer types, UTF-8 library
- **No** Lua 5.3+ features: integer division `//`, bitwise `&|~^`, etc.
- JIT compilation for fast Lua code

## Available Standard Libraries

| Library | Available | Notes |
|---------|-----------|-------|
| `string` | ✅ | All functions |
| `table` | ✅ | All functions |
| `math` | ✅ | All functions |
| `coroutine` | ✅ | All functions |
| `io` | ❌ | Use `read_file()`, `write_file()`, `list_dir()` instead |
| `os` | ❌ | Use `os_time()`, `os_clock()`, `os_date()` instead |
| `debug` | ❌ | Disabled for security |
| `package.loadlib` | ❌ | No C extensions |
| `require` | ✅ (jailed) | Limited to mudlib and game directories |

## Object Hierarchy

All MUD objects inherit from a shared base class:

```
Object (mudlib/lib/object.lua)
├── Room   (game/lib/room.lua)
├── Item   (mudlib/lib/item.lua)
│   ├── Weapon (mudlib/lib/weapon.lua)
│   └── Armor  (mudlib/lib/armor.lua)
└── Mobile (mudlib/lib/mobile.lua)
    └── Player (mudlib/lib/player.lua)
```

See [Object Hierarchy](./object-hierarchy.md) for the full API reference.
