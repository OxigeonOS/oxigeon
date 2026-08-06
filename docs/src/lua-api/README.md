# Lua API Reference

This section documents all Lua functions (efuns) provided by the Oxigeon driver,
plus the daemon APIs and event hooks available to your mudlib.

Efuns are Rust functions exposed to Lua — they form the bridge between your mudlib and the driver's subsystems.

## Categories

- **[Daemons — Service Layer](./daemons.md)** — `DAEMON.journal`, `DAEMON.audit`, `DAEMON.ticker`, `DAEMON.event`, `DAEMON.prompt`, `DAEMON.world`, `DAEMON.room`, `DAEMON.character`, `DAEMON.codegen`, `DAEMON.olc`
- **[Object Hierarchy](./object-hierarchy.md)** — `Object` → `Room`, `Item`, `Mobile` (→ `Player`): fields, methods, inheritance — plus the archetype/component/system model that replaced the `Weapon` and `Armor` subclasses
- **[World Building — Rooms & Areas](./world-building.md)** — Data-oriented rooms, virtual providers, area metadata, multi-file areas, object state, tickers
- **[Signals & Events (EVENT_D)](./signals.md)** — Godot-style signals: subscribe, emit, priority, deferred events, bulk cleanup
- **[Character Data & Persistence](./character-data.md)** — `CHARACTER_D`, `save_character_data()`, `load_character_data()`
- **[Document Store — Persisting Anything](./document-store.md)** — `db_put()`, `db_find()`, `db_update()`, `db_incr()`: persist any type with no Rust and no migration
- **[Efuns — Driver Functions](./efuns.md)** — `send()`, `send_prompt()`, `broadcast()`, `authenticate_session()`, `set_object_state()`, `schedule_timer()`, `reload()`, etc.
- **[Event Hooks](./events.md)** — `on_connect`, `on_input`, `on_disconnect`, `on_gmcp`, `on_timer`, `on_shutdown`, `on_load`, `on_unload`
- **[State Cache](./state-cache.md)** — memory / write-behind / write-through, and cooldowns
- **[Items, Equipment & Containers](./items.md)** — templates and instances, `get`/`drop`/`put`, wearing things, and what that does to your numbers
- **[Shops & the Economy](./shops.md)** — stock, prices, restocking, a ledger
- **[Traits](./traits.md)** — any numeric datum on any entity; presence decided by storage
- **[Effects](./effects.md)** — buffs, debuffs, and the event pipeline
- **[Abilities](./abilities.md)** — cost, cooldown, target, damage and effects as a data bag; a pared-down GAS for a MUD
- **[Creatures & Combat](./combat.md)** — mobs, spawning, rounds
- **[Debugging & Tracing](./debugging.md)** — the debug adapter and execution tracing
- **[Interface](./interface.md)** — prompt templates, colour, the pager, channels, snooping, NAWS
- **[Observability & Logging](./observability.md)** — journal_d, audit_d, server info, the Lua heap
- **[OLC — Building In-Game](./olc.md)** — `olc`, `dig`, `codegen_d`, and the build → file → reload round trip
- **[Prototypes](./prototypes.md)** — a named, inheritable skeleton a template names and overrides; resolved at area load
- **[Permissions & Roles](./permissions.md)** — RBAC system, role management, permission checks
- **[File & System Access](./file-access.md)** — `read_file()`, `write_file()`, `list_dir()`, `os_time()`, `os_date()`, etc.
- **[Sandboxing & Security](./sandboxing.md)** — What is and isn't available, and why.
- **[Performance & the JIT Trade-off](./performance.md)** — What the compiler is worth, measured, and how to re-measure it
- **[Compute — Off-Thread Lua](./compute.md)** — `compute()`: run a long computation in a worker process without freezing the game

## Lua Version

**Lua 5.5 by default**, with LuaJIT (Lua 5.1) available as a build-time
alternative — `cargo build --no-default-features --features luajit`. Which one
you get is a property of the *build*, not of your mudlib, so game code that has
to run on both should stay inside the common subset.

The default is 5.5 because of the debugger, not speed: on LuaJIT a breakpoint
can only freeze the whole server, while on 5.5 it can suspend one player's
command and let everyone else carry on. See
[Debugging](./debugging.md#-what-a-breakpoint-costs) and
[Performance](./performance.md#luajit-against-lua-55), which has the numbers.

What differs, if you are writing for both:

| | Lua 5.5 (default) | LuaJIT |
|---|---|---|
| Integers | a distinct subtype: `3` and `3.0` are different | one number type; `3` *is* `3.0` |
| `1/2` | `0.5`; `//` is integer division | `0.5`; no `//` |
| `goto`, `&` `\|` `~`, `<<` `>>` | yes | no |
| `utf8` library | yes | no |
| `setfenv` / `getfenv` / `loadstring` | gone — use `load`'s 4th argument | yes |
| `#` on a table with holes | undefined either way — do not rely on it | same |
| Errors from a hook | uncatchable, so the instruction budget cannot be `pcall`ed away | catchable |

The integer/float split is the one that bites. `tostring(3/1)` is `"3.0"` on 5.5
and `"3"` on LuaJIT, and `string.format("%d", x)` **raises** on 5.5 if `x` has a
fractional part rather than silently truncating. Floor before formatting, or use
`require('lib.strings').number(n)`, which renders a whole number without a `.0` on
either runtime.

Traits are already careful about this: a trait declares `round`, and anything
displayed goes through it.

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
| `jit` | ❌ | LuaJIT builds only, and removed there: `jit.on()` would disarm the instruction limit |
| `utf8` | ✅ | Lua 5.5 builds only |
| `package.loadlib` | ❌ | No C extensions |
| `require` | ✅ (jailed) | Limited to mudlib and game directories |

## Object Hierarchy

All MUD objects inherit from a shared base class:

```
Object (mudlib/lib/object.lua)
├── Room   (game/lib/room.lua)
├── Item   (mudlib/lib/item.lua)
│       Roles are components, not subclasses: Weapon{} and Armor{}
│       are archetypes that build an Item carrying item.weapon /
│       item.armour. See object-hierarchy.md.
└── Mobile (mudlib/lib/mobile.lua)
    └── Player (mudlib/lib/player.lua)
```

See [Object Hierarchy](./object-hierarchy.md) for the full API reference.
