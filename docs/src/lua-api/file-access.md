# File & System Access

Oxigeon provides controlled file and time efuns that replace the blocked `io` module. `os` is reduced to its clock functions — `os.time`, `os.date`, `os.clock` and `os.difftime` remain available; everything else is removed. See [Sandboxing & Security](./sandboxing.md).

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
List the contents of a directory. Returns an array of entry tables, or `nil` if
the directory doesn't exist, the path escapes the jail, or reading it is refused
by `permissions.toml`. An **empty table** means a directory that exists and holds
nothing — so a caller can tell a misconfigured command path from an empty one.

Both roots are searched, game layer first, deduplicated by name: command and
area discovery spans the two layers, and the order matches `package.path` so the
layer that would be `require`d is the layer that is reported.

> [!WARNING]
> **This efun was registered twice, and the unsafe one won.** `register_io_efuns`
> installed the permission-checked, path-jailed version; `register_utility_efuns`
> ran later and overwrote it with one that joined the caller's path straight onto
> the two roots — no jail, no permission check. `list_dir("../../..")` escaped for
> as long as that was true, while this page and `sandboxing.md` both claimed
> traversal prevention "for all file efuns".
>
> The jailed implementation existed the whole time and production never reached
> it — the same failure shape as the sandbox and instruction-limit bugs
> `CLAUDE.md`'s testing section was written about. `tests/list_dir_jail.rs` asks
> the question through the engine's own VM, so a helper-level test cannot pass
> while the reachable version is broken.

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

### `uuid() → string`
A v4 UUID as a string. For anything that needs addressing and has no natural
key.

```lua
local id = "item:" .. uuid()   --> "item:6f9619ff-8b86-d011-b42d-00cf4fc964ff"
```

Item instances are the first user. A monotonic counter is enough for mobs, which
are never saved; a container in a player's inventory **is** saved, and a counter
restarting at zero on the next boot would hand out an id that already means
something else in somebody's save file — a data-corruption bug that only shows
up after a restart.

v4 rather than v1, so the id carries no timestamp and no MAC address. An id that
leaks when the server was started leaks more than an id needs to.

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
