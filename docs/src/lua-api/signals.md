# Signals & Events (EVENT_D)

The event system provides Godot-style signals: named event channels that any object can dynamically subscribe to or emit. It's a core mudlib daemon — pure Lua with no driver involvement needed.

Events are the backbone of reactive game systems: mobs responding to threats, areas triggering alarms, combat tracking kills, and players broadcasting achievements.

## Quick Start

```lua
-- 1. Subscribe: "when a mob dies, I want to know"
DAEMON.event.on("mob.died", "my_system.on_death", function(data)
    log("info", data.mob_id .. " was killed by " .. tostring(data.killer))
end)

-- 2. Emit: "a mob just died, here's the context"
DAEMON.event.emit("mob.died", {
    mob_id   = "mob.guard_1",
    room_id  = "town.gate",
    killer   = char_id,
})

-- 3. Unsubscribe: "I don't care anymore"
DAEMON.event.off("mob.died", "my_system.on_death")
```

---

## Core API

### `DAEMON.event.on(event, listener_id, callback, priority?)`

Subscribe to an event. When the event is emitted, `callback(data)` is called.

| Param | Type | Description |
|-------|------|-------------|
| `event` | string | Event name (e.g. `"mob.died"`) |
| `listener_id` | string | Unique ID for this subscription |
| `callback` | function | Called with `(data)` when event fires |
| `priority` | number? | Optional. Lower fires first. Default `0`. |

```lua
-- Default priority (0)
DAEMON.event.on("room.entered", "trap.check", function(data)
    check_for_traps(data.char_id, data.room_id)
end)

-- Higher priority fires later
DAEMON.event.on("room.entered", "announce.arrival", function(data)
    local char = get_character(data.char_id)
    if char then
        messaging.send_to_room(data.room_id,
            char.name .. " arrives.", data.char_id)
    end
end, 10)
```

### `DAEMON.event.off(event, listener_id)`

Remove a specific listener. Returns `true` if found and removed.

```lua
DAEMON.event.off("mob.died", "guard_2.enrage")
```

### `DAEMON.event.off_all(event)`

Remove **all** listeners for an event. Returns the count removed. Use for area unloads or system resets.

```lua
-- Unload an area's alarm system
DAEMON.event.off_all("area.dungeon.alarm")
```

### `DAEMON.event.off_by_prefix(prefix)`

Remove all listeners across **all events** whose listener ID starts with `prefix`. Returns the count removed.

This is the primary cleanup mechanism — when a mob dies, an area unloads, or a system shuts down, remove all its subscriptions in one call.

```lua
-- A mob dies: remove everything it subscribed to
DAEMON.event.off_by_prefix("mob.guard_1.")

-- An area unloads: remove everything with that area prefix
DAEMON.event.off_by_prefix("area.dungeon.")
```

### `DAEMON.event.emit(event, data)`

Fire an event. All registered listeners are called synchronously in priority order (lower first). Each handler is `pcall`-wrapped — a failure in one never stops the others.

Returns the number of listeners called.

```lua
local count = DAEMON.event.emit("mob.died", {
    mob_id   = "mob.guard_1",
    room_id  = "town.gate",
    killer   = char_id,
})
log("debug", count .. " listeners handled mob.died")
```

### `DAEMON.event.defer(event, data, delay?)`

Emit an event after a delay, using `DAEMON.ticker`. Avoids re-entrancy issues (e.g., a death handler emitting another death event in the same call stack).

```lua
-- Respawn event after 60 seconds
DAEMON.event.defer("mob.respawn", { mob_id = "mob.guard_1" }, 60)

-- "Next tick" (0.01s default)
DAEMON.event.defer("combat.ended", { winner = char_id })
```

---

## Introspection

| Method | Returns | Description |
|--------|---------|-------------|
| `has_listeners(event)` | boolean | Any listeners registered for this event? |
| `count(event)` | number | How many listeners on this event. |
| `listeners(event)` | table | Array of listener ID strings. |
| `events()` | table | Array of all event names with listeners. |
| `clear_all()` | — | Nuclear reset: remove everything. |

```lua
-- Admin command: show event system status
for _, event_name in ipairs(DAEMON.event.events()) do
    local n = DAEMON.event.count(event_name)
    send(session_id, event_name .. ": " .. n .. " listener(s)\r\n")
end
```

---

## Event Naming Conventions

Use dotted names that describe **scope** and **action**:

### Combat

```lua
"mob.died"          -- { mob_id, room_id, killer }
"mob.damaged"       -- { mob_id, room_id, amount, source }
"combat.started"    -- { attacker, defender, room_id }
"combat.ended"      -- { winner, loser, room_id }
```

### World & Movement

```lua
"room.entered"      -- { char_id, room_id, from_direction }
"room.left"         -- { char_id, room_id, direction }
"area.reset"        -- { area_name }
```

### Area-Scoped

```lua
"area.dungeon.alarm"  -- { triggered_by }
"area.forest.weather" -- { new_weather }
```

### Player

```lua
"player.login"      -- { char_id, session_id }
"player.logout"     -- { char_id }
"player.levelup"    -- { char_id, new_level }
```

### Items

```lua
"item.picked_up"    -- { item_id, char_id, room_id }
"item.dropped"      -- { item_id, char_id, room_id }
"item.used"         -- { item_id, char_id, target }
```

---

## Patterns & Examples

### Area-Wide Alarm

All guards in a dungeon react when an alarm is triggered:

