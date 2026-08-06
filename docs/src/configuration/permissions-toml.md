# permissions.toml

`config/permissions.toml` controls which driver efuns and mudlib directory paths
require a session to hold a specific permission string before they can be used.

If the file is missing, the server starts with all efuns and paths **unrestricted**
(appropriate for development). For production, create this file and configure it.

---

## Location

```
config/
├── driver.toml
├── server.toml
└── permissions.toml   ← this file
```

---

## `[efuns]` Section

Maps efun names to the permission string required to call them.
Sessions without the permission receive a Lua runtime error ("Permission denied").
Superusers (`account.is_admin = true`) bypass all checks.

```toml
[efuns]
# Hot-reload
reload = "efun.reload"

# File system (write operations — read is unrestricted unless [directories] says otherwise)
write_file  = "efun.write_file"
append_file = "efun.append_file"
delete_file = "efun.delete_file"

# Session control (only applies when disconnecting OTHER sessions, not yourself)
disconnect = "efun.disconnect"

# Broadcasting to all players
broadcast = "efun.broadcast"
```

**Ungated efuns** (not listed here) are callable by any authenticated session:
`send`, `send_prompt`, `send_gmcp`, `start_echo`, `stop_echo`, `get_session`,
`all_sessions`, `authenticate_session`, `enter_game_session`, `get_account`,
`create_account`, `get_character`, `get_characters`, `create_character`,
`has_permission`, `refresh_permissions`, `get_roles`, `get_permissions`,
`read_file`, `file_exists`, `list_dir`, `os_time`, `os_clock`, `os_date`,
`log`, `config`, `set_persistent`, `get_persistent`

---

## `[directories]` Section

Maps mudlib-relative path prefixes to per-operation permission requirements.
**Longest-prefix match wins.** Omitting `read` or `write` for a prefix leaves
that operation unrestricted.

```toml
[directories]
# Format: "prefix" = { read = "perm_string", write = "perm_string" }
# Both keys are optional — omit whichever should be unrestricted.

"/admin" = { read = "dir.read.admin",  write = "dir.write.admin" }
"/data"  = { write = "dir.write.data" }    # read is open; write requires perm
"/cmds"  = { write = "dir.write.cmds" }    # prevents players editing command files
```

### How prefix matching works

Given a file at `/data/admin/secret.lua` and two entries `/data` and `/data/admin`:
- `/data/admin` wins (longer match)

Given a file at `/data/scores.lua` and the same two entries:
- `/data` wins (only matching prefix)

Given a file at `/areas/dungeon.lua` with no matching entry:
- No restriction — any authenticated session can read or write.

---

## Full Example

```toml
# config/permissions.toml — production configuration

[efuns]
reload      = "efun.reload"
write_file  = "efun.write_file"
append_file = "efun.append_file"
delete_file = "efun.delete_file"
disconnect  = "efun.disconnect"
broadcast   = "efun.broadcast"

[directories]
# Admin tooling — only staff can read or write
"/admin" = { read = "dir.read.admin", write = "dir.write.admin" }

# Player data — anyone can read scores etc; only staff can write
"/data"  = { write = "dir.write.data" }

# Command files — builders can read; only senior staff can modify
"/cmds"  = { read = "dir.read.cmds", write = "dir.write.cmds" }

# Area files — builders can write; everyone can read
"/areas" = { write = "dir.write.areas" }
```

---

## Reloading Permission Config

The permission config is loaded once at startup. To change it, restart the server.

> [!NOTE]
> Future versions may support live-reloading `permissions.toml` via a driver command.
> For now, use `refresh_permissions(session_id)` in Lua to push role/permission
> changes to online sessions without a restart.
