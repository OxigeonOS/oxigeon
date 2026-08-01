# Changelog

## Phase 2: Game World

### Features

#### Object System
- Base `Object` class (`game/lib/object.lua`) — shared fields (`id`, `short`, `description`), `resolve()` (lfun pattern), state access methods
- `Room` inherits from `Object` via metatable chain — exits, contents, actions, items, appearance rendering
- Callable properties (lfun pattern) — any property can be a string or a function returning a string
- `resolve()` returns `<invalid lfun return>` for non-string function returns

#### World Engine
- Two-layer Lua architecture: `mudlib/` (system) + `game/` (content)
- DAEMON service registry (DAEMON global table)
- Data-oriented room definitions — area files are pure data tables with logic separated
- `ROOM_D.from_data()` — creates Room objects from data tables with field mapping and validation
- `ROOM_D.load_area()` — processes area data arrays, extracts `_meta`, registers with world_d
- `ROOM_D.merge()` — combines multiple data arrays for multi-file areas
- Builder pattern (ROOM_D) preserved for dynamic/programmatic room generation
- Area metadata (`_meta`) — stored in area files, queryable via `DAEMON.world.get_area_meta()`
- Multi-file areas — large areas split across sub-files, assembled via `ROOM_D.merge()`
- Virtual room providers — register generators by prefix for infinite/procedural spaces
- Virtual room caching and eviction (`evict_virtual`)
- World daemon (world_d) — room registry with virtual fallback, character locations, movement
- Room actions (add_action) — room-scoped custom commands
- Layered command dispatch: room actions → system commands
- Movement library with room-scoped messaging
- CHARACTER_D — in-memory character state cache with DB persistence

#### Object State
- In-memory key/value state store scoped by object ID (rooms, items, mobs)
- Driver-side efuns: `set_object_state()`, `get_object_state()`, `get_all_object_state()`, `clear_object_state()`
- Survives hot-reloads (Lua VM globals), cleared on restart
- `Object:get_state(key)` / `Object:set_state(key, value)` convenience methods

#### Timer System (TICKER_D)
- Tokio-backed async timers — zero polling, each timer sleeps independently
- `schedule_timer(id, delay)` efun — one-shot timer via `tokio::spawn`
- `schedule_repeating(id, interval)` efun — repeating timer via `tokio::time::interval`
- `cancel_timer(id)` efun — immediate cancellation via `AbortHandle`
- `LuaCommand::TimerFired` — engine dispatches `on_timer(id)` to Lua
- `DAEMON.ticker.after(delay, id, fn)` — one-shot with Lua callback
- `DAEMON.ticker.every(interval, id, fn)` — repeating with Lua callback
- `DAEMON.ticker.remove(id)` — cancel timer and callback
- Input validation, pcall-wrapped callbacks, journal_d error logging

#### Event System (EVENT_D)
- Godot-style signals — named event channels with subscribe/emit
- `DAEMON.event.on(event, id, fn, priority?)` — subscribe with optional priority
- `DAEMON.event.off(event, id)` / `off_all(event)` / `off_by_prefix(prefix)` — flexible unsubscribe
- `DAEMON.event.emit(event, data)` — synchronous dispatch in priority order
- `DAEMON.event.defer(event, data, delay)` — deferred emit via TICKER_D
- pcall-wrapped handlers, sorted listener cache, full introspection API

#### Efuns (new in Phase 2)
- `save_character_data()`, `load_character_data()` — character JSON persistence
- `set_object_state()`, `get_object_state()`, `get_all_object_state()`, `clear_object_state()`
- `schedule_timer()`, `schedule_repeating()`, `cancel_timer()`
- New config keys: `game.command_paths`, `game.start_room`, `game.game_path`

#### Observability
- Structured error logging via journal_d for all critical operations
- `pcall`-wrapped cleanup chains (disconnect, init loading)
- Input validation in all daemons with logged warnings

#### Tests
- 147 tests total — all passing

---

## v0.1.0 (Current)

**Initial release of Oxigeon**

### Features

#### Network
- Telnet server (RFC 854) with full IAC state machine
- RFC 1143 Q Method option negotiation (prevents infinite loops)
- Initial negotiation offers: SGA, GMCP, MCCP2, TTYPE, NAWS
- ECHO option (password masking) — `start_echo()`/`stop_echo()`; sends `IAC WILL ECHO` / `IAC WONT ECHO`
- GMCP support (option 201) — `send_gmcp()` efun, `on_gmcp()` event hook, `Core.Hello` on connect
- CR/LF normalization on send and receive
- Full bidirectional relay loop: TCP read → Telnet parse → Lua on_input; Lua send → session channel → TCP write

