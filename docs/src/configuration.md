# Configuration Reference

Oxigeon uses two TOML configuration files. Both are loaded at startup.

## driver.toml

Infrastructure concerns — which servers run, database, logging.

### `[database]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | `"sqlite"` | **`"sqlite"` only** — see below |
| `url` | string | `"oxigeon.db"` | SQLite filename |
| `pool_size` | integer | `5` | Connection pool size |

```toml
[database]
backend = "sqlite"
url = "oxigeon.db"
pool_size = 5
```

> [!WARNING]
> **`postgresql` parses and does not work.** Only the SQLite Diesel feature is
> enabled and the driver calls `get_sqlite()` unconditionally, so selecting
> PostgreSQL logs "PostgreSQL" and then misbehaves. The value is accepted
> because the enum has always had it; it has never had a runtime path.
>
> This is recorded rather than removed because the abstraction it implies —
> `AnyPool`, one connection type behind a trait — is the right shape and is
> worth keeping. What is missing is the second implementation, the migrations
> for it, and a test that runs against it. A backend nobody can test is a
> backend nobody should ship.

### `[servers.telnet]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable the Telnet server |
| `bind` | string | `"0.0.0.0"` | IP address to bind to |
| `port` | integer | `4000` | Port number |

```toml
[servers.telnet]
enabled = true
bind = "0.0.0.0"
port = 4000
```

### `[servers.debug]`

Optional — omit the section entirely to disable, which is the default. Enables
the Lua debug adapter for VS Code. See
[Debugging & Tracing](./lua-api/debugging.md).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `false` | Listen for DAP clients and load the Lua `debug` stdlib |
| `bind` | string | `"127.0.0.1"` | Bind address. **Never expose** — see below |
| `port` | integer | `4711` | Listen port |
| `auto_continue_secs` | integer | `300` | Give up and resume if the client stops responding (`0` = never) |
| `stop_the_world` | bool | `true` | Whether a breakpoint holds the whole VM, or only the dispatch that hit it |
| `trace_capacity` | integer | `5000` | `trace show` ring buffer size, in records |
| `timing_capacity` | integer | `200` | `trace timings` ring buffer size |

```toml
[servers.debug]
enabled = true
bind = "127.0.0.1"
port = 4711
stop_the_world = true
```

