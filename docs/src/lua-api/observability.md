# Observability & Logging

Oxigeon provides rich, structured observability that goes far beyond what traditional
MUD drivers offer. Rather than silent failures and vague errors, every significant
event is logged with context: who did it, what happened, and why.

---

## Architecture

```
Lua errors ──────────────────────────────────────────┐
Efun permission denials ──────────────────────────────┤
Lua journal_write() calls ────────────────────────────┤
                                                      ▼
                                              logs/journal.log
                                              (JSON Lines)

Efun permission denials ──────────────────────────────┐
Command watch list hits ──────────────────────────────┤
Explicit audit_write() calls ─────────────────────────┤
                                                      ▼
                                              logs/audit.log
                                              (JSON Lines)
```

Both files are append-only JSON Lines (one JSON object per line) so they can be
`grep`'ed, tailed, and parsed with standard tools.

---

## Daemons

Daemons are singleton Lua modules loaded at startup and stored in the global `DAEMON` table.

```lua
-- Available after mudlib init:
DAEMON.journal   -- journal_d daemon
DAEMON.audit     -- audit_d daemon
```

### Daemon Loading

```lua
-- mudlib/init.lua loads daemons automatically:
DAEMON = {}
DAEMON.journal = require("daemons.journal_d")
DAEMON.audit   = require("daemons.audit_d")
```

---

## journal_d — Server Journal

The journal is a general-purpose log for info, warnings, errors, and debug output.
All Lua crashes are automatically written here by the driver.

### Writing to the Journal (Lua)

```lua
-- Via daemon (preferred):
DAEMON.journal.info("Player entered the dungeon", '{"char":"Gandalf"}')
DAEMON.journal.warn("Low memory in Lua VM")
DAEMON.journal.error("Area file parse failed: areas/dungeon.lua")
DAEMON.journal.debug("Spawning monster at room 42")

-- Direct efun:
journal_write("info", "Server event: respawn cycle", nil)
```

**Log levels:** `trace`, `debug`, `info`, `warn`, `error`

### Reading the Journal (Lua)

```lua
-- Last 20 entries:
local entries = DAEMON.journal.recent(20)

-- Last 50 error entries:
local errors = DAEMON.journal.recent(50, "error")

-- Format for display:
for _, raw in ipairs(entries) do
    print(DAEMON.journal.format_entry(raw))
end
```

### `journal` Command

```
journal [count] [level]
```

| Argument | Description |
|----------|-------------|
| `count`  | Number of entries to show (default 20, max 200) |
| `level`  | Filter by level: `error`, `warn`, `info`, `debug`, `trace` |

```
> journal 10
> journal 50 error
> journal warn
```

**Requires:** `daemon.journal_d.read` permission

### Journal Log Entry Format

```json
{
  "ts":     "2026-07-15T18:30:00Z",
  "level":  "error",
  "source": "login.lua:42",
  "msg":    "attempt to index a nil value (field 'state')\nstack traceback:\n  ...",
  "meta": {
    "event":   "on_input",
    "sid":     "f3a2b1c0",
    "char_id": 7,
    "input":   "look"
  }
}
```

### Automatic Lua Error Capture

When any Lua callback (`on_input`, `on_connect`, etc.) throws an error, the driver
automatically captures:

- File name and line number (parsed from the Lua error string)
- Full stack traceback
- Current session ID
- Event name and arguments (input truncated to 100 chars)
- Character ID

This means **you never lose a Lua crash** — every error is persisted to disk.

---

## audit_d — Audit Trail

The audit log records security-relevant events: privilege use, permission denials,
and any command the server owner decides to watch.

### What Gets Automatically Audited

| Event | Recorded |
|-------|----------|
| Efun permission denied (e.g. `reload` without perm) | Always |
| Command permission denied (via `M.permission` check) | Always |
| Commands in the watch list (see below) | When condition matches |
| Explicit `audit_write()` calls | Always |