```lua
-- During area load, subscribe all guards
local guard_ids = { "mob.guard_1", "mob.guard_2", "mob.guard_3" }
for _, mob_id in ipairs(guard_ids) do
    DAEMON.event.on("area.dungeon.alarm", mob_id .. ".react", function(data)
        set_object_state(mob_id, "alert", true)
        local room_id = DAEMON.world.get_character_room(mob_id)
        if room_id then
            messaging.send_to_room(room_id, "The guard snaps to attention!")
        end
    end)
end

-- A trip wire room action triggers the alarm
local function trip_wire(session_id, args_str, args)
    DAEMON.event.emit("area.dungeon.alarm", { triggered_by = session_id })
    send(session_id, "A thin wire snaps underfoot. An alarm bell rings!\r\n")
end
```

### Mob Enrage on Comrade Death

When a mob dies, nearby mobs of the same faction fly into a rage:

```lua
DAEMON.event.on("mob.died", "enrage_system", function(data)
    local room = DAEMON.world.get_room(data.room_id)
    if not room then return end
    for _, char_id in ipairs(room:get_characters()) do
        if is_mob(char_id) and get_faction(char_id) == get_faction(data.mob_id) then
            set_object_state(char_id, "enraged", true)
            messaging.send_to_room(data.room_id,
                get_mob_name(char_id) .. " flies into a rage!")
        end
    end
end)
```

### Timed Puzzle with Event Feedback

A dial puzzle that emits events for other systems to react to:

```lua
-- The puzzle completion handler
DAEMON.event.on("puzzle.solved", "dungeon.door_open", function(data)
    if data.puzzle_id == "dungeon.dial" then
        set_object_state("dungeon.treasure_room", "door_open", true)
        messaging.send_to_room(data.room_id,
            "A heavy grinding sound echoes as a hidden door slides open.")
    end
end)

-- The room action
local function turn_dial(session_id, args_str, args)
    local pos = (get_object_state("dungeon.cell", "dial_position") or 0) + 1
    set_object_state("dungeon.cell", "dial_position", pos)
    send(session_id, "You turn the dial to position " .. pos .. ".\r\n")

    if pos == 3 then
        DAEMON.event.emit("puzzle.solved", {
            puzzle_id = "dungeon.dial",
            room_id   = "dungeon.cell",
            solver    = session_id,
        })
    end

    -- Reset after 10 seconds
    DAEMON.ticker.after(10, "dungeon.cell.dial_reset", function()
        set_object_state("dungeon.cell", "dial_position", 0)
        messaging.send_to_room("dungeon.cell",
            "The dial clicks back to its starting position.")
    end)
end
```

### Chained Events

Events can emit other events — but use `defer()` to avoid deep call stacks:

```lua
-- When a boss dies, trigger an area-wide celebration
DAEMON.event.on("mob.died", "boss.check", function(data)
    if data.mob_id == "mob.dragon_king" then
        -- Defer to avoid re-entrancy
        DAEMON.event.defer("area.dragon_lair.cleared", {
            cleared_by = data.killer,
        })
    end
end)

DAEMON.event.on("area.dragon_lair.cleared", "celebration", function(data)
    broadcast("\r\nThe ground shakes as the Dragon King falls! The realm rejoices!\r\n")
end)
```

### Cleanup on Area Unload

```lua
-- When unloading area "dungeon":
-- 1. Remove all area-scoped event channels
DAEMON.event.off_all("area.dungeon.alarm")
DAEMON.event.off_all("area.dungeon.reset")

-- 2. Remove all mob listeners from this area
for _, mob_id in ipairs(dungeon_mob_ids) do
    DAEMON.event.off_by_prefix(mob_id .. ".")
end
```

---

## How It Works

EVENT_D is pure Lua — no driver efuns involved.

```
  Publisher                    EVENT_D                   Subscribers
     │                           │                           │
     │  emit("mob.died", data)   │                           │
     ├──────────────────────────►│                           │
     │                           │  sort by priority         │
     │                           │  for each listener:       │
     │                           │    pcall(callback, data)──►│ "enrage_system"
     │                           │    pcall(callback, data)──►│ "loot_system"
     │                           │    pcall(callback, data)──►│ "score_tracker"
     │                           │                           │
     │                  returns count                        │
     │◄──────────────────────────│                           │
```

- **Synchronous** — all handlers fire immediately on `emit()`
- **Priority-ordered** — lower priority number fires first
- **Fault-tolerant** — each handler is `pcall`-wrapped; failures are logged, never stop the chain
- **Sorted cache** — listener list is sorted once on subscribe, reused on every emit

---

## API Reference

| Method | Description |
|--------|-------------|
| `on(event, id, fn, priority?)` | Subscribe to an event. Lower priority fires first. |
| `off(event, id)` | Unsubscribe one listener. |
| `off_all(event)` | Remove all listeners for an event. |
| `off_by_prefix(prefix)` | Remove all listeners whose ID starts with prefix. |
| `emit(event, data)` | Fire an event, calling all listeners in priority order. |
| `defer(event, data, delay?)` | Fire an event after a delay (via TICKER_D). Default 0.01s. |
| `has_listeners(event)` | Check if any listeners exist for an event. |
| `count(event)` | Count listeners for an event. |
| `listeners(event)` | List listener IDs for an event. |
| `events()` | List all events with active listeners. |
| `clear_all()` | Remove all listeners for all events. |