#### Sessions
- UUID-based session identifiers
- Full session lifecycle: `Connected → Authenticating → Authenticated → Playing`
- `authenticate_session(session_id, account_id)` — marks session authenticated, enforces multisession policy
- `enter_game_session(session_id, account_id, character_id)` — marks session as playing with character
- Multisession policy: `single`, `shared_character`, `multi_character`, `full_multi`
- Max connections limit; `"Server is full"` message on reject

#### Database
- SQLite via Diesel 2.x + r2d2 connection pool
- Automatic migrations on startup (embedded via `embed_migrations!`)
- Account model with Argon2id password hashing
- Character model with per-account limit enforcement and globally unique name constraint

#### Scripting
- LuaJIT (5.1 API) on a dedicated OS thread
- Full sandbox: `io`, `os`, `debug` modules removed; binary bytecode loading blocked
- `require()` jailed to mudlib directory — sets `package.path` before loading `init.lua`
  (Windows UNC `\\?\` extended path prefix stripped to avoid Lua `?` substitution conflicts)
- Path traversal prevention (`../` jailing) for all file efuns
- Hot-reload: `reload(module_name)` from Lua, `on_load`/`on_unload` hooks
- `LuaCommand` channel pre-created so `cmd_tx` is available in `EfunContext` for Lua-triggered reloads
- Persistent storage across reloads: `set_persistent()`/`get_persistent()`

#### Architecture
- Three-layer design: **Core** (driver internals) → **Domain** (creator-facing models) → **Mudlib** (Lua game)
- `src/domain/` (previously `src/middle/`) — renamed to reflect DDD terminology
- `AccountStore` and `CharacterStore` traits defined in `src/domain/traits.rs`
- `DieselAccountStore` and `DieselCharacterStore` implement the respective traits

#### Efuns (complete list)
**Output:** `send()`, `send_prompt()`, `broadcast()`, `disconnect()`

**Telnet:** `send_gmcp()`, `start_echo()`, `stop_echo()`

**Session:** `this_session()`, `get_session()`, `all_sessions()`, `set_session_state()`,
`authenticate_session()`, `enter_game_session()`

**Account:** `authenticate()`, `create_account()`, `get_account()`

**Character:** `create_character()`, `get_characters()`, `get_character()`

**Utility:** `log()`, `time()`, `config()`

**File I/O (mudlib-jailed):** `read_file()`, `write_file()`, `append_file()`, `file_exists()`,
`list_dir()`, `delete_file()`, `os_time()`, `os_clock()`, `os_date()`

**Hot-reload:** `reload()`, `set_persistent()`, `get_persistent()`

#### Mudlib (Starter)
- `mudlib/init.lua` — event handlers (`on_connect`, `on_input`, `on_disconnect`, `on_gmcp`,
  `on_load`, `on_unload`), command dispatcher (`help`, `who`, `time`, `say`, `quit`)
- `mudlib/login.lua` — full login/registration flow with ECHO masking; calls
  `authenticate_session()` + `enter_game_session()` for proper session state transitions
- `mudlib/lib/strings.lua` — string utilities
- `mudlib/lib/tables.lua` — table utilities

#### Documentation
- mdbook-based documentation served at `docs/` (`mdbook serve docs/ --port 3000`)
- Lua API reference: efuns, events, sandboxing, file access
- Architecture overview with layer diagram and concurrency model
- Configuration reference (driver.toml and server.toml)
- Protocol documentation: Telnet, GMCP, MCCP, ECHO
- Rust API reference: domain models, swappable traits, extension guide

### Tests
- **74 tests total — all passing**
- 43 unit tests:
  - Telnet parser FSM (13 tests)
  - Option negotiation Q Method (8 tests)
  - Codec encode/decode (8 tests)
  - Session handler + multisession policy (6 tests)
  - Lua sandbox: io/os/debug blocked, path traversal prevented, bytecode rejected (8 tests)
- 31 integration tests:
  - Account store CRUD + Argon2 authentication (9 tests)
  - Character store CRUD + per-account limits (6 tests)
  - Hot-reload: module update, error resilience, multiple cycles, event dispatch (4 tests)
  - Sandbox: io module, os.execute, loadfile, dofile, debug module, pcall, coroutine, table (12 tests)

### Known Limitations / Upcoming
- MCCP2 zlib compression negotiated but not yet applied to the write stream
- PostgreSQL backend declared in config but not fully wired (requires libpq)
- WebSocket and TLS listeners not yet implemented
- `set_persistent()`/`get_persistent()` live in VM memory only — not persisted across server restarts
- Object state (`set_object_state`) lives in VM memory only — not persisted across server restarts
