# Object Hierarchy

Oxigeon provides a class hierarchy for all game entities. Every object in the MUD — rooms, items, weapons, monsters, players — inherits from a shared `Object` base class, gaining common fields and driver state access.

## Inheritance Tree

```
Object (mudlib/lib/object.lua)
│   Base for all MUD objects. Provides id, short, description,
│   the lfun resolve() pattern, and driver state access.
│
├── Room (mudlib/lib/room.lua)
│       Exits, contents, actions, scenery items, appearance rendering.
│
├── Item (mudlib/lib/item.lua)
│       Weight, value, slot, stackability, interaction hooks, tags.
│       Roles are COMPONENTS, not subclasses — see the note below.
│
└── Mobile (mudlib/lib/mobile.lua)
    │   Stats, inventory, equipment, AI behavior, echoes, patrol,
    │   loot table, respawn, dialogue, faction.
    │
    └── Player (mudlib/lib/player.lua)
            Database persistence (from_save/to_save), XP, gold,
            quest flags, session link.
```

> [!IMPORTANT]
> **`Weapon` and `Armor` are no longer classes.** They are *archetypes* —
> functions that build a plain `Item` carrying data-only components:
>
> ```lua
> local Weapon = require('components.weapon')
> local dagger = Weapon{ id = "silver_dagger", damage = {2, 8},
>                        damage_type = "magic", required_level = 3 }
>
> dagger.weapon     --> { min = 2, max = 8, speed = 1.0, damage_type = "magic", ... }
> dagger.requires   --> { level = 3 }
> Weapon.is(dagger) --> true
> Weapon.roll_damage(dagger, roll)
> ```
>
> Every method those classes had was a pure function of the item's own data, so
> the inheritance bought the method-call syntax and nothing else — while an item
> that is a weapon *and* a light source *and* a quest token had no single class
> it could be. Three parts, kept apart:
>
> | | Holds | Rule |
> |---|---|---|
> | **Archetype** `Weapon{...}` | construction | flat authoring data in, an `Item` out |
> | **Component** `item.weapon` | data | no functions, no metatables; its presence is the has-component test |
> | **System** `components/weapon.lua` | behaviour | module functions taking the item — never installed on the instance |
>
> This is the rule [`effect_d`](./effects.md) already follows for definitions
> versus instances, applied to the object model. Authoring does not change: an
> area file writes the same flat table it always did.

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
| `stats` | table \| nil | — | Where [traits](./traits.md) are stored. Copied from `data.stats`; left absent when nothing was authored, because storage is what decides which traits an entity has. |

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `Object:new(data) → obj` | Constructor. Initializes id, short, description and `stats` from a data table. |
| `resolve` | `Object.resolve(value, obj) → string\|nil` | **Static.** Resolves lfun properties: returns strings as-is, calls functions with `obj` as argument. |
| `trait` | `obj:trait(id) → number` | The effective value of a [trait](./traits.md), after the trait graph and every effect have had their say. The default when the object does not hold it. |
| `has_trait` | `obj:has_trait(id) → bool` | Whether the object holds it at all — a different question from what it is worth. |
| `get_state` | `obj:get_state(key) → any` | Get a driver-side state value, scoped to this object's `id`. |
| `set_state` | `obj:set_state(key, value)` | Set a driver-side state value. |
| `get_all_state` | `obj:get_all_state() → table\|nil` | Get all state key/values for this object. |
| `clear_state` | `obj:clear_state()` | Clear all state for this object. |

> [!IMPORTANT]
> **`trait()` lives here, on `Object`, not on `Mobile`.** It was `Mobile:stat`
> until a trait stopped meaning "a character statistic": a sword's durability
> and a room's corruption are traits in exactly the sense a mob's strength is,
> and one accessor should answer for all of them. The rename was hard, with no
> alias — this codebase deleted `create_sandboxed_env` to have one boundary
> rather than two.
>
> `obj.stats[id]` is the *stored* value, which for a buffed or derived trait is
> the wrong answer. Read `obj:trait(id)`.

---

## Room

