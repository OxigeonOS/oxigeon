# World Building — Rooms & Areas

The Oxigeon MUD engine provides two ways to define rooms: a **data-oriented format** (recommended for authored content) and a **builder pattern** (useful for programmatic/dynamic room generation).

## Data-Oriented Rooms (Recommended)

Area files are Lua files that return plain arrays of room data tables. Logic (action functions) is defined at the top of the file, separate from the data. Most rooms are 100% data — no functions needed.

```lua
-- game/areas/tavern.lua

-- Logic: define action functions at the top
    send(session_id, "You find a dusty coin under the counter.\r\n")
    -- The command dispatcher renders the prompt automatically after each command.
end

-- Data: room definitions as plain tables
return {
    {
        id    = "tavern.main_hall",
        short = "The Rusty Anchor",
        light = 1,
        smell = "Stale ale and wood smoke.",
        sound = "The creak of old floorboards.",

        description = [[
You stand in a dimly lit tavern. Heavy oak beams cross the low ceiling,
blackened by years of hearth smoke. A long bar runs along the far wall,
its surface sticky with the residue of countless spilled drinks.]],

        exits = {
            north = "tavern.kitchen",
            out   = "town.market_square",
        },

        items = {
            bar   = "A sticky wooden counter, scarred by knife marks and mug rings.",
            beams = "Thick oak beams, darkened almost black by decades of smoke.",
        },

        actions = {
            search = { func = search_bar, hint = "search" },
        },
    },

    {
        id    = "tavern.kitchen",
        short = "The Kitchen",
        light = 2,

        description = [[
A cramped kitchen with a cast-iron stove and hanging copper pots.]],

        exits = {
            south = "tavern.main_hall",
        },
    },
}
```

### Room Data Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `id` | string | **yes** | — | Unique room identifier (e.g., `"area.room_name"`) |
| `short` | string\|func | no | `"A Room"` | Room title shown at the top of `look` |
| `description` | string\|func | no | `"You are in a room."` | Full prose description |
| `light` | number | no | `2` | Light level: 0=dark, 1=dim, 2=normal, 3=bright |
| `smell` | string\|func | no | `nil` | Ambient smell |
| `sound` | string\|func | no | `nil` | Ambient sound |
| `exits` | table | no | `{}` | `{ direction = "target_room_id" }` |
| `items` | table | no | `{}` | `{ keyword = "description" }` |
| `actions` | table | no | `{}` | `{ verb = { func = fn, hint = "..." } }` |

### Registering Areas

**You do not.** Areas are *discovered*: `lib/areaload.lua` walks `areas/` and
loads whatever directories it finds, in passes across all of them — items, then
rooms, then mobs, then shops. Put a directory under `game/areas/` with a
`rooms.lua` in it and it loads on the next boot.

`game/init.lua` used to name every area explicitly, and it cost two things: an
area OLC created was invisible until somebody edited that file, and nothing
registered a reset source, so `areas reset <new_area>` answered "No registered
source" for every area OLC had ever made. Registering the reset spec is now the
last act of loading an area, so a working reset is not something anyone has to
remember.

The five entry file names are `rooms.lua` (or `init.lua`, for an area assembled
from several files), `items.lua`, `mobs.lua`, `shops.lua` and `custom.lua`.
Anything else in the directory is included by one of those.

The underlying calls are still there for a game that wants to place something by
hand:

```lua
local rooms = DAEMON.room.load_area(require('areas.tavern'))
DAEMON.world.register_area(rooms)
```

### Room Appearance

When a player types `look`, the room generates:

```text
The Rusty Anchor
You stand in a dimly lit tavern...
Obvious exits: north, out
Smell: Stale ale and wood smoke.
Sound: The creak of old floorboards.
You could try: search
Aldric is here.
```

## Callable Properties (lfun Pattern)

Any field marked `string|func` in the table above accepts either a literal string or a function that returns a string. The room object is passed as the argument:

```lua
{
    id = "outside.field",
    short = "Open Field",
    description = function(room)
        if room.light_level == 0 then
            return "It is pitch dark. You can't see a thing."
        else
            return "Sunlight streams across the grassy field."
        end
    end,
}
```

> [!WARNING]
> If a callable property function does not return a string, `<invalid lfun return>` is displayed in-game.

## Room Actions

Actions are room-scoped commands that only work while the player is in that room. They are checked *before* system commands during dispatch.

```lua
actions = {
    search = {
        func = function(session_id, args_str, args)
            send(session_id, "You find a hidden compartment!\r\n")
        end,
        hint = "search the area",  -- shown in room description
    },
},
```

> [!TIP]
> You do not need to call `send_prompt()` in commands or room actions. The command dispatcher calls `DAEMON.prompt.render()` automatically after every command.

**Command dispatch order:** room actions → system commands.
*(Future: guild → items → room → system.)*