### Writing to the Audit Log (Lua)

```lua
-- Via daemon:
DAEMON.audit.log("cmd.spawn", true)            -- success
DAEMON.audit.log("cmd.smite", false, "target not found")  -- failure

-- Direct efun (uses current session automatically):
audit_write("cmd.kick", true, nil)
audit_write("cmd.give", false, "item not found")
```

### Reading the Audit Log (Lua)

```lua
local entries = DAEMON.audit.recent(20)
-- Returns array of raw JSON strings; use audit_read() directly for
-- lower-level access.
```

### `audit` Command

```
audit [count]                     -- tail recent entries
audit list                        -- show command watch list
audit add <command> <condition>   -- watch a command
audit rm <command>                -- stop watching a command
```

**Reading** requires: `daemon.audit_d.read`
**Managing** (add/rm) requires: `daemon.audit_d.manage`

#### Examples

```
> audit 10
> audit list
> audit add spawn all
> audit add give success
> audit add kick fail
> audit rm spawn
```

#### Watch Conditions

| Condition | When logged |
|-----------|-------------|
| `all`     | Always — success and failure |
| `success` | Only when the command completed without error |
| `fail`    | Only when the command threw a Lua error |

### Audit Log Entry Format

```json
{
  "ts":      "2026-07-15T18:30:00Z",
  "sid":     "f3a2b1c0-...",
  "char":    "Gandalf",
  "action":  "cmd.spawn",
  "success": true,
  "reason":  null
}
```

#### Denial Entry (auto-generated)

```json
{
  "ts":      "2026-07-15T18:31:00Z",
  "sid":     "a1b2c3d4-...",
  "char":    "Sauron",
  "action":  "efun.reload",
  "success": false,
  "reason":  "permission denied",
  "required": "efun.reload"
}
```

### Audit Watch Persistence

The watch list is saved to `logs/audit_watch.json` automatically whenever you
`add` or `rm` a command. It's loaded from disk when the server starts, so
your configuration survives restarts.

```json
{
  "spawn": "all",
  "give":  "success",
  "kick":  "fail"
}
```

---

## Server Information & Uptime

### `server_info()` Efun

```lua
local info = server_info()
-- info.version         "0.1.0"
-- info.name            "My MUD"     (from server.toml [game].name)
-- info.started_at      "2026-07-15T18:00:00Z"
-- info.uptime_secs     1842.3       (float seconds since start)
-- info.dropped_output  0            (output lost to full session channels)
-- info.lua             { ... }      (the Lua heap — see below)
-- info.compute         { ... }      (absent when compute is off)
```

### The Lua heap

```lua
info.lua.heap_bytes      -- what the allocator has handed out
info.lua.heap_kb         -- the same number in the unit collectgarbage("count") uses
info.lua.limit_bytes     -- limits.lua_memory_mb, in bytes
info.lua.heap_fraction   -- heap / limit, absent when there is no limit
info.lua.gc_full_count   -- explicit full collections since boot
info.lua.gc_full_ms      -- what they cost, cumulatively
info.lua.gc_freed_bytes  -- what they recovered, cumulatively
```

**Nothing measured any of this before.** There were zero `collectgarbage` calls
anywhere and no Rust-side GC configuration, so LuaJIT ran at its default pause
of 200 — the heap roughly doubles before a full cycle — against a
`lua_memory_mb = 64` ceiling. A live set nearing ~32 MB grows into that ceiling,
LuaJIT runs an emergency full collection before failing, and the signature under
pressure is **latency spikes first, catchable allocation errors second**,
surfacing in whatever code happened to allocate rather than in the code
responsible.

### `gc_collect()` and the heap drill

```lua
local result = gc_collect()
-- result.freed_bytes, result.ms, result.heap_bytes
```

Runs a full collection and reports what it cost. Also reachable as
`mudstatus gc`, which audits the call — a full cycle is a stop-the-world pause
on the game thread, so it is behind a subcommand rather than run on every status
read. A diagnostic that causes the hitch it is meant to measure is worse than
none.