**File:** [`mudlib/lib/room.lua`](file:///c:/Code/oxigeon/mudlib/lib/room.lua) — Inherits from **Object**

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

`resolve()`, `trait()`, `has_trait()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

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

`resolve()`, `trait()`, `has_trait()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

---

## Weapon — archetype, component, system

**File:** [`mudlib/components/weapon.lua`](file:///c:/Code/oxigeon/mudlib/components/weapon.lua) — **not a class.** Produces an `Item`.

```lua
local Weapon = require('components.weapon')

local dagger = Weapon{ id = "silver_dagger", short = "a silver dagger",
                       damage = {2, 8}, speed = 1.2, damage_type = "magic",
                       required_level = 3 }
```

### Authoring fields

Unchanged from the class this replaces. `damage` accepts `8`, `{2, 8}` or
`{ min = 2, max = 8 }`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `damage` | number \| table | `1` | Damage spread. |
| `speed` | number | `1.0` | Attack speed multiplier. |
| `weapon_type` | string \| nil | — | `"sword"`, `"axe"`, `"bow"`, `"staff"`, etc. |
| `damage_type` | string | `"physical"` | `"physical"`, `"fire"`, `"ice"`, `"magic"`. |
| `two_handed` | boolean | `false` | Requires both hand slots. |
| `range` | string | `"melee"` | `"melee"` or `"ranged"`. |
| `required_level` / `required_strength` / `required_dexterity` | number \| nil | — | Becomes the [`requires`](#requires) component. |
| `hit_message` / `miss_message` / `crit_message` | string \| function \| nil | — | Combat text (lfun). |

### The `item.weapon` component

`{ min, max, speed, weapon_type, damage_type, two_handed, range, hit_message, miss_message, crit_message }`

`item.weapon ~= nil` **is** the has-component test.

> [!WARNING]
> The three message fields are lfuns and may hold functions. That is safe only
> because they live on a **template**, which is code and is never serialized.
> An item *instance* must not carry them — the same definitions-versus-instances
> rule [`effect_d`](./effects.md) follows.

### The system

| Function | |
|---|---|
| `Weapon{data}` / `Weapon.new(data)` | build an `Item` with the component |
| `Weapon.is(item)` | `-> boolean` |
| `Weapon.roll_damage(item, roll)` | `-> number \| nil`. **`roll` is injected** — pass `DAEMON.combat._roll` so a pinned fight stays pinned. The class version called `math.random` directly, which made it a second source of randomness nothing could override. |
| `Weapon.avg_damage(item)` / `Weapon.dps(item)` | `-> number \| nil` |
| `Weapon.from_data(data)` | build just the component |
| `Weapon.describe(item)` | `-> array of examine lines` |

There is deliberately no `Weapon:new(...)`; the colon form would suggest a class.

---

## Armor — archetype, component, system

**File:** [`mudlib/components/armor.lua`](file:///c:/Code/oxigeon/mudlib/components/armor.lua) — **not a class.** Produces an `Item`.

```lua
local Armor = require('components.armor')

local cloak = Armor{ id = "warded_cloak", short = "a warded cloak", slot = "back",
                     defense = 4, armor_type = "cloth", resist = { magic = 6 } }
```

### Authoring fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `defense` | number | `1` | Base damage reduction. |
| `armor_type` | string | `"medium"` | `"cloth"`, `"light"`, `"medium"`, `"heavy"`. |
| `resist` | table | `{}` | Damage type → reduction. Negative is a weakness: `{ fire = 5, ice = -3 }`. |
| `stat_bonus` | table | `{}` | Passive trait bonuses while worn, e.g. `{ max_hp = 20 }`. |
| `required_level` / `required_strength` / `required_dexterity` | number \| nil | — | Becomes the [`requires`](#requires) component. |

Omitting `slot` logs a warning and defaults to `"chest"`.

### The `item.armour` component

`{ defense, armor_type, resist, stat_bonus }` — note the component key is
`armour` while the module is `armor.lua`, so it does not collide with the
`armor_type` field inside it.

### The system

| Function | |
|---|---|
| `Armor{data}` / `Armor.new(data)` | build an `Item` with the component |
| `Armor.is(item)` | `-> boolean` |
| `Armor.defense(item)` | base + any `defense_bonus` from object state |
| `Armor.resist(item, damage_type)` | `-> number`, **0 when unmentioned**, so callers add unconditionally |
| `Armor.encumbrance(item)` | cloth 0, light 1, medium 2, heavy 3 |
| `Armor.from_data(data)` / `Armor.describe(item)` | |

> [!NOTE]
> `Armor.defense` reads `defense_bonus` from object state, which is keyed on the
> item's `id`. While items are shared registry templates, every copy of a
> breastplate shares one enchantment. Per-instance identity is what fixes it.

---

## Requires

**File:** [`mudlib/components/requires.lua`](file:///c:/Code/oxigeon/mudlib/components/requires.lua)

One requirement check, shared by every kind of item. `Weapon` and `Armor` each
carried their own near-identical `meets_requirements`; Armor tested dexterity
and Weapon did not, for no recorded reason. Now every item gets all three.

```lua
item.requires = { level = 3, strength = 16, dexterity = 12 }   -- absent = unconstrained
```

| Function | |
|---|---|
| `Requires.met(item, source)` | `-> boolean, reason?`. An item with no component is always usable. |
| `Requires.from_data(data)` | returns **nil** when nothing is required, so the component stays absent rather than becoming an empty table |
| `Requires.describe(item)` | the `examine` line, or nil |

`source` may be an entity (anything answering `:trait(id)`), a plain stats table,
or an entity with a `.stats` table. **The entity form is the correct one** —
`entity.stats[id]` is the *stored* value, which for a buffed or derived trait is
the wrong answer. Refusal messages are emitted in a fixed order, so a character
short on two counts is always told about the same one first.

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
| `tags` | table | `{}` | Array of strings (e.g. `"boss"`, `"merchant"`). |
| `on_death` | function \| nil | — | `function(mob)`. Who did it is `mob._killed_by`, an identity table `{char_id, id}` — not the entity, which would make two fighters a reference cycle. |
| `on_combat` | function \| nil | — | `function(mob, target)` — the target entity, not an id |
| `on_interact` | function \| nil | — | `function(mob, user_id, verb)` |
| `on_spawn` | function \| nil | — | `function(mob, room_id)` |

**Default stats:** `hp=10, mp=0, strength=5, dexterity=5, intelligence=5, constitution=5, level=1`

> [!NOTE]
> There is no `max_hp` default, and authoring one does nothing: `max_hp` is a
> [derived trait](./traits.md) and `attach` deletes any value stored under one.
> A creature that needs a specific maximum sets **`max_hp_flat`** — see
> [Creatures & Combat](./combat.md#authored-health--max_hp_flat).

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
| `examine` | `mob:examine() → string` | Full text with race, faction, level. |

> [!NOTE]
> **`get_skill` / `set_skill` and the `skills` table are gone.** They existed as
> a parallel `skill -> level` map only because traits could not be sparse. A
> skill is a `category = "skill"` trait the entity happens to hold now, so it
> reads through `mob:trait("swordsmanship")` and gains clamping, bounds and a
> derived mastery for free. See [Traits](./traits.md).

### Inherited from Object

`resolve()`, `trait()`, `has_trait()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

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

`is_alive()`, `take_damage()`, `heal()`, `get_level()`, `roll_echo()`, `has_item()`, `add_item()`, `remove_item()`, `has_tag()`, `is_aggressive()`, `get_dialogue()`

### Inherited from Object

`resolve()`, `trait()`, `has_trait()`, `get_state()`, `set_state()`, `get_all_state()`, `clear_state()`

---

## Method Override Summary

Some classes override methods defined by their parent. When overridden, the child's version is called:

| Method | Object | Room | Item | Mobile | Player |
|--------|--------|------|------|--------|--------|
| `new` | ✦ | ✦ | ✦ | ✦ | `from_save` |
| `examine` | — | `get_appearance` | ✦ | ✦ | ✦ |
| `display_name` | — | — | ✦ | — | ✦ |
| `has_tag` | — | — | ✦ | ✦ | ↑ |
| `resolve` | ✦ | ↑ | ↑ | ↑ | ↑ |
| `get_state` | ✦ | ↑ | ↑ | ↑ | ↑ |
| `set_state` | ✦ | ↑ | ↑ | ↑ | ↑ |

**Legend:** ✦ = defines the method · ↑ = inherits from parent · — = not applicable

`Weapon` and `Armor` have no column: they are not classes and override nothing.
Their behaviour is module functions, and `meets_requirements` is gone entirely —
both copies became the shared [`Requires.met`](#requires).
