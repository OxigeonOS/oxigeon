# Changelog

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
- `mudlib/lib/utils.lua` — string/table utilities

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
- `delay(seconds, callback)` efun not yet implemented
- `set_persistent()`/`get_persistent()` live in VM memory only — not persisted across server restarts
