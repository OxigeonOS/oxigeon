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
- **[Document Store — Persisting Anything](./document-store.md)** — `db_put()`, `db_find()`, `db_update()`, `db_incr()`: persist any type with no Rust and no migration
- **[Efuns — Driver Functions](./efuns.md)** — `send()`, `send_prompt()`, `broadcast()`, `authenticate_session()`, `set_object_state()`, `schedule_timer()`, `reload()`, etc.
- **[Event Hooks](./events.md)** — `on_connect`, `on_input`, `on_disconnect`, `on_gmcp`, `on_timer`, `on_shutdown`, `on_load`, `on_unload`
- **[State Cache](./state-cache.md)** — memory / write-behind / write-through, and cooldowns
- **[Traits](./traits.md)** — character attributes, derived values, regeneration
- **[Effects](./effects.md)** — buffs, debuffs, and the event pipeline
- **[Creatures & Combat](./combat.md)** — mobs, spawning, rounds
- **[Debugging & Tracing](./debugging.md)** — the debug adapter and execution tracing
- **[Observability & Logging](./observability.md)** — journal_d, audit_d, server info
- **[Permissions & Roles](./permissions.md)** — RBAC system, role management, permission checks
- **[File & System Access](./file-access.md)** — `read_file()`, `write_file()`, `list_dir()`, `os_time()`, `os_date()`, etc.
- **[Sandboxing & Security](./sandboxing.md)** — What is and isn't available, and why.
- **[Performance & the JIT Trade-off](./performance.md)** — What the compiler is worth, measured, and how to re-measure it
- **[Compute — Off-Thread Lua](./compute.md)** — `compute()`: run a long computation on a worker thread without freezing the game

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
| `os` | ⚠️ Clocks only | `os.time`, `os.date`, `os.clock`, `os.difftime` are kept; everything else is removed. `os_time()`/`os_clock()`/`os_date()` are efun equivalents |
| `debug` | ❌ | Can escape any sandbox. Loaded but hidden from `_G` when the debug adapter is enabled |
| `jit` | ❌ | `jit.on()` would disarm the instruction limit |
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
