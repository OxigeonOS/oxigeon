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
mudlib/                  ← the driver's Lua half: reusable across games
├── init.lua             ← event hooks, daemon loading, the disconnect chain
├── login.lua            ← login and registration
├── lib/
│   ├── commands.lua     ← dispatcher and lazy-loader
│   ├── object.lua       ← base class; `trait()` lives here
│   ├── item.lua         ← items (Weapon/Armor/Container are archetypes over it)
│   ├── weapon.lua  armor.lua  container.lua  requires.lua
│   ├── carry.lua        ← moving an item between floor, character and container
│   ├── equipment.lua    ← slots, requirements, `equip:` effect sources
│   ├── light.lua        ← whether you can see
│   ├── mobile.lua       ← NPCs and monsters (→ Player)
│   ├── player.lua       ← persistence via CHARACTER_D
│   ├── room.lua  movement.lua  messaging.lua  checks.lua  color.lua
│   └── traits.lua  effects.lua  persist.lua  jsonsafe.lua  strings.lua  tables.lua
├── daemons/             ← 25 singleton services on the DAEMON table
│   ├── journal_d.lua  audit_d.lua       ← logging and the audit trail
│   ├── ticker_d.lua   task_d.lua        ← timers, and named periodic work
│   ├── event_d.lua                      ← signals
│   ├── cache_d.lua    cooldown_d.lua    ← tiered state
│   ├── trait_d.lua    effect_d.lua      ← numbers, and what modifies them
│   ├── room_d.lua     world_d.lua       ← rooms, the registry, virtual providers
│   ├── item_d.lua     shop_d.lua        ← templates, instances, economy
│   ├── mob_d.lua      combat_d.lua      ← creatures and fights
│   ├── character_d.lua death_d.lua
│   ├── tag_d.lua                        ← the reverse index over tags
│   ├── prompt_d.lua   channel_d.lua  pager_d.lua  snoop_d.lua  gmcp_d.lua
│   └── codegen_d.lua  olc_d.lua         ← online creation
├── compute/
│   └── pathfind.lua     ← runs on a worker thread; no efuns
├── cmds/                ← ~60 commands, auto-discovered
└── tasks/

game/                    ← this game: content, and policy the driver has no view on
├── init.lua             ← registers everything below
├── setup_roles.lua      ← which roles exist and what they may do
├── daemons/
│   ├── aggro_d.lua      ← whether an aggressive creature attacks
│   ├── weather_d.lua    ← and what the sky is doing
│   ├── quest_d.lua      board_d.lua  spell_d.lua  reach_d.lua
│   └── gmcp_game_d.lua  ← this game's own GMCP packages
├── traits/  effects/  spells/  quests/
├── areas/
│   ├── thornhollow/     ← a town, in three room files merged into one area
│   ├── greywater_marsh/ ← weather-driven descriptions, aggressive creatures
│   ├── collapsed_mine/  ← dark rooms, a locked door, a puzzle, a boss
│   └── wizard_workshop/ ← the original example, kept as a regression fixture
└── cmds/                ← board, quest, cast, navigate
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
