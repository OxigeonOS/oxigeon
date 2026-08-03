# Changelog

## Phase 3: Hardening, Performance & Persistence

### Security

- **The sandbox is now applied to the running VM.** `apply_sandbox` was well tested and never called: `io.open`, `io.popen`, `os.execute`, `os.exit` and `package.loadlib` were all reachable from mudlib code, and `io.open` read files outside the mudlib jail. The dead `create_sandboxed_env` was deleted so there is one boundary, not two.
- `jit`, `package.cpath` and the native module loaders removed; `load`/`loadstring` now refuse binary bytecode.
- **Argon2 moved off the game thread.** Every login froze the whole game for ~370 ms, pre-authentication, so spamming attempts was a trivial denial of service. `authenticate` and `create_account` are now asynchronous and answer at `on_auth_result`. Added a bounded queue and a per-address lockout after 5 failed attempts.
- **Timer-dispatched code has an explicit identity.** Gated efuns called from a daemon tick used to fail closed, silently. Engine-internal dispatch now declares itself; `tests/timer_identity.rs` pins that a player session is still refused.

### Correctness

- **`lua_to_json` had five silent failures on the `save_character_data` path**: a table that is both a list and a map lost every string key, cycles exhausted the Rust stack and killed the process, NaN and infinity became `0`, functions became `null`, and unusual keys vanished. All now raise, naming the offending field.
- **Output is no longer silently dropped.** Ten `try_send` sites against a 64-slot channel lost text with no log, counter or marker. Drops are counted, logged, surfaced in `server_info()`, and the player sees a truncation notice.
- **Lock poisoning is survivable.** 42 `.unwrap()` calls on lock acquisition would have turned one panic into a permanently dead game. `read_recover`/`write_recover`/`lock_recover` recover and report.
- **The stat whitelist in `Mobile:new` is gone.** It rebuilt `obj.stats` from a fixed list of nine keys on every load, so any other stat was silently dropped even though `to_save` had faithfully written it — a trait named `wisdom` would have vanished on every login.
- **`Char.Status` reported 0 experience and 0 gold for every character, always**, because `gmcp_d` read `player.stats.xp` and `player.stats.gold`, which have never existed. `death_d`'s XP-loss-on-death was dead code for the same reason.
- **`ticker_d.remove_by_prefix` now exists.** `character_d.unload` had called it since it was written; the function was never there, so the call raised into a pcall that logged at debug level and every per-player timer leaked.
- **`send_lines` accepts a table again.** Both spellings were in use across the mudlib and the table form printed `table: 0x...` to the player — which is what `death_d` had been announcing deaths with.
- **A clean shutdown now saves.** `LuaCommand::Shutdown` broke the engine loop without dispatching anything to Lua, and the Ctrl+C path never joined the Lua thread — `Drop for ScriptEngine` only sent the command and returned. Two failures compounding: nothing asked the mudlib to save, and nothing would have waited if it had. Since `CHARACTER_D` is a write-back cache flushed by the autosave ticker, **every clean restart discarded up to `autosave_seconds` (default 300) of every online player's progress.** The engine now dispatches [`on_shutdown`](./lua-api/events.md) under its own identity before breaking, and the driver waits for the thread — bounded by `game.shutdown_timeout_seconds` (default 30) so a mudlib that wedges in the hook cannot hang the process. `tests/clean_shutdown.rs` pins all of it, ending with a save through the real mudlib.
- Fixed the file jail refusing every legitimate read on a relative mudlib root — `audit_d` could not load its watch list and said nothing.
- SQLite now runs in WAL mode with `synchronous = NORMAL`, removing an fsync from the Lua thread on every write.

### Performance

- **`lua_instruction_limit` is enforced, and on by default.** It was parsed and never read. Enforcing it disables the LuaJIT compiler — LuaJIT dispatches no debug hooks from inside a compiled trace — but measured through the real mudlib that costs 2-7% on commands, because the compiler is worth ~1.00x on command dispatch and 2.10x only on tight arithmetic. See [Performance & the JIT Trade-off](./lua-api/performance.md).
- `lua_memory_mb` is enforced too; it turned out to work on this build after all.
- `cargo bench` (criterion) measures the real mudlib and refuses to run if its own control shows the JIT toggle is broken.

### Features

- **[Traits](./lua-api/traits.md)** — character attributes that are *computed* rather than stored: derived from other traits (Willpower from Wisdom), filtered through active effects, and regenerating from a timestamp rather than a timer. Deliberately no `mod` field on a trait: Evennia's Traits contrib stores one, which makes a buff a write to the thing it buffs, so any path that misses the matching unapply leaves the character permanently wrong. Here nothing is stored, so there is nothing to unapply. Dependencies are declared and *enforced* — reading an undeclared one raises — which is what lets `seal()` report a cycle as a path rather than a shrug.
- **[Effects](./lua-api/effects.md)** — buffs and debuffs as an event pipeline. `run(entity, "damage_taken", ev)` passes the numbers through every effect that cares. Ordering is by declared **phase**, not registration order, so "-15% damage" and "-5 flat damage" on a 30-point hit give 20 rather than depending on which buff landed first. Passive stat modifiers are the same pipeline under the hook family `trait:<id>`, so a +2 ring and a -15% buff are authored identically. Definitions hold functions and live in code; instances are nine plain fields and live in the cache.
- **[State Cache](./lua-api/state-cache.md)** — the write-behind tier `task_list.md` item 3 asked for, plus `DAEMON.cooldown`. Three tiers chosen by how much you would mind losing the data. Measured: 10 changes to one player cost 1.20 ms written through and 0.15 ms written behind; 1000 changes, 1077 ms against 2.3 ms. A flush is one `db_put` of the whole scope rather than a merge patch, because RFC 7396 expresses deletion as a JSON null and a Lua table cannot hold one — a merge flush could never remove an expired effect. Values are checked against `lua_to_json`'s rules when written rather than when flushed, so a bad value is refused at the call site instead of raising inside `on_shutdown`.
- **[Creatures & Combat](./lua-api/combat.md)** — `mob_d` and `combat_d`: templates, instances, room occupancy, respawn, and a minimal round-based fight so the pipeline is visible in numbers a player sees. Combat state is memory-tier and never written.
- **[Compute Bridge](./lua-api/compute.md)** — `compute()` runs a long computation on a worker thread with its own LuaJIT VM and answers at `on_compute_result`. Worker VMs have no efuns at all.
- **[Document Store](./lua-api/document-store.md)** — twelve `db_*` efuns over a generic JSON table. Persisting a new type needs no Rust, no migration and no rebuild, which matters because `embed_migrations!` is compile-time and the game layer can never ship schema.

### Fixed

- `mudstatus` printed "0s" uptime: it read `info.uptime_seconds`, but the field is `uptime_secs` — and `types/oxigeon.lua` declared the wrong name, which is why. The stub also declared two fields that never existed.
- `save_character_data`/`load_character_data` were annotated as taking and returning strings; they take and return tables.
- The "Persistent Store" annotation claimed persistence across restarts. It is a Lua table that survives hot reload only.


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
- Full sandbox: `io` and `debug` removed, `os` reduced to its clock functions, binary bytecode loading blocked
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

**Account:** `authenticate()`, `create_account()` (both asynchronous — they answer at `on_auth_result`), `get_account()`

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
