# File & System Access

Oxigeon provides controlled file and time efuns that replace the blocked `io` module. `os` is reduced to its clock functions — `os.time`, `os.date`, `os.clock` and `os.difftime` remain available; everything else is removed. See [Sandboxing & Security](./sandboxing.md).

All file operations are **jailed to two directories** — the mudlib and the game
layer. You cannot reach anything outside them.

---

## Two roots, and how a path chooses one

The jail used to be the mudlib alone, and the cost was concrete: OLC generated
`mudlib/areas/…` while the world loads `game/areas/…`, so nothing it created
could ever load — and `dig` reported `File written: game/areas/…` regardless.

A path may name its root with a `game:` or `mudlib:` prefix. Unprefixed, the two
operations differ, and deliberately:

| | Unprefixed behaviour |
|---|---|
| **read** (`read_file`, `file_exists`, `verify_file`, `list_dir`) | game layer first, then the mudlib — the same order `require` uses, so the layer that would be loaded is the layer that is read |
| **write** (`write_file`, `append_file`) | always the mudlib |

Reads can search, because the file either exists or it does not. A write names a
file that may not exist yet, so there is nothing to search and the root has to be
*chosen* — and every automatic rule for choosing it is a guess. `audit_d` writes
`logs/audit_watch.json` and creates it on first use; a "new files go to the game
root" rule would have relocated it, a later read would still have found it
through the fallback, and the two copies would have drifted with nothing
reporting it.

So an unprefixed write goes where it has always gone, and anything that means
the game tree says so:

```lua
write_file("game:areas/crypt/rooms.lua", source)   -- content
write_file("mudlib:logs/audit_watch.json", json)   -- a file this daemon owns
```

> **A file a daemon owns should name its root.** Writes default to the mudlib and
> reads prefer the game layer, so a stray `game/logs/audit_watch.json` would
> shadow the one `audit_d` writes, permanently and silently. Being explicit costs
> seven characters.

`delete_file` resolves like a *read* — to whichever tree actually holds the file.
There is no sensible answer for deleting something that does not exist, and the
alternative would have `delete_file("areas/x.lua")` quietly miss the game-layer
file it was plainly aimed at.

An unknown prefix (`gmae:`) is an **error**, not a filename.

---

## File Operations

### `read_file(path) → string|nil`
Read an entire file as a string. Returns `nil` if the file doesn't exist, the
path is invalid, or reading it is refused by `permissions.toml`.

```lua
local contents = read_file("data/motd.txt")
if contents then
    send(session_id, contents)
end
```

### `write_file(path, content) → (bool ok, string? err)`
Write a string to a file (overwrites existing). Creates parent directories
automatically. Returns `true` on success; on failure, `false` and a reason.

```lua
local ok, err = write_file("game:areas/crypt/rooms.lua", source)
if not ok then
    log("error", "could not write the area: " .. tostring(err))
end
```

> **This returns failure; it does not raise it.** So
> `local ok, err = pcall(write_file, path, content)` gives you `ok = true,
> err = false` — the `pcall` succeeded, and the refusal is in a return value it
> discarded. `codegen_d` was written that way and reported success for every
> refused write for as long as it existed. Call it directly and read both values.

The error names the permission that would have allowed it, so a refusal is
something a builder can act on rather than a dead end:

```
permission denied: /game/areas/crypt/rooms.lua needs 'dir.write.game.areas' to write
```

### `append_file(path, content) → (bool ok, string? err)`
Append a string to a file (creates the file if it doesn't exist). Same contract
as `write_file`.

```lua
local timestamp = os_time()
append_file("mudlib:logs/chat.log", "[" .. timestamp .. "] " .. text .. "\n")
```

### `file_exists(path) → bool`
Check if a file exists in either root. Agrees with `read_file` by construction —
same resolution, same answer.

```lua
if file_exists("data/player.json") then
    local data = read_file("data/player.json")
end
```

### `file_root(path) → "game"|"mudlib"|nil`
Which tree a read of this path would land in, or `nil` if it exists in neither.

The only way to ask *which* file you got. Without it, shadowing between the
layers is invisible: two files with the same relative path look like one.

```lua
if file_root("cmds/verify.lua") == "game" then
    -- the mudlib's copy is shadowed by a game-layer one
end
```

### `dir_permission(path, "read"|"write") → string|nil`
The permission a directory rule demands at this path, or `nil` if it is
unrestricted. Read-only, and it answers about the **rule**, not about you — so
combine it with `has_permission` to decide what to show somebody.

Ungated, because `permissions.toml` is not a secret and the alternative is
probing by attempting the operation, which conflates "denied" with "does not
exist" and costs a syscall per entry listed.

```lua
local needed = dir_permission("/mudlib/admin", "read")
if needed and not has_permission(session_id, needed) then
    -- name the directory, hide its contents
end
```

### `list_dir(path) → table|nil`
List the contents of a directory. Returns an array of entry tables, or `nil` if
the directory doesn't exist, the path escapes the jail, or reading it is refused
by `permissions.toml`. An **empty table** means a directory that exists and holds
nothing — so a caller can tell a misconfigured command path from an empty one.

**Unprefixed, both roots are searched**, game layer first, deduplicated by name:
command and area discovery spans the two layers, and the order matches
`package.path` so the layer that would be `require`d is the layer that is
reported. `list_dir("game:areas")` lists exactly one root.

The merge is what discovery wants and the opposite of what somebody deciding
*where a file goes* wants, so which one you get is chosen at the call site rather
than assumed. Every entry carries a `root` field either way.

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
| `root` | string | `"game"` or `"mudlib"` — which tree this entry came from |

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

### `delete_file(path) → (bool ok, string? err)`
Delete a file. Same contract as `write_file` — returns failure, does not raise it.

Resolved like a **read**, to whichever tree holds the file, because deleting
something that does not exist has no sensible root to pick.

```lua
local ok, err = delete_file("mudlib:tmp/session_" .. session_id .. ".tmp")
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

All paths are relative to a root, chosen as described above. Absolute paths are
rejected, as is anything that resolves outside whichever root it landed in.

A `game:` or `mudlib:` prefix chooses a root; it is not a way past one. The jail
runs after the prefix is stripped, against that root.

| Path | Allowed | Reason |
|------|---------|--------|
| `data/players.db` | ✅ | Normal relative path |
| `lib/strings.lua` | ✅ | Subdirectory |
| `game:areas/crypt/rooms.lua` | ✅ | Names a root, stays inside it |
| `cmds/../lib/strings.lua` | ✅ | `..` that resolves back inside the root |
| `../../etc/passwd` | ❌ | Resolves outside the root |
| `game:../../etc/passwd` | ❌ | A prefix chooses a root; it does not escape one |
| `/etc/passwd` | ❌ | Absolute path is rejected |
| `gmae:areas/x.lua` | ❌ | Unknown root — an error, not a filename |

Note that `..` is judged by where it *lands*, not by its presence:
`cmds/../lib/strings.lua` is inside the mudlib and is allowed. Refusing it would
break nothing dangerous and surprise anyone building a path by hand.

> [!CAUTION]
> Even with jailing, be careful about what you write to disk. Consider using a dedicated `data/`
> directory for player-writeable data.