## Room Items

Items are scenery that players can examine using `look <keyword>`:

```lua
items = {
    door  = "A heavy oak door covered in faded silver runes.",
    desk  = "An old wooden desk covered in papers.",
},
```

## Area Metadata (`_meta`)

Area files can include a `_meta` key with metadata about the area. This is stored in `DAEMON.world` and queryable at runtime.

```lua
return {
    _meta = {
        name   = "elven_forest",
        title  = "The Elven Forest",
        author = "Silvanus",
        level  = "10-20",
        status = "live",   -- "draft" | "review" | "live"
    },

    { id = "elven_forest.entrance", ... },
    { id = "elven_forest.clearing", ... },
}
```

Since `_meta` is a string key, `ipairs` (used by `load_area`) skips it automatically — it won't be treated as a room.

Query metadata at runtime:
```lua
local meta = DAEMON.world.get_area_meta("elven_forest")
if meta then
    log("info", meta.title .. " by " .. meta.author)
end
```

## Multi-File Areas

Large areas can be split across multiple files using `ROOM_D.merge()`:

```
game/areas/
├── wizard_workshop.lua              ← small area, single file
└── elven_forest/                    ← large area, split by section
    ├── init.lua                     ← index file: merges sub-files
    ├── village.lua                  ← village section
    ├── deep_woods.lua               ← forest section
    └── treetops.lua                 ← canopy section
```

The index file assembles sub-sections:

```lua
-- game/areas/elven_forest/init.lua
local ROOM_D = require('daemons.room_d')

return ROOM_D.merge(
    require('areas.elven_forest.village'),
    require('areas.elven_forest.deep_woods'),
    require('areas.elven_forest.treetops')
)
```

Each sub-file is a normal area data file. Put the `_meta` in whichever sub-file makes sense (usually the first one or the index); `merge()` takes the first `_meta` it finds.

Discovery handles it: an `init.lua` in the area directory *is* the entry file, so
a multi-file area needs no registration and no configuration. `thornhollow` is
built this way.

## Sharing a Skeleton Between Areas

Splitting one area across files is the easy half. The harder one is two areas
that keep saying the same thing — four creatures in two areas differing by four
numbers and repeating the other twelve keys each, or three rooms with the same
tags, light level and ambient sound.

A **prototype** is that skeleton, named once, in `game/prototypes/*.lua`:

```lua
-- game/prototypes/caves.lua
return {
    rooms = {
        ["cave"] = { light = 0, tags = { "indoor", "dark", "underground" },
                     sound = "Water, somewhere behind the rock." },
    },
}
```

```lua
-- game/areas/collapsed_mine/rooms.lua — only what differs
{
    id          = "collapsed_mine.first_level",
    prototype   = "cave",
    short       = "The First Level",
    description = "Props every eight feet, and none of them straight.",
    exits       = { up = "collapsed_mine.adit" },
},
```

Resolved at area load, before `custom.lua` and before anything is registered — so
a room built this way is a perfectly ordinary room and nothing downstream can
tell. The layering, in full:

```
schema defaults  ←  prototype chain  ←  the area's data file  ←  custom.lua
```

They do not compete with `custom.lua`. `custom.lua` is *this area's* last word; a
prototype is *everyone's* first word, and unlike `custom.lua` it can be
inherited by an area that does not exist yet.

`map` fields — `exits`, `items`, `stats` — merge key-by-key, so a room adds one
exit without restating the rest. Everything else replaces.

See [Prototypes](./prototypes.md).

## Virtual Rooms

Virtual rooms are generated on-the-fly from a **virtual provider** — a function registered by prefix on `DAEMON.world`. When `get_room()` can't find a room in the static registry, it checks providers by matching the first segment of the room ID (everything before the first dot).

### Registering a Provider

```lua
-- game/daemons/ocean_d.lua
local ROOM_D = require('daemons.room_d')

local M = {}

function M.generate(room_id)
    local x, y = room_id:match("^ocean%.(%d+)%.(%d+)$")
    if not x then return nil end
    x, y = tonumber(x), tonumber(y)

    return ROOM_D.from_data({
        id    = room_id,
        short = "The Open Ocean [" .. x .. ", " .. y .. "]",
        light = 3,
        smell = "Salt spray and seaweed.",
        sound = "The rhythmic crash of waves.",

        description = function(room)
            local descs = {
                "Endless waves stretch to every horizon.",
                "The sun glints off the rolling swells.",
                "A brisk wind fills your sails.",
            }
            return descs[((x * 7 + y * 13) % #descs) + 1]
        end,

        exits = {
            north = "ocean." .. x .. "." .. (y + 1),
            south = "ocean." .. x .. "." .. (y - 1),
            east  = "ocean." .. (x + 1) .. "." .. y,
            west  = "ocean." .. (x - 1) .. "." .. y,
        },
    })
end

return M
```

