# Object Hierarchy

Oxigeon provides a class hierarchy for all game entities. Every object in the MUD — rooms, items, weapons, monsters, players — inherits from a shared `Object` base class, gaining common fields and driver state access.

## Inheritance Tree

```
Object (mudlib/lib/object.lua)
│   Base for all MUD objects. Provides id, short, description,
│   the lfun resolve() pattern, and driver state access.
│
├── Room (game/lib/room.lua)
│       Exits, contents, actions, scenery items, appearance rendering.
│
├── Item (mudlib/lib/item.lua)
│   │   Weight, value, slot, stackability, interaction hooks, tags.
│   │
│   ├── Weapon (mudlib/lib/weapon.lua)
│   │       Damage range, speed, weapon/damage type, requirements, combat messaging.
│   │
│   └── Armor (mudlib/lib/armor.lua)
│           Defense, armor type, resistances, stat bonuses, encumbrance.
│
└── Mobile (mudlib/lib/mobile.lua)
    │   Stats, inventory, equipment, AI behavior, echoes, patrol,
    │   loot table, respawn, dialogue, faction.
    │
    └── Player (mudlib/lib/player.lua)
            Database persistence (from_save/to_save), XP, gold,
            quest flags, skills, session link.
```

---

## Object

**File:** [`mudlib/lib/object.lua`](file:///c:/Code/oxigeon/mudlib/lib/object.lua)

The root base class. Every entity inherits these fields and methods.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | string | `"unknown"` | Unique identifier (e.g. `"town.square"`, `"player.42"`) |
| `short` | string \| function | `"Something"` | Display name. Supports lfun pattern. |
| `description` | string \| function | `"You see nothing special."` | Full description. Supports lfun pattern. |

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `Object:new(data) → obj` | Constructor. Initializes id, short, description from a data table. |
| `resolve` | `Object.resolve(value, obj) → string\|nil` | **Static.** Resolves lfun properties: returns strings as-is, calls functions with `obj` as argument. |
| `get_state` | `obj:get_state(key) → any` | Get a driver-side state value, scoped to this object's `id`. |
| `set_state` | `obj:set_state(key, value)` | Set a driver-side state value. |
| `get_all_state` | `obj:get_all_state() → table\|nil` | Get all state key/values for this object. |
| `clear_state` | `obj:clear_state()` | Clear all state for this object. |

---

## Room

**File:** [`game/lib/room.lua`](file:///c:/Code/oxigeon/game/lib/room.lua) — Inherits from **Object**

Represents a location in the game world.

### Fields (own)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `long` | string \| function | `"You are in a room."` | Detailed room description. Supports lfun. |
| `light_level` | number | `2` | Light level (0=pitch dark, 3=bright). |
| `smell` | string \| function \| nil | — | Ambient smell. Supports lfun. |
| `sound` | string \| function \| nil | — | Ambient sound. Supports lfun. |
| `exits` | table | `{}` | Direction → target room ID mapping. |
| `contents` | table | `{}` | Array of character IDs currently in the room. |
| `actions` | table | `{}` | Verb → `{ func, hint }` room-scoped commands. |
| `items` | table | `{}` | Keyword → description for examinable scenery. |

### Methods (own)

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `Room:new(data) → room` | Constructor. Calls `Object:new()`, adds room fields. |
| `get_appearance` | `room:get_appearance(session_id) → string` | Renders the full room view (title, description, exits, characters). |
| `add_character` | `room:add_character(char_id)` | Add a character to the room's contents. |
| `remove_character` | `room:remove_character(char_id)` | Remove a character from contents. |
| `get_characters` | `room:get_characters() → table` | Get a copy of all character IDs in the room. |
| `has_exit` | `room:has_exit(direction) → bool` | Check if an exit exists. |
| `get_exit` | `room:get_exit(direction) → string\|nil` | Get the target room ID for a direction. |
| `add_action` | `room:add_action(verb, func, hint)` | Register a room-scoped command. |
| `get_action` | `room:get_action(verb) → table\|nil` | Get the action entry for a verb. |
| `get_action_hints` | `room:get_action_hints() → table` | Array of hint strings for all actions. |
| `add_item` | `room:add_item(keyword, description)` | Add an examinable scenery object. |
| `get_item` | `room:get_item(keyword) → string\|nil` | Get a scenery item's description. |

### Inherited from Object

`resolve()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

---

## Item

**File:** [`mudlib/lib/item.lua`](file:///c:/Code/oxigeon/mudlib/lib/item.lua) — Inherits from **Object**

Base class for all tangible things: weapons, armor, potions, keys, treasure.

### Fields (own)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `weight` | number | `1` | Affects carry capacity. |
| `value` | number | `0` | Currency value (for shops, loot). |
| `stackable` | boolean | `false` | Can multiples merge in inventory. |
| `quantity` | number | `1` | Stack count (if stackable). |
| `slot` | string \| nil | `nil` | Equipment slot (`"weapon"`, `"head"`, `"chest"`, etc.). `nil` = not equippable. |
| `equippable` | boolean | auto | Defaults to `true` if `slot` is set. |
| `tags` | table | `{}` | Array of strings for filtering (e.g. `"quest"`, `"magical"`). |
| `on_use` | function \| nil | — | `function(item, user_id)` — called on "use" command. |
| `on_pickup` | function \| nil | — | `function(item, user_id)` — called when picked up. |
| `on_drop` | function \| nil | — | `function(item, user_id)` — called when dropped. |
| `on_equip` | function \| nil | — | `function(item, user_id)` — called when equipped. |
| `on_remove` | function \| nil | — | `function(item, user_id)` — called when unequipped. |

### Methods (own)

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `Item:new(data) → item` | Constructor. Calls `Object:new()`, adds item fields. |
| `is_equippable` | `item:is_equippable() → bool` | Check if item can be equipped. |
| `is_stackable` | `item:is_stackable() → bool` | Check if item can stack. |
| `has_tag` | `item:has_tag(tag) → bool` | Check if item has a specific tag. |
| `display_name` | `item:display_name() → string` | Short name, with `(xN)` suffix for stacks. |
| `examine` | `item:examine() → string` | Full examination text with weight, value, slot. |

### Inherited from Object

`resolve()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

---

## Weapon

**File:** [`mudlib/lib/weapon.lua`](file:///c:/Code/oxigeon/mudlib/lib/weapon.lua) — Inherits from **Item** → Object

Represents any weapon: swords, axes, bows, staves, daggers.

### Fields (own)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `damage` | table | `{ min=1, max=1 }` | Damage range. Accepts `{ min, max }`, `{ N, N }`, or a single number. |
| `speed` | number | `1.0` | Attack speed multiplier. |
| `weapon_type` | string \| nil | — | `"sword"`, `"axe"`, `"bow"`, `"staff"`, etc. |
| `damage_type` | string | `"physical"` | `"physical"`, `"fire"`, `"ice"`, `"magic"`. |
| `two_handed` | boolean | `false` | Requires both hand slots. |
| `range` | string | `"melee"` | `"melee"` or `"ranged"`. |
| `required_level` | number \| nil | — | Minimum level to equip. |
| `required_strength` | number \| nil | — | Minimum strength to equip. |
| `hit_message` | string \| function \| nil | — | Custom hit text (lfun). |
| `miss_message` | string \| function \| nil | — | Custom miss text (lfun). |
| `crit_message` | string \| function \| nil | — | Custom critical hit text (lfun). |

### Methods (own)

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `Weapon:new(data) → weapon` | Constructor. Defaults `slot` to `"weapon"`. |
| `roll_damage` | `weapon:roll_damage() → number` | Random damage between min and max. |
| `avg_damage` | `weapon:avg_damage() → number` | Average of min and max. |
| `dps` | `weapon:dps() → number` | Average damage × speed. |
| `meets_requirements` | `weapon:meets_requirements(stats) → bool, reason?` | Check level/strength requirements. |
| `examine` | `weapon:examine() → string` | Full text with damage, type, speed, element. **Overrides Item.** |

### Inherited from Item

`is_equippable()`, `is_stackable()`, `has_tag()`, `display_name()`

### Inherited from Object

`resolve()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

---

## Armor

**File:** [`mudlib/lib/armor.lua`](file:///c:/Code/oxigeon/mudlib/lib/armor.lua) — Inherits from **Item** → Object

Represents any defensive equipment: helmets, breastplates, shields, boots, gloves, robes.

### Fields (own)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `defense` | number | `1` | Base damage reduction. |
| `armor_type` | string | `"medium"` | `"cloth"`, `"light"`, `"medium"`, `"heavy"`. |
| `resist` | table | `{}` | Damage type → reduction value (e.g. `{ fire = 5, ice = -3 }`). |
| `required_level` | number \| nil | — | Minimum level to equip. |
| `required_strength` | number \| nil | — | For heavy armor. |
| `required_dexterity` | number \| nil | — | For light armor. |
| `stat_bonus` | table | `{}` | Passive stat bonuses while equipped (e.g. `{ max_hp = 20 }`). |

### Methods (own)

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `Armor:new(data) → armor` | Constructor. Defaults `slot` to `"chest"`. Warns if no slot set. |
| `get_defense` | `armor:get_defense() → number` | Base defense + any `defense_bonus` from object state. |
| `get_resist` | `armor:get_resist(damage_type) → number` | Resistance for a specific damage type (0 if none). |
| `meets_requirements` | `armor:meets_requirements(stats) → bool, reason?` | Check level/strength/dexterity requirements. |
| `encumbrance` | `armor:encumbrance() → number` | Weight class tier: cloth=0, light=1, medium=2, heavy=3. |
| `examine` | `armor:examine() → string` | Full text with defense, type, resistances, bonuses. **Overrides Item.** |

### Inherited from Item

`is_equippable()`, `is_stackable()`, `has_tag()`, `display_name()`

### Inherited from Object

`resolve()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

---

## Mobile

**File:** [`mudlib/lib/mobile.lua`](file:///c:/Code/oxigeon/mudlib/lib/mobile.lua) — Inherits from **Object**

Base class for all living entities: monsters, NPCs, shopkeepers, quest givers.

### Fields (own)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `stats` | table | (see below) | `{ hp, max_hp, mp, max_mp, strength, dexterity, intelligence, constitution, level }` |
| `faction` | string \| nil | — | For ally/enemy detection. |
| `race` | string \| nil | — | `"human"`, `"orc"`, `"undead"`, etc. |
| `gender` | string \| nil | — | `"male"`, `"female"`, `"neutral"`. |
| `title` | string \| nil | — | Optional title (e.g. `"the Blacksmith"`). |
| `inventory` | table | `{}` | Array of item IDs. |
| `equipment` | table | `{}` | Slot → item ID mapping. |
| `aggressive` | boolean | `false` | Attacks players on sight. |
| `stationary` | boolean | `false` | Never wanders from spawn room. |
| `unique` | boolean | `false` | Only one can exist at a time. |
| `echoes` | table | `{}` | Atmospheric messages: strings or `{ text, weight }`. |
| `echo_interval` | number | `30` | Seconds between echo rolls. |
| `patrol` | table \| nil | — | Array of room IDs for patrol route. |
| `patrol_interval` | number | `15` | Seconds between patrol moves. |
| `loot_table` | table | `{}` | Array of `{ item_id, chance }`. |
| `respawn_time` | number \| nil | — | Seconds to respawn after death. |
| `spawn_room` | string \| nil | — | Room to respawn in. |
| `dialogue` | table | `{}` | Keyword → response (string or function). |
| `skills` | table | `{}` | Skill name → level mapping. |
| `tags` | table | `{}` | Array of strings (e.g. `"boss"`, `"merchant"`). |
| `on_death` | function \| nil | — | `function(mob, killer_id)` |
| `on_combat` | function \| nil | — | `function(mob, target_id)` |
| `on_interact` | function \| nil | — | `function(mob, user_id, verb)` |
| `on_spawn` | function \| nil | — | `function(mob, room_id)` |

**Default stats:** `hp=10, max_hp=10, mp=0, strength=5, dexterity=5, intelligence=5, constitution=5, level=1`

### Methods (own)

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `Mobile:new(data) → mob` | Constructor. Calls `Object:new()`, adds all mobile fields. |
| `is_alive` | `mob:is_alive() → bool` | True if HP > 0. |
| `take_damage` | `mob:take_damage(amount) → number` | Apply damage, clamp to 0. Returns remaining HP. |
| `heal` | `mob:heal(amount) → number` | Restore HP, clamp to max. Returns new HP. |
| `get_level` | `mob:get_level() → number` | Get the mobile's level. |
| `roll_echo` | `mob:roll_echo() → string\|nil` | Pick a weighted random echo. Supports lfun resolution. |
| `has_item` | `mob:has_item(item_id) → bool` | Check if item is in inventory. |
| `add_item` | `mob:add_item(item_id)` | Add an item to inventory. |
| `remove_item` | `mob:remove_item(item_id) → bool` | Remove an item. Returns true if found. |
| `has_tag` | `mob:has_tag(tag) → bool` | Check for a tag. |
| `is_aggressive` | `mob:is_aggressive() → bool` | Check aggressive flag. |
| `get_dialogue` | `mob:get_dialogue(keyword) → string\|nil` | Get a dialogue response (lfun-resolved). |
| `get_skill` | `mob:get_skill(skill) → number` | Get skill level (0 if unlearned). |
| `set_skill` | `mob:set_skill(skill, level)` | Set skill level. |
| `examine` | `mob:examine() → string` | Full text with race, faction, level. |

### Inherited from Object

`resolve()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

---

## Player

**File:** [`mudlib/lib/player.lua`](file:///c:/Code/oxigeon/mudlib/lib/player.lua) — Inherits from **Mobile** → Object

Represents a logged-in player. Bridges live game objects with database persistence via CHARACTER_D.

### Persistent vs. Transient Fields

| Persistent (saved to DB) | Transient (runtime only) |
|---|---|
| stats, inventory, equipment | session_id |
| gold, xp, quest_flags, skills | char_id, account_id, name |
| title, race, gender, tags | current combat target |
| custom (open-ended table) | room (tracked by WORLD_D) |

The `SAVE_FIELDS` table declares which fields are serialized by `to_save()`.

### Fields (own — beyond Mobile)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `char_id` | number | — | Database character ID. **Transient.** |
| `account_id` | number | — | Database account ID. **Transient.** |
| `name` | string | — | Character name (from DB record). **Transient.** |
| `session_id` | string \| nil | — | Current session. **Transient.** |
| `gold` | number | `0` | Currency. **Persistent.** |
| `xp` | number | `0` | Experience points. **Persistent.** |
| `quest_flags` | table | `{}` | Flag name → value. **Persistent.** |
| `custom` | table | `{}` | Open-ended game-specific data. **Persistent.** |


**Default starting stats:** `hp=100, max_hp=100, mp=50, max_mp=50, strength=10, dexterity=10, intelligence=10, constitution=10, level=1`

### Methods (own)

| Method | Signature | Description |
|--------|-----------|-------------|
| `from_save` | `Player:from_save(char_id, char_info, saved) → player` | Hydrate from DB records. Layers defaults → saved data → identity. |
| `to_save` | `player:to_save() → table` | Serialize persistent fields to a JSON-safe table. |
| `save` | `player:save() → bool` | Convenience — calls `CHARACTER_D.save(self.char_id)`. |
| `award_xp` | `player:award_xp(amount)` | Add XP. Emits `"player.xp_gained"` event. |
| `award_gold` | `player:award_gold(amount)` | Add gold. |
| `spend_gold` | `player:spend_gold(amount) → bool` | Deduct gold. Returns false if insufficient. |
| `set_quest_flag` | `player:set_quest_flag(flag, value?)` | Set a quest flag (defaults to `true`). |
| `get_quest_flag` | `player:get_quest_flag(flag) → any` | Get a quest flag value. |
| `has_quest_flag` | `player:has_quest_flag(flag) → bool` | Check if a flag is set (truthy). |
| `display_name` | `player:display_name() → string` | Name + title. **Overrides Mobile.** |
| `examine` | `player:examine() → string` | Player-specific examination text. **Overrides Mobile.** |

### Inherited from Mobile

`is_alive()`, `take_damage()`, `heal()`, `get_level()`, `roll_echo()`, `has_item()`, `add_item()`, `remove_item()`, `has_tag()`, `is_aggressive()`, `get_dialogue()`, `get_skill()`, `set_skill()`

### Inherited from Object

`resolve()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

---

## Method Override Summary

Some classes override methods defined by their parent. When overridden, the child's version is called:

| Method | Object | Room | Item | Weapon | Armor | Mobile | Player |
|--------|--------|------|------|--------|-------|--------|--------|
| `new` | ✦ | ✦ | ✦ | ✦ | ✦ | ✦ | `from_save` |
| `examine` | — | `get_appearance` | ✦ | ✦ | ✦ | ✦ | ✦ |
| `display_name` | — | — | ✦ | ↑ | ↑ | — | ✦ |
| `has_tag` | — | — | ✦ | ↑ | ↑ | ✦ | ↑ |
| `meets_requirements` | — | — | — | ✦ | ✦ | — | — |
| `resolve` | ✦ | ↑ | ↑ | ↑ | ↑ | ↑ | ↑ |
| `get_state` | ✦ | ↑ | ↑ | ↑ | ↑ | ↑ | ↑ |
| `set_state` | ✦ | ↑ | ↑ | ↑ | ↑ | ↑ | ↑ |

**Legend:** ✦ = defines the method · ↑ = inherits from parent · — = not applicable
