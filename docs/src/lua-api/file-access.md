# File & System Access

Oxigeon provides controlled file and time efuns that replace the blocked `io` and `os` modules.

All file operations are **jailed to the mudlib directory** — you cannot access files outside it.

---

## File Operations

### `read_file(path) → string|nil`
Read an entire file as a string. Returns `nil` if the file doesn't exist or the path is invalid.

```lua
local contents = read_file("data/motd.txt")
if contents then
    send(session_id, contents)
end
```

### `write_file(path, content) → bool`
Write a string to a file (overwrites existing). Creates parent directories automatically.
Returns `true` on success, `false` on failure.

```lua
local ok = write_file("data/player_notes.txt", "Some notes")
```

### `append_file(path, content) → bool`
Append a string to a file (creates the file if it doesn't exist).
Returns `true` on success, `false` on failure.

```lua
local timestamp = os_time()
append_file("logs/chat.log", "[" .. timestamp .. "] Player said: " .. text .. "\n")
```

### `file_exists(path) → bool`
Check if a file exists within the mudlib.

```lua
if file_exists("data/player.json") then
    local data = read_file("data/player.json")
end
```

### `list_dir(path) → table|nil`
List the contents of a directory. Returns an array of entry tables, or `nil` if the
directory doesn't exist or the path is invalid.

**Entry table fields:**
| Field | Type | Description |
|-------|------|-------------|
| `name` | string | File or directory name |
| `is_dir` | bool | `true` if this entry is a directory |
| `size` | integer | Size in bytes (0 for directories) |

```lua
local entries = list_dir("areas")
if entries then
    for _, entry in ipairs(entries) do
        if not entry.is_dir then
            log("debug", "Area file: " .. entry.name .. " (" .. entry.size .. " bytes)")
        end
    end
end
```

### `delete_file(path) → bool`
Delete a file. Returns `true` on success, `false` on failure.

```lua
delete_file("tmp/session_" .. session_id .. ".tmp")
```

---

## Time Functions

### `os_time() → number`
Returns the current Unix timestamp in seconds (float).

```lua
local now = os_time()
```

### `os_clock() → number`
Returns the current wall-clock time as a float in seconds. Useful for measuring durations.

```lua
local start = os_clock()
-- ... do work ...
local elapsed = os_clock() - start
log("debug", "Operation took " .. elapsed .. "s")
```

### `os_date(format) → string`
Returns the current local time formatted using a strftime-style format string.

```lua
local date_str = os_date("%Y-%m-%d %H:%M:%S")
append_file("logs/server.log", "[" .. date_str .. "] Server started\n")
```

Common format codes:

| Code | Example | Meaning |
|------|---------|---------|
| `%Y` | `2026` | 4-digit year |
| `%m` | `07` | Month (01–12) |
| `%d` | `15` | Day of month |
| `%H` | `14` | Hour (24h) |
| `%M` | `30` | Minutes |
| `%S` | `05` | Seconds |

---

## Path Security

All paths are relative to the mudlib root. Absolute paths are rejected, as are paths that escape
the mudlib (via `..`).

| Path | Allowed | Reason |
|------|---------|--------|
| `data/players.db` | ✅ | Normal relative path |
| `lib/strings.lua` | ✅ | Subdirectory |
| `../../etc/passwd` | ❌ | Contains `..` |
| `subdir/../../outside` | ❌ | Contains `..` |
| `/etc/passwd` | ❌ | Absolute path is rejected |

> [!CAUTION]
> Even with jailing, be careful about what you write to disk. Consider using a dedicated `data/`
> directory for player-writeable data.
