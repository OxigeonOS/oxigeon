# Configuration Reference

Oxigeon uses two TOML configuration files. Both are loaded at startup.

## driver.toml

Infrastructure concerns — which servers run, database, logging.

### `[database]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | `"sqlite"` | Database backend: `"sqlite"` or `"postgresql"` |
| `url` | string | `"oxigeon.db"` | SQLite filename or PostgreSQL connection string |
| `pool_size` | integer | `5` | Connection pool size |

```toml
[database]
backend = "sqlite"
url = "oxigeon.db"
pool_size = 5

# PostgreSQL example:
# backend = "postgresql"
# url = "postgres://user:pass@localhost/oxigeon"
```

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
| `auto_continue_secs` | integer | `300` | Resume the VM if the editor stops responding while stopped (`0` = never) |
| `trace_capacity` | integer | `5000` | `trace show` ring buffer size, in records |
| `timing_capacity` | integer | `200` | `trace timings` ring buffer size |

```toml
[servers.debug]
enabled = true
bind = "127.0.0.1"
port = 4711
```

> **Freeze-the-world.** Hitting a breakpoint stops the entire Lua VM — every
> player is frozen until you continue. Connections stay alive and input queues,
> but repeating timers accumulate during the pause and fire as a burst on
> resume. This is a development tool.

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

---

## server.toml

Game-level concerns — name, account policy, session behavior, Lua limits.

### `[game]`

| Key | Type | Description |
|-----|------|-------------|
| `name` | string | Game name (shown in login banner) |
| `mudlib_path` | string | Path to mudlib directory (relative to working dir) |

```toml
[game]
name = "My MUD"
mudlib_path = "./mudlib"
```

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
| `lua_memory_mb` | integer | Max Lua VM memory in megabytes |
| `lua_instruction_limit` | integer | Max Lua instructions per call (prevents infinite loops) |
| `input_buffer_bytes` | integer | Max bytes in a single input line |

---

## permissions.toml

Permission gating for driver efuns and mudlib directories.
See the [full permissions.toml reference](./configuration/permissions-toml.md) for details.

```toml
[efuns]
reload     = "efun.reload"
write_file = "efun.file.write"
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
