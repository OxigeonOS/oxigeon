# Getting Started

This guide walks you through setting up a fresh Oxigeon MUD from scratch.

## Prerequisites

- Rust (stable, 1.75+) — install via [rustup](https://rustup.rs/)
- A C compiler (MSVC on Windows, gcc on Linux) — required by LuaJIT
- `mdbook` (for documentation) — `cargo install mdbook`

## Installation

```bash
# Clone or download Oxigeon
cd oxigeon/

# Build the driver (first build compiles LuaJIT — takes a few minutes)
cargo build --release

# Run with default config
cargo run
```

## Configuration

Oxigeon uses two configuration files:

| File | Purpose |
|------|---------|
| `config/driver.toml` | Infrastructure — which servers run, database, logging |
| `config/server.toml` | Game — name, account policy, Lua limits |

Minimal setup (SQLite, Telnet on port 4000):

```toml
# config/driver.toml
[database]
backend = "sqlite"
url = "oxigeon.db"
pool_size = 5

[servers.telnet]
enabled = true
bind = "0.0.0.0"
port = 4000

[logging]
level = "info"
```

```toml
# config/server.toml
[game]
name = "My MUD"
mudlib_path = "./mudlib"

[sessions]
multisession_mode = "single"
max_connections = 256

[accounts]
allow_creation = true
min_password_length = 8
max_characters_per_account = 1

[limits]
lua_memory_mb = 64
lua_instruction_limit = 1000000
input_buffer_bytes = 4096
```

## Your First Mudlib

The starter mudlib already includes a full command system. The key files are:

```
mudlib/
├── init.lua           ← event hooks; delegates commands to lib/commands
├── login.lua          ← login/registration flow
├── lib/
│   └── commands.lua   ← dispatcher and lazy-loader
└── cmds/
    ├── help.lua
    ├── quit.lua
    ├── say.lua
    ├── time.lua
    └── who.lua
```

### Adding a New Command

Create `mudlib/cmds/look.lua`:

```lua
local M = {}

M.name       = "look"
M.aliases    = { "l" }
M.category   = "movement"
M.summary    = "Look at your surroundings."
M.permission = nil   -- nil = any logged-in player

function M.execute(session_id, args_str, args)
    -- args_str : everything after "look" (raw)
    -- args     : whitespace-split tokens
    send(session_id, "\r\nYou see nothing but void.\r\n")
    send_prompt(session_id, "> ")
end

return M
```

That's it — no registration required. The dispatcher lazy-loads `cmds/<verb>.lua`
automatically the first time the verb is typed.

### Command Metadata

Every command should set these fields so the future help system can use them:

| Field | Purpose |
|-------|---------|
| `M.name` | Canonical verb (used for file lookup) |
| `M.aliases` | Alternative spellings (e.g. `"l"` for `"look"`) |
| `M.category` | Used to group commands in help (e.g. `"movement"`, `"combat"`) |
| `M.summary` | One-line description shown in help listings |
| `M.permission` | `nil` = unrestricted; string = required permission name |


## Connect with Mudlet

1. Start Oxigeon: `cargo run`
2. Open Mudlet → New Profile → Host: `localhost`, Port: `4000`
3. Click Connect

## Hot-Reload

While the server is running, you can reload a Lua module without restarting:

```lua
-- From a Lua admin command handler:
reload("login")  -- reloads mudlib/login.lua
```

Optionally, scripts can define lifecycle hooks:

```lua
function on_unload(module_name)
    -- Save state before module is cleared
end

function on_load(module_name)  
    -- Re-register anything needed after reload
end
```