The drill: record the heap at boot, then again after an hour of a mob respawn
loop, a walk out into the virtual grid and back, and several `reload` cycles.
The number should return close to its baseline each time. A monotonic climb
across all three is the signature that object-state leaks, uncached virtual
rooms and closure retention on hot reload produce, and it is the only way to
tell those apart from an ordinary working set.

> [!IMPORTANT]
> **Do not tune GC parameters without one of these numbers.** Defaults are
> usually right and tuning blind makes things worse. These counters exist so
> that any later `setpause`/`setstepmul` change is justified by a measurement.

### `uptime` Command

Formats the uptime into human-readable text:

```
> uptime
My MUD has been running for 2 days, 3 hours and 15 minutes.
(Started: 2026-07-13T15:00:00Z)
```

No permission required — any player can check uptime.

---

## Privileged Broadcasts

### `alert` — Staff Alert

Sends a message only to online sessions holding the `daemon.alert` permission.
Use this for staff coordination: reboots, emergencies, coordination.

```
> alert Server reboot in 5 minutes for maintenance.
```

**Displayed as:**
```
[STAFF ALERT from Gandalf] Server reboot in 5 minutes for maintenance.
```

**Requires:** `daemon.alert` permission

### `announce` — Server-Wide Announcement

Sends a message to **all** connected sessions regardless of permissions.
Use this for player-facing announcements.

```
> announce The dungeon of doom has been reopened!
```

**Displayed as:**
```
[ANNOUNCEMENT from Gandalf] The dungeon of doom has been reopened!
```

**Requires:** `daemon.announce` permission

### `broadcast_to_perm()` Efun

For Lua-driven selective broadcasts:

```lua
-- Send to all sessions with a given permission
local count = broadcast_to_perm("daemon.alert", "\r\n[ALERT] Emergency server shutdown in 60s\r\n")
DAEMON.journal.info("Alert sent to " .. count .. " staff members")
```

**Requires:** `daemon.broadcast` permission on the calling session

---

## `verify` — In-Game Lua Syntax Check

The `verify` command compiles a Lua file through the runtime WITHOUT executing it.
This catches syntax errors and obvious runtime issues before you reload a file.

```
> verify cmds/spawn.lua
  ✓ File compiles successfully.

> verify cmds/broken.lua
  ✗ Compile error:
      broken.lua:15: '=' expected near 'end'
      stack traceback:
          [C]: in ?
```

**Requires:** `efun.verify` permission

The underlying efun can be called from Lua too:

```lua
local ok, err = verify_file("cmds/spawn.lua")
if not ok then
    DAEMON.journal.error("Syntax error in spawn.lua: " .. err)
end
```

---

## Required Permissions Reference

| Permission | Required for |
|------------|--------------|
| `daemon.journal_d.read` | Reading journal entries (`journal` command, `journal_read()`) |
| `daemon.audit_d.read` | Reading audit entries (`audit` command, `audit_read()`) |
| `daemon.audit_d.manage` | Adding/removing audit watches (`audit add`, `audit rm`) |
| `daemon.alert` | Sending staff alerts (`alert` command, `broadcast_to_perm("daemon.alert", ...)`) |
| `daemon.announce` | Server-wide announcements (`announce` command) |
| `daemon.broadcast` | Using `broadcast_to_perm()` efun |
| `efun.verify` | Using `verify` command and `verify_file()` efun |

Grant these like any other permission:

```lua
grant_permission("admin",   "daemon.audit_d.read")
grant_permission("admin",   "daemon.audit_d.manage")
grant_permission("admin",   "daemon.journal_d.read")
grant_permission("admin",   "daemon.alert")
grant_permission("admin",   "daemon.announce")
grant_permission("builder", "daemon.journal_d.read")
grant_permission("builder", "efun.verify")
```
