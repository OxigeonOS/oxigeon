# Permissions & Roles

Oxigeon uses **Role-Based Access Control (RBAC)** to let game creators define in-game
authority structures without touching Rust code.

## How It Works

```
Account
  └── is_admin flag (superuser bypass — set directly in the database)

Character
  └── Roles  (e.g. "admin", "builder", "player")
        └── Permissions  (e.g. "efun.reload", "dir.write.areas")

Session (Playing)
  └── Cached permission set (populated at login, refreshed on role change)
```

**Creators invent their own role names and permission strings.** The driver only
enforces what's in `config/permissions.toml` — all other permission strings are
available for your own command-level checks.

---

## Session Permission Cache

When a character enters the game via `enter_game_session()`, the driver:
1. Looks up `account.is_admin` for the superuser bypass
2. Loads all permissions from all of the character's roles
3. Caches the result on the `Session` object in memory

Permissions are **checked in-memory** — no database query on every command.

If you change a character's roles while they're online, call `refresh_permissions(session_id)`
to update the cache immediately without a re-login.

---

## Superuser Bypass

If `account.is_admin = 1` in the database, that session's `has_permission()` returns
`true` for **any** permission string — driver-level and mudlib-level alike.

This is the escape hatch for the initial server owner. Set it via:

```sql
UPDATE accounts SET is_admin = 1 WHERE username = 'your_username';
```

---

## Permission Management Efuns

### Roles

```lua
-- Create a new role. Returns {id, name, created_at} or nil on failure.
local role = create_role("builder")

-- Delete a role (cascades to character assignments and role_permissions).
local ok = delete_role("builder")   -- → bool

-- List all roles.
local roles = list_roles()  -- → array of {id, name, created_at}

-- Assign/revoke a role from a character (by character_id).
local ok = assign_role(character_id, "builder")
local ok = revoke_role(character_id, "builder")

-- Get all roles a character currently has.
local role_names = get_roles(character_id)  -- → array of strings
```

### Permissions

```lua
-- Grant a permission string to a role.
local ok = grant_permission("builder", "dir.write.areas")
local ok = grant_permission("admin", "efun.reload")

-- Revoke a permission from a role.
local ok = revoke_permission("builder", "dir.write.areas")

-- Get all permissions granted to a role.
local perms = get_permissions("builder")  -- → array of strings
```

### Checking and Refreshing

```lua
-- Check if a session has a permission (checks their cached set).
local is_allowed = has_permission(session_id, "efun.reload")

-- Reload permissions from DB into the session cache.
-- Call this after assign_role/revoke_role if the player is already online.
local ok = refresh_permissions(session_id)
```

---

## Permission Naming Convention

Permission strings are arbitrary, but follow these conventions for clarity:

| Prefix | Used for |
|--------|----------|
| `efun.` | Driver efun access (e.g. `efun.reload`, `efun.broadcast`) |
| `efun.file.` | File efun access (e.g. `efun.file.write`, `efun.file.delete`) |
| `efun.session.` | Session control (e.g. `efun.session.disconnect`) |
| `dir.read.` | Directory read access (e.g. `dir.read.admin`) |
| `dir.write.` | Directory write access (e.g. `dir.write.areas`) |
| `cmd.` | Command-level gates (e.g. `cmd.reload`) — mudlib-defined |

The `cmd.*` namespace is purely for mudlib use — set `M.permission = "cmd.smite"` on
a command module and check it via `has_permission()` in the dispatcher.

---

## `config/permissions.toml`

This file controls which **driver efuns** and **file system paths** require a permission.
Missing file = all open (safe default for development).

```toml
# config/permissions.toml

[efuns]
# Maps efun name → required permission string.
# Sessions without this permission receive "Permission denied".
# Superusers bypass all checks.

reload      = "efun.reload"
write_file  = "efun.file.write"
append_file = "efun.file.write"
delete_file = "efun.file.delete"
disconnect  = "efun.session.disconnect"   # only for disconnecting OTHER sessions
broadcast   = "efun.broadcast"

[directories]
# Maps mudlib-relative path prefix → {read, write} permission strings.
# Longest-prefix match wins. Omitting read or write leaves that op unrestricted.

"/admin" = { read = "dir.read.admin",  write = "dir.write.admin" }
"/data"  = { write = "dir.write.data" }    # read is open
"/cmds"  = { write = "dir.write.cmds" }    # read is open; creators lock down cmd files
```

Directory paths are relative to the mudlib root. A path of `/data/foo.txt` matches
the `/data` prefix.

---

## Worked Example — Builder Role

```lua
-- 1. Create the role
local builder = create_role("builder")
if not builder then error("failed to create role") end

-- 2. Grant permissions
grant_permission("builder", "dir.write.areas")   -- can edit area files
grant_permission("builder", "dir.write.data")    -- can write to /data

-- 3. Assign the role to a character (e.g. in a god command)
function M.execute(session_id, args_str, args)
    local target_name = args[1]
    -- Find the character
    local char = get_character_by_name(target_name)  -- your mudlib function
    if not char then
        send(session_id, "\r\nCharacter not found.\r\n")
        return
    end
    assign_role(char.id, "builder")
    -- Refresh if online
    local online = find_session_for_character(char.id)  -- your mudlib function
    if online then refresh_permissions(online) end
    send(session_id, "\r\n" .. target_name .. " is now a builder.\r\n")
end

-- 4. In a command that requires builder access:
M.permission = "dir.write.areas"   -- dispatcher checks this automatically
```

---

## Command-Level Permissions

Set `M.permission` in any command file. The dispatcher checks it automatically:

```lua
-- mudlib/cmds/dig.lua
local M = {}
M.name       = "dig"
M.permission = "cmd.dig"    -- requires this permission to use the command

function M.execute(session_id, args_str, args)
    -- only reached if session has "cmd.dig"
end

return M
```

Grant it like any other permission:

```lua
grant_permission("builder", "cmd.dig")
```

---

## Database Schema

For reference, the underlying tables:

```sql
-- Named role groupings
CREATE TABLE roles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Interned permission name strings
CREATE TABLE permissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
);

-- Role → Permission mapping (many-to-many)
CREATE TABLE role_permissions (
    role_id       INTEGER NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    permission_id INTEGER NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, permission_id)
);

-- Character → Role assignment (many-to-many)
CREATE TABLE character_roles (
    character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    role_id      INTEGER NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    PRIMARY KEY (character_id, role_id)
);
```