```lua
-- game/init.lua
local ocean = require('daemons.ocean_d')
DAEMON.world.register_virtual("ocean", ocean.generate)
```

### How It Works

1. Player moves to `ocean.5.3`
2. `get_room("ocean.5.3")` checks the static registry — not found
3. Prefix `"ocean"` matches a virtual provider
4. Provider generates the room and returns a Room object
5. Room is cached in the registry while occupied
6. If the MUD crashes and the player reconnects, the provider regenerates the room from the ID

The room ID **is** the persistence — it encodes everything needed to recreate the room.

### Use Cases

- **Ocean** — `ocean.X.Y` infinite grid
- **Desert** — `desert.X.Y` with biome variation from coordinates
- **Aether sky** — `sky.altitude.sector` with descriptions thinning at height
- **Procedural dungeon** — `dungeon.seed.level.room` where the seed determines layout

## Object State

All MUD objects (rooms, items, mobs) have an in-memory key/value state store that lives in the driver. It survives Lua hot-reloads but not server restarts. Use it for runtime state like opened doors, dropped items, or destruction.

State can be accessed via the efuns directly, or through `Object:get_state(key)` / `Object:set_state(key, value)` methods inherited by all object types.

### Efuns

```lua
-- Set a value
set_object_state("wizard_workshop.entrance", "door_locked", true)

-- Get a value
local locked = get_object_state("wizard_workshop.entrance", "door_locked")

-- Get all state for an object (returns table or nil)
local state = get_all_object_state("wizard_workshop.entrance")

-- Clear all state for an object
clear_object_state("wizard_workshop.entrance")
```

### Example: A Door That Can Be Opened

```lua
local function open_door(session_id, args_str, args)
    local room_id = "dungeon.cell"
    if get_object_state(room_id, "door_open") then
        send(session_id, "The door is already open.\r\n")
    else
        set_object_state(room_id, "door_open", true)
        send(session_id, "You heave the iron door open with a screech.\r\n")
    end
end

return {
    {
        id = "dungeon.cell",
        short = "A Dank Cell",
        description = function(room)
            if get_object_state("dungeon.cell", "door_open") then
                return "A cramped stone cell. The iron door stands open."
            else
                return "A cramped stone cell. A heavy iron door bars the exit."
            end
        end,
        actions = {
            open = { func = open_door, hint = "open the door" },
        },
    },
}
```

> [!NOTE]
> Object state does not survive server restarts. For state that must persist across restarts, serialize it to a file or database using a daemon.

## Tickers (TICKER_D)

The ticker system provides precise, event-driven timers backed by Tokio async tasks. Unlike traditional MUD heartbeats, tickers sleep and wake only when due — zero polling.

### API

```lua
-- One-shot: fire once after delay
DAEMON.ticker.after(10, "puzzle.reset", function()
    set_object_state("dungeon.cell", "dial_position", 0)
    messaging.send_to_room("dungeon.cell",
        "The dial clicks back to its starting position.")
end)

-- Repeating: fire every interval
DAEMON.ticker.every(15, "mob.guard_1.echo", function()
    local echoes = {
        "The guard shifts his weight from one foot to the other.",
        "The guard yawns loudly.",
    }
    messaging.send_to_room("town.gate", echoes[math.random(#echoes)])
end)

-- Cancel a ticker
DAEMON.ticker.remove("mob.guard_1.echo")

-- Check if active
DAEMON.ticker.is_active("mob.guard_1.echo")  -- true/false

-- List all active timer IDs
local ids = DAEMON.ticker.list()

-- Cancel everything (shutdown/reset)
DAEMON.ticker.clear_all()
```

### Use Cases

| Pattern | API | Example |
|---------|-----|---------|
| NPC echoes | `every(15, id, fn)` | Random atmospheric text from an NPC |
| Puzzle reset | `after(10, id, fn)` | Reset a dial 10 seconds after being turned |
| Mob respawn | `after(60, id, fn)` | Respawn a mob 60 seconds after death |
| Combat rounds | `every(3, id, fn)` | Run combat logic every 3 seconds |
| Weather cycle | `every(300, id, fn)` | Change weather every 5 minutes |

## Events (EVENT_D)

The event system provides Godot-style signals for reactive game systems: mobs responding to threats, areas triggering alarms, puzzles emitting completion events.

```lua
-- Subscribe
DAEMON.event.on("mob.died", "guard.enrage", function(data)
    set_object_state("mob.guard_2", "enraged", true)
end)

-- Emit
DAEMON.event.emit("mob.died", { mob_id = "mob.guard_1", room_id = "town.gate" })

-- Cleanup
DAEMON.event.off_by_prefix("mob.guard_1.")
```

