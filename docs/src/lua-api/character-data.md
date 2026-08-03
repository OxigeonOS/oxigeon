# Character Data & Persistence

The Oxigeon engine utilizes `CHARACTER_D` (the Character Daemon) to manage character state in-memory during play, while persisting it as JSON in the database on disconnect.

## The Data Lifecycle

1. **Login:** When a player authenticates and connects, `CHARACTER_D.load(char_id)` is called. The engine fetches the JSON blob from the database, deserializes it into a Lua table, and stores it in the daemon's memory cache.
2. **Gameplay:** During play, scripts read and modify character data exclusively via the in-memory cache using `CHARACTER_D` methods. This ensures high performance without constant database hits.
3. **Disconnect:** When a player quits or disconnects, `CHARACTER_D.unload(char_id)` is called. The in-memory table is serialized back to JSON and saved to the database, then cleared from memory.

> [!TIP]
> You generally do not need to save character data manually during normal gameplay. Rely on the automated save during disconnect. Only force a save for critical milestones (like level-ups or major transactions) to prevent data loss in the event of an engine crash.

## Usage Examples

Reading and writing data is straightforward using the daemon's API.

```lua
local CHARACTER_D = require('daemons.character_d')
local char_id = 42

-- Set a single value
CHARACTER_D.set(char_id, "hp", 100)

-- Get a single value
local current_hp = CHARACTER_D.get_value(char_id, "hp")

-- Merge multiple values at once (useful for initialization)
CHARACTER_D.merge(char_id, {
    max_hp = 100,
    mp = 50,
    max_mp = 50,
    level = 1,
    guild = "adventurer"
})

-- Working with nested tables (like inventory or quest flags)
local flags = CHARACTER_D.get_value(char_id, "quest_flags") or {}
flags.has_met_king = true
CHARACTER_D.set(char_id, "quest_flags", flags)
```

## CHARACTER_D API Reference

| Method | Description |
|--------|-------------|
| `load(char_id)` | Loads character data from the database into the memory cache. |
| `save(char_id)` | Saves the current memory cache for the character back to the database. |
| `get(char_id)` | Returns the entire data table for the character. |
| `get_value(char_id, key)` | Returns a specific value from the character's data table. |
| `set(char_id, key, value)` | Sets a specific value in the character's data table. |
| `merge(char_id, table)` | Merges the provided table into the character's existing data table. |
| `unload(char_id)` | Saves the character data and removes it from the memory cache. |
| `is_loaded(char_id)` | Returns `true` if the character is currently in the memory cache. |

## Engine Functions (Efuns)

The interaction between `CHARACTER_D` and the SQLite database is powered by two specialized engine functions (efuns). While `CHARACTER_D` wraps these for convenience, they are available globally if needed for custom implementations.

### `save_character_data(char_id, data_table) → boolean`
Takes a Lua **table** — not a JSON string — and serializes it to the character's `data` column. Returns `true` on success.

Raises if the table cannot be represented as JSON: one that is both a list and
a map (`{"a", "b", n = 1}`), a cycle, NaN or infinity, or a function value.
Those used to be silently mangled — the mixed-table case dropped every string
key on every save — so they now fail loudly and name the offending field.
`CHARACTER_D.save` already wraps this in `pcall` and journals the error.

### `load_character_data(char_id) → table|nil`
Loads the character's persistent data from the database, deserializing the JSON into a Lua table. Returns `nil` if the character doesn't exist or has no data.


## What lives in `stats`

[TRAIT_D](./traits.md) owns the contents of `player.stats`. It holds bases for
attributes and counters, current values for gauges, and regeneration anchors
under the reserved key `_at`:

```lua
player.stats = {
    strength = 12, wisdom = 14, level = 3,   -- attribute bases
    hp = 74, mp = 50,                        -- gauge currents
    _at = { hp = 1754151000 },               -- regeneration anchors
    -- max_hp, willpower: absent. Derived, and never stored.
}
```

> [!WARNING]
> **`stats[id]` is what is stored, not what is true.** For an attribute under a
> buff the two differ, and for a derived trait there is nothing stored at all.
> Read `player:stat(id)`.

Keys beginning with `_` are reserved. Everything else in `stats` is saved
verbatim — `Mobile:new` used to filter it through a fixed list of nine names,
which silently dropped any stat that was not on it.

Effects and cooldowns are *not* here. They have their own lifetimes and live in
the [state cache](./state-cache.md); the rule for deciding which home a piece of
state gets is on that page.
