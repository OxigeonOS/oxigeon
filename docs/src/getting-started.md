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
# Enforcing this disables the LuaJIT compiler — measured at 2-7% on real
# commands, which is why it is on by default. 0 turns it off.
# See docs/src/lua-api/performance.md.
lua_instruction_limit = 1000000
input_buffer_bytes = 4096
```

## Your First Mudlib

The starter mudlib already includes a full command system. The key files are:

```text
mudlib/
├── init.lua           ← event hooks, daemons setup
├── login.lua          ← login/registration flow
├── lib/
│   ├── commands.lua   ← dispatcher and lazy-loader
│   ├── object.lua     ← base class for all MUD objects
│   ├── item.lua       ← items (→ Weapon, Armor)
│   ├── weapon.lua     ← weapon subclass
│   ├── armor.lua      ← armor subclass
│   ├── mobile.lua     ← NPCs, monsters (→ Player)
│   └── player.lua     ← player object (persistence via CHARACTER_D)
├── daemons/
│   ├── journal_d.lua  ← structured logging
│   ├── audit_d.lua    ← audit trail
│   ├── ticker_d.lua   ← timer scheduler
│   ├── event_d.lua    ← signal/event system
│   └── prompt_d.lua   ← prompt template engine
└── cmds/
    ├── help.lua
    └── quit.lua

game/
├── init.lua           ← game daemons and areas loading
├── daemons/
│   ├── room_d.lua     ← room creation
│   ├── character_d.lua← character state cache
│   ├── world_d.lua    ← room registry, movement
│   ├── codegen_d.lua  ← OLC code generation
│   └── olc_d.lua      ← OLC session manager
├── lib/
│   └── room.lua       ← room class
├── areas/
│   └── wizard_workshop/ ← example area
└── cmds/
    ├── look.lua       ← game commands
    ├── prompt.lua     ← prompt customization
    ├── olc.lua        ← online creation entry
    └── dig.lua        ← room creation (OLC)
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
    -- No need to call send_prompt() — the dispatcher renders the
    -- player's prompt automatically after every command.
end

return M
```

That's it — no registration required. The dispatcher lazy-loads `cmds/<verb>.lua`
automatically the first time the verb is typed (based on the `game.command_paths` config).

### Creating Your First Room

To add content to your game, you can use the `ROOM_D` builder in an area file.

Create `game/areas/starter.lua`:

```lua
local ROOM_D = require('daemons.room_d')

local rooms = {}

rooms[#rooms + 1] = ROOM_D.create("starter.tavern")
    :set_short("The Rusty Anchor")
    :set_description("You are standing in a dimly lit tavern. The smell of stale ale fills the air.")
    :set_light(1)
    :add_item("bar", "A sticky wooden counter.")
    :finish()

return rooms
```

Make sure to register this area in your `game/init.lua`:
```lua
DAEMON.world.register_area(require('areas.starter'))
```

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
