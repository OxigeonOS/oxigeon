# Oxigeon — Project Rules

## Architecture

Oxigeon is a MUD (Multi-User Dungeon) game driver. Rust handles the infrastructure (networking, database, Lua VM), Lua handles all game logic.

**Two Lua layers:**
- `mudlib/` — Core system layer (login, command dispatch, event hooks, base daemons)
- `game/` — Game content layer (rooms, areas, game-specific daemons)

**Daemons** are singleton services registered in the global `DAEMON` table. Convention: files are named `*_d.lua` (e.g. `room_d.lua`, `character_d.lua`).

## Error Handling — Mandatory Practices

### 1. Never Silently Swallow Errors

Every operation that can fail must either:
- Be wrapped in `pcall()` with the error logged, **or**
- Return a status value that the caller checks

Silent failures (empty `if not x then return end` with no logging) are **not acceptable** for operations involving data persistence, world state, or player actions.

### 2. Use journald for Structured Error Logging

`log(level, msg)` writes to the server console (Rust tracing). `DAEMON.journal` writes to the **structured journal** (persisted, searchable, queryable by admins).

**Always log critical failures to both:**
```lua
local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        DAEMON.journal.error(message)
    end
end
```

Use `log()` alone for debug/trace level messages. Use both for `warn` and `error` level messages that admins need to see.

### 3. Protect Cleanup Chains

When multiple cleanup steps must all run (e.g. on disconnect: save data → remove from world → clear login state), wrap **each step** in its own `pcall` so a failure in one doesn't prevent the others:

```lua
-- CORRECT: each step is independent
if DAEMON.character then
    local ok, err = pcall(DAEMON.character.unload, char_id)
    if not ok then log_error("unload failed: " .. tostring(err)) end
end
if DAEMON.world then
    local ok, err = pcall(DAEMON.world.remove_character, char_id)
    if not ok then log_error("remove failed: " .. tostring(err)) end
end

-- WRONG: second step skipped if first throws
DAEMON.character.unload(char_id)
DAEMON.world.remove_character(char_id)
```

### 4. Protect Init Loading

When loading daemons and areas in init files, wrap each `require()` in `pcall` so a broken file doesn't crash the entire layer:

```lua
local ok, err = pcall(function() DAEMON.world = require('daemons.world_d') end)
if not ok then log("error", "Failed to load world_d: " .. tostring(err)) end
```

### 5. Validate Inputs in Daemons

Daemon functions that receive IDs or tables should validate before acting:
- Log a warning if called with unexpected arguments
- Return a clear failure value (false, nil) rather than crashing

## journald vs auditd

These are two separate logging daemons with distinct purposes:

### journald (`DAEMON.journal`)
**Purpose:** General-purpose structured server log.

Use journald for operational events — things a server operator or developer needs to see:
- Daemon load/unload events
- Errors and warnings during gameplay (save failures, missing rooms, bad lfun returns)
- Module hot-reloads
- Performance or state warnings

```lua
DAEMON.journal.error("CHARACTER_D: Failed to save data for char 42")
DAEMON.journal.info("Module reloaded: areas.wizard_workshop")
```

Entries are written to `logs/journal.log` via the Rust `GameLogger`. Searchable by level, readable via `DAEMON.journal.recent(n, level)`.

### auditd (`DAEMON.audit`)
**Purpose:** Security and compliance audit trail for player/admin actions.

Use auditd for tracking **who did what** — things a moderation team needs to review:
- Privileged command executions (spawn, ban, grant)
- Permission denials
- Admin actions
- Any command on the audit watch list

```lua
DAEMON.audit.log("cmd.ban", true, "banned user xyz")
DAEMON.audit.after_command("spawn", session_id, args_str, ok, err)
```

Entries are written to `logs/audit.log` via the Rust `GameLogger`. Commands can be added to the watch list with `DAEMON.audit.watch("spawn", "all")`.

**Rule of thumb:** If the question is "what went wrong?" → journald. If the question is "who did this?" → auditd.

## Lua Coding Conventions

- Use `\r\n` for player-facing text sent via `send()`
- Use `[[ ]]` for multi-line description strings
- Daemon files: `*_d.lua` naming convention
- Room IDs: `area_name.room_name` dotted notation
- All MUD objects inherit from `Object` (`game/lib/object.lua`)
- Use data-oriented `from_data()` / `load_area()` for authored rooms; builder for dynamic generation
- Properties support the lfun pattern — strings or functions returning strings, resolved via `Object.resolve()`
- Object state uses `set_object_state(id, key, value)` / `get_object_state(id, key)` efuns (or `Object:set_state(key, value)`)
- Timers go through `DAEMON.ticker` — never raw `schedule_timer` unless building a daemon
- Commands are auto-loaded from paths in `game.command_paths` config
- Character state goes through CHARACTER_D (in-memory cache), not raw efuns

## Testing

Run `cargo test` before committing. All tests must pass. Current count: 147.