> **What a breakpoint costs depends on `stop_the_world`.** Left `true` — the
> default, and all a LuaJIT build can do — hitting one stops the entire VM:
> every player is frozen until you continue, connections stay alive and input
> queues, and repeating timers accumulate and fire as a burst on resume.
>
> Set it `false` on a Lua 5.5 build and a stop suspends only the dispatch that
> hit it; everyone else keeps playing. That is what makes debugging a server
> with people on it possible. Changeable while running with `trace freeze
> on|off`. See [What a breakpoint costs](./lua-api/debugging.md#-what-a-breakpoint-costs).

> **Security.** The adapter grants unauthenticated arbitrary Lua execution in
> the game VM: `evaluate` is a REPL with no login. Enabling it also loads the
> `debug` standard library through mlua's unsafe constructor — the table is
> hidden from `_G` before any mudlib code runs, and `package.loadlib` is closed
> back up, but the VM no longer has mlua's safety guarantees. Leave this
> disabled in production.

### `[logging]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `level` | string | `"info"` | Log level: `trace`, `debug`, `info`, `warn`, `error` |
| `file` | string | (none) | Optional log file path (omit for stdout only) |

```toml
[logging]
level = "info"
# file = "logs/oxigeon.log"
```

`file` appends, creates any parent directory, and writes without ANSI escapes —
colour codes in a log someone will `grep` are noise, and the terminal is not
reading it. A path that cannot be opened is reported on stderr and logging falls
back to stdout, rather than failing silently.

> This key was parsed and then **ignored** for a long time: only `level` was
> read, so setting a path produced no file and no warning. A config key that
> looks like it works and does nothing is worse than one that does not exist.

---

## server.toml

Game-level concerns — name, account policy, session behavior, Lua limits.

### `[game]`

| Key | Type | Description |
|-----|------|-------------|
| `name` | string | Game name (shown in login banner) |
| `mudlib_path` | string | Path to mudlib directory (relative to working dir) |
| `game_path` | string | Path to the game content layer. Default: `./game` |
| `command_paths` | string[] | Subdirectories searched for commands. Default: `["cmds"]` |
| `start_room` | string | Room ID new characters spawn in |
| `area_reset_seconds` | integer | How often areas reset. 0 disables. Default: 900 |
| `autosave_seconds` | integer | How often loaded player data is saved. 0 disables. Default: 300 |
| `shutdown_timeout_seconds` | integer | How long a clean shutdown waits for `on_shutdown`. Default: 30 |
| `cache_flush_seconds` | integer | How often dirty state-cache scopes are considered for writing. 0 disables. Default: 5 |
| `cache_flush_budget` | integer | Scopes one flush tick may write before deferring the rest. Default: 32 |
| `cache_evict_seconds` | integer | Idle eviction for unowned cache scopes. 0 disables. Default: 900 |
| `cooldown_durable_seconds` | integer | Cooldowns at least this long are stored durably. Default: 60 |
| `effect_sweep_seconds` | integer | How often expired effects are swept. 0 disables. Default: 5 |
| `effect_heartbeat_seconds` | integer | Drives effects that tick. 0 disables. Default: 3 |
| `combat_round_seconds` | integer | Seconds per combat round. 0 disables. Default: 3 |
| `respawn_room` | string | Where the dead reappear. Falls back to `start_room` |
| *anything else* | any | Captured and readable from Lua as `config("game.<key>")` |

```toml
[game]
name = "My MUD"
mudlib_path = "./mudlib"
autosave_seconds = 300
shutdown_timeout_seconds = 30
```

`autosave_seconds` bounds how much progress a *crash* can cost — a kill or a
power cut reaches no Lua at all. A clean Ctrl+C loses nothing, because the
driver dispatches [`on_shutdown`](./lua-api/events.md) and waits for the mudlib
to flush. `shutdown_timeout_seconds` bounds that wait: when it expires the
server logs an error and exits anyway, so a mudlib that wedges in `on_shutdown`
cannot hang the process.

The seven keys below it tune the periodic subsystems, and every one of them
accepts 0 to turn the corresponding ticker off entirely — which is what the test
harness does, so a timer never fires in the background of an unrelated test. See
[State Cache](./lua-api/state-cache.md), [Effects](./lua-api/effects.md) and
[Combat](./lua-api/combat.md).

**`[game]` is open.** Any key the driver has no opinion about is captured rather
than rejected, and reachable from Lua as `config("game.<key>")` with no Rust
change:

```toml
[game]
respawn_room = "thornhollow.square"
shop_restock_seconds = 600
builder_area = "sandbox"
```

`config()` used to be an eighteen-key allowlist in Rust, so every game-layer
setting needed a driver edit before Lua could see it — and that pressure is why
`death_d`, a *mudlib* file, once had `wizard_workshop.entrance` written into it.
See [`config()`](./lua-api/efuns.md#configkey--any).

### `[sessions]`

| Key | Type | Description |
|-----|------|-------------|
| `multisession_mode` | string | How multiple connections per account are handled |
| `max_connections` | integer | Maximum total concurrent connections |

**Multisession modes:**

| Mode | Behavior |
|------|----------|
| `"single"` | New login kicks old session (traditional MUD) |
| `"shared_character"` | Multiple sessions share one character |
| `"multi_character"` | Multiple sessions, each playing a different character |
| `"full_multi"` | Multiple sessions can share characters freely |

### `[accounts]`

| Key | Type | Description |
|-----|------|-------------|
| `allow_creation` | bool | Whether new accounts can be created from login screen |
| `min_password_length` | integer | Minimum password length in characters |
| `max_characters_per_account` | integer | Max characters per account |

### `[limits]`

| Key | Type | Description |
|-----|------|-------------|
| `lua_memory_mb` | integer | Max Lua VM memory in megabytes. Enforced; `0` = no ceiling |
| `lua_instruction_limit` | integer | Max Lua instructions per dispatch. `0` disables it. On a LuaJIT build, enforcing it disables the compiler — see below |
| `input_buffer_bytes` | integer | Max bytes in a single input line |

> [!IMPORTANT]
> On a **LuaJIT** build, `lua_instruction_limit` and the compiler are mutually
> exclusive: LuaJIT dispatches no debug hooks from inside a compiled trace, so a
> runaway loop is invisible to any hook while the JIT is on, and the engine calls
> `jit.off()` whenever a limit is configured. Measured through the real mudlib,
> enforcing costs 2-7% on commands and the compiler is worth ~1.00x on them, so
> it ships **on**.
>
> On the default **Lua 5.5** build there is no compiler to give up, and errors
> raised from a hook are uncatchable — so `while true do pcall(f) end`, which
> could swallow the budget error on 5.1, cannot survive it. See
> [Performance & the JIT Trade-off](./lua-api/performance.md).

### `[compute]`

Running long Lua off the game thread, in worker *processes*. Off by default;
with `enabled = false` no child process is ever started. Full guide:
[Compute — Off-Thread Lua](./lua-api/compute.md).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `false` | Register the `compute()` efun and start the pool |
| `workers` | integer | `2` | Worker processes, started lazily on first use |
| `worker_path` | string | *beside the server* | Where the `oxigeon-compute` binary is |
| `queue_depth` | integer | `16` | Jobs allowed to wait before new ones are refused |
| `default_deadline_ms` | integer | `5000` | Wall clock a job gets before it is given up on |
| `max_deadline_ms` | integer | `60000` | Ceiling a caller may ask for |
| `instruction_limit` | integer | `0` | Instructions one job may run. `0` keeps the LuaJIT compiler |
| `memory_mb` | integer | `256` | Memory ceiling per worker VM. `0` = no ceiling |
| `roots` | string[] | `["compute"]` | Module prefixes an entry point may live under |
| `max_arg_depth` | integer | `64` | Nesting limit when copying a value between VMs |
| `max_arg_nodes` | integer | `100000` | Size limit for the same |

> [!IMPORTANT]
> **A worker is a separate binary** and has to be built:
> `cargo build --release -p oxigeon-compute`. It links LuaJIT whatever the server
> was built with — the server defaults to Lua 5.5 for the debugger, and a compute
> job wants the compiler instead; one binary cannot link both. It is looked for
> beside the server binary unless `worker_path` says otherwise.

A job that overruns its deadline has its worker process **killed and replaced**,
and is counted in `server_info().compute.wedged`. Setting `instruction_limit`
lets a job stop itself instead, at the cost of the compiler in that VM.

---

## permissions.toml

Permission gating for driver efuns and mudlib directories.
See the [full permissions.toml reference](./configuration/permissions-toml.md) for details.

```toml
[efuns]
reload     = "efun.reload"
write_file = "efun.write_file"
broadcast  = "efun.broadcast"

[directories]
"/admin" = { read = "dir.read.admin", write = "dir.write.admin" }
"/data"  = { write = "dir.write.data" }
```

Missing file → all efuns and directories are unrestricted (open default).

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `DRIVER_CONFIG` | Override path to driver.toml (default: `config/driver.toml`) |
| `SERVER_CONFIG` | Override path to server.toml (default: `config/server.toml`) |
| `RUST_LOG` | Override log level (overrides `logging.level` in driver.toml) |

```bash
RUST_LOG=debug DRIVER_CONFIG=config/driver.dev.toml cargo run
```