See **[Signals & Events](./signals.md)** for the full API, naming conventions, patterns, and examples.

## Builder Pattern (Advanced)

For programmatic room creation (e.g., dungeon generators, runtime room spawning), the chainable builder is still available:

```lua
local ROOM_D = require('daemons.room_d')

local room = ROOM_D.create("generated.room_42")
    :set_short("Generated Chamber")
    :set_description("A room formed by magic.")
    :set_light(2)
    :add_exit("north", "generated.room_43")
    :finish()
```

Use `from_data()` and `load_area()` for authored content. Use the builder for dynamic generation.

## ROOM_D API Reference

### Data-Oriented (Preferred)

| Method | Description |
|--------|-------------|
| `from_data(table)` | Creates a single Room object from a plain data table. |
| `load_area(array)` | Processes an array of data tables into Room objects. Extracts and stores `_meta`. |
| `merge(...)` | Merges multiple room data arrays into one. Used for multi-file areas. |

### Builder Pattern

| Method | Description |
|--------|-------------|
| `create(id)` | Starts building a new room with the given unique string ID. |
| `set_short(text\|func)` | Sets the room title/short name. |
| `set_description(text\|func)` | Sets the main room description. |
| `set_light(level)` | Sets the light level (integer 0-3). |
| `set_smell(text\|func)` | Sets the ambient smell description. |
| `set_sound(text\|func)` | Sets the ambient sound description. |
| `add_exit(dir, target_id)` | Adds a directional exit pointing to another room ID. |
| `add_item(keyword, desc)` | Adds an item or scenery description. |
| `add_action(verb, func, hint)` | Adds a local command scoped to the room. |
| `finish()` | Completes construction and returns the room object. |

## World Daemon API Reference

### Room Registry

| Method | Description |
|--------|-------------|
| `get_room(room_id)` | Returns Room, checking static registry then virtual providers. |
| `register_room(room)` | Registers a single Room in the static registry. |
| `register_area(rooms_array)` | Registers an array of Room objects. |

### Virtual Providers

| Method | Description |
|--------|-------------|
| `register_virtual(prefix, fn)` | Register a generator function for a room ID prefix. |
| `unregister_virtual(prefix)` | Remove a virtual provider. |
| `virtual_prefixes()` | List registered virtual prefixes. |
| `evict_virtual(room_id)` | Remove a cached virtual room from the registry. |

### Area Metadata

| Method | Description |
|--------|-------------|
| `set_area_meta(name, meta)` | Store metadata for an area. |
| `get_area_meta(name)` | Retrieve metadata for an area. |
| `all_area_meta()` | Get all loaded area metadata. |

### Object State Efuns

| Efun | Description |
|------|-------------|
| `set_object_state(id, key, value)` | Set a key/value on an object's state. |
| `get_object_state(id, key)` | Get a value from an object's state. |
| `get_all_object_state(id)` | Get the entire state table for an object. |
| `clear_object_state(id)` | Clear all state for an object. |

### Timer Efuns

| Efun | Description |
|------|-------------|
| `schedule_timer(id, delay)` | Schedule a one-shot timer (seconds). |
| `schedule_repeating(id, interval)` | Schedule a repeating timer (seconds). |
| `cancel_timer(id)` | Cancel a timer. Returns `true` if found. |

### TICKER_D API

| Method | Description |
|--------|-------------|
| `DAEMON.ticker.after(delay, id, fn)` | One-shot timer with callback. |
| `DAEMON.ticker.every(interval, id, fn)` | Repeating timer with callback. |
| `DAEMON.ticker.remove(id)` | Cancel a timer and its callback. |
| `DAEMON.ticker.is_active(id)` | Check if a timer is registered. |
| `DAEMON.ticker.list()` | List all active timer IDs. |
| `DAEMON.ticker.clear_all()` | Cancel all timers. |

### EVENT_D API

| Method | Description |
|--------|-------------|
| `DAEMON.event.on(event, id, fn, priority?)` | Subscribe to an event. Lower priority fires first. |
| `DAEMON.event.off(event, id)` | Unsubscribe one listener. |
| `DAEMON.event.off_all(event)` | Remove all listeners for an event. |
| `DAEMON.event.off_by_prefix(prefix)` | Remove all listeners whose ID starts with prefix. |
| `DAEMON.event.emit(event, data)` | Fire an event, calling all listeners in priority order. |
| `DAEMON.event.defer(event, data, delay?)` | Fire an event after a delay (via TICKER_D). |
| `DAEMON.event.has_listeners(event)` | Check if any listeners exist. |
| `DAEMON.event.count(event)` | Count listeners for an event. |
| `DAEMON.event.listeners(event)` | List listener IDs for an event. |
| `DAEMON.event.events()` | List all events with listeners. |
| `DAEMON.event.clear_all()` | Remove all listeners for all events. |
