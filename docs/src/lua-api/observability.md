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
DAEMON.journal   -- journald daemon
DAEMON.audit     -- auditd daemon
```

### Daemon Loading

```lua
-- mudlib/init.lua loads daemons automatically:
DAEMON = {}
DAEMON.journal = require("daemons.journald")
DAEMON.audit   = require("daemons.auditd")
```

---

## journald — Server Journal

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

**Requires:** `daemon.journald.read` permission

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

## auditd — Audit Trail

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

**Reading** requires: `daemon.auditd.read`
**Managing** (add/rm) requires: `daemon.auditd.manage`

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
-- info.version      "0.1.0"
-- info.name         "My MUD"     (from server.toml [game].name)
-- info.started_at   "2026-07-15T18:00:00Z"
-- info.uptime_secs  1842.3       (float seconds since start)
```

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
| `daemon.journald.read` | Reading journal entries (`journal` command, `journal_read()`) |
| `daemon.auditd.read` | Reading audit entries (`audit` command, `audit_read()`) |
| `daemon.auditd.manage` | Adding/removing audit watches (`audit add`, `audit rm`) |
| `daemon.alert` | Sending staff alerts (`alert` command, `broadcast_to_perm("daemon.alert", ...)`) |
| `daemon.announce` | Server-wide announcements (`announce` command) |
| `daemon.broadcast` | Using `broadcast_to_perm()` efun |
| `efun.verify` | Using `verify` command and `verify_file()` efun |

Grant these like any other permission:

```lua
grant_permission("admin",   "daemon.auditd.read")
grant_permission("admin",   "daemon.auditd.manage")
grant_permission("admin",   "daemon.journald.read")
grant_permission("admin",   "daemon.alert")
grant_permission("admin",   "daemon.announce")
grant_permission("builder", "daemon.journald.read")
grant_permission("builder", "efun.verify")
```
