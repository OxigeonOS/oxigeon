# Items, Equipment & Containers

An item exists in two forms, and the difference is the thing to hold on to:

| | | |
|---|---|---|
| **Template** | `DAEMON.items.get("brass_key")` | Shared, never mutated, one per item id. *What is a brass key?* |
| **Instance** | `DAEMON.items.spawn("brass_key", where)` | A particular one, with its own id and its own location. *Which brass key, and where is it?* |

Only templates existed until recently. Items lived as entries in
`player.inventory` and as rows in the registry, and **nothing could put one on a
floor** — there was no `get`, no `drop`, no `put`, no `give`, no `use`, no
`examine`. Combat loot "went straight to the killer" because there was nowhere
else for it to go, and `Item.on_pickup` / `on_drop` / `on_use` were declared and
had never once been called.

## Where an item is

Locations are scheme-prefixed strings, so one field answers "where is this" for
every kind of somewhere:

```lua
DAEMON.items.location("room", "thornhollow.square")  --> "room:thornhollow.square"
DAEMON.items.location("item", backpack.id)           --> "item:<uuid>"
```

| Home | Held by | Persisted |
|---|---|---|
| the floor | `item_d`'s index, `location = "room:<id>"` | **no** — memory tier |
| a container | `item_d`'s index, `location = "item:<instance id>"` | only if the container is carried |
| a character | an entry in `player.inventory` | yes, by CHARACTER_D |

Ground items are memory-tier state by the rule in
[state-cache.md](./state-cache.md): if the server restarts, the sword someone
left in the square is gone and the area reset puts the world back. A container
in somebody's pack is the case that spans both homes — its *contents* are always
indexed under `item:<its id>` whichever home the container itself is in, which
is what makes `put` and `get from` the same code path as `drop` and `get`. On
save the contents are folded onto the entry and on load they are put back, so a
backpack does not empty itself overnight.

### Instance ids are uuids

A mob instance is `"mob:" .. seq`, because a mob is never saved and a counter is
enough. An item instance is `"item:" .. uuid()`, because a container in
somebody's inventory **is** saved — and a counter restarting at zero on every
boot would hand out an id that already means something else in a save file.

## The verbs

All of them go through `lib/carry.lua`, which is the one place that knows how to
move an item between the floor, a character and a container. Five commands
asking four questions between them — *what did they mean*, *may it move*, *move
it*, *who should be told* — written five times would drift, so they are written
once.

| Command | |
|---|---|
| `get <item>` / `get all` / `get <item> from <container>` | pick up |
| `drop <item>` / `drop all` | put down |
| `put <item> in <container>` | store |
| `give <item> to <player>` | hand over |
| `use <item> [on <target>]` | `Item.on_use`; opens a container that has no hook |
| `examine <item>` | everything the item's components have to say |
| `inventory` | what you are carrying |

Each fires the documented hook and event:

| Verb | Hook | Event |
|---|---|---|
| `get` | `on_pickup(item, char_id)` | `item.picked_up` |
| `drop` | `on_drop(item, char_id)` | `item.dropped` |
| `put` | — | `item.stored` |
| `give` | — | `item.given` |
| `use` | `on_use(item, char_id, target)` | `item.used` |
| `wear`/`wield` | `on_equip` | `item.equipped` |
| `remove` | `on_remove` | `item.unequipped` |

A hook that raises is logged and does not take the verb down with it: dropping a
sword must work even if the sword's `on_drop` is broken.

## Equipment

```
wear <item>     wield <weapon>     remove <item|slot|all>     equipment
```

Slots: `head neck chest back hands waist legs feet weapon offhand light ring`.

`wear` and `wield` are the same operation with different words and different
refusals — you wield a weapon and wear everything else. Both go through
`equipment.equip`, so the requirement check, the displaced-item handling and the
effect source are written once.

A two-handed weapon occupies `weapon` **and** `offhand`, in both directions:
wielding one clears a shield, and putting a shield on clears it. Displaced
pieces are named out loud, because "you wield the greatsword" while a shield
silently comes off is how a player loses track of what they are holding.

### Requirements

```lua
Armor{ id = "guards_mail", slot = "chest", defense = 8,
       required_level = 5, required_strength = 14 }
```

One rule, in [`components/requires.lua`](./object-hierarchy.md#requires), shared by
weapons and armour. Read through the entity rather than its stored stats, so a
strength buff genuinely lets you lift the greatsword:

```
> wield greatsword
Requires 16 strength
```

### What wearing something does — the `equip:` source

Bonuses are **effects**, applied through the documented source pattern:

```lua
DAEMON.effect.set_source_effects(entity, "equip:chest", specs)
```

*The effects from this source are now exactly these.* Idempotent, so it is safe
on every login and every slot change without working out what it did last time.
`persist = false`, so nothing is ever written — what is worn is saved, and the
aura is derived from it. Persisting the aura as well would be a second copy of
the truth that can disagree with the first.

Two things a worn piece contributes:

| Field | Becomes |
|---|---|
| `stat_bonus = { intelligence = 2 }` | a `trait:intelligence` handler in the `add` phase |
| `defense` + `resist = { magic = 6 }` | a `damage_taken` handler in the **`reduce`** phase |

> [!NOTE]
> **The effect definitions are generated, and that is deliberate.** An effect's
> hooks are fixed when it is *defined*, and a trait modifier is a `trait:<id>`
> hook — so one "equipment aura" definition cannot modify strength on one
> character and wisdom on another. One definition per trait any gear actually
> touches (`equip_trait_<id>`) is created on demand, with the amount carried per
> instance in `state`. Protection needs only one (`equip_protection`), because
> `damage_taken` is a single hook whatever the damage type.
>
> A `stat_bonus` aimed at a gauge or a counter is refused with a message naming
> the item's field. Effects modify attributes and derived traits; to raise a
> gauge's ceiling, modify the trait that is its `max`.

### Armour finally mitigates

The `reduce` phase, so a percentage multiplier lands first. That is the ordering
[effects.md](./effects.md) argues for, and it is why a 30-point hit against
stoneskin and a leather jerkin yields 17 rather than depending on which one you
picked up first:

```
30 * 0.85 = 25.5      mult phase   (stoneskin)
     - 5             reduce phase  (stoneskin's flat reduction)
     - 3             reduce phase  (the jerkin)
   = 17.5 -> 17
```

`resist` is looked up by the damage type on the event, so a warded cloak blunts
a silver dagger and does nothing at all against a sword. A negative entry is a
weakness and increases the number, which is the same arithmetic.

## Containers

```lua
local Container = require('components.container')

Container{ id = "leather_backpack", short = "a battered leather backpack",
           slot = "back", capacity = 12, capacity_weight = 40 }

Container{ id = "iron_strongbox", capacity = 6,
           closeable = true, starts_closed = true,
           key = "brass_key", starts_locked = true }
```

| Field | |
|---|---|
| `capacity` | how many items fit; **0 = unlimited**, which a corpse wants |
| `capacity_weight` | total weight; 0 = unlimited |
| `closeable` / `starts_closed` | a corpse cannot be shut; a chest can |
| `key` / `starts_locked` | the template id of the key that opens it |

Open, closed and locked are **per instance** and live in object state, not in
the component: the component is shared with the template, so writing `closed`
onto it would shut every chest in the game at once.

Refusals name the problem, because "the backpack is full" and "that is too heavy
for the backpack" have different fixes and a player told only "you can't" will
try the same thing again.

A container cannot end up inside itself at any depth. `put bag in box` then `put
box in bag` is two legal-looking moves that between them would make a cycle
nothing could ever reach again, so `move` walks the chain rather than making one
comparison.

## Weight

`carry_capacity` is a **trait**, not a constant, so a strength buff or a bag of
holding changes it through the ordinary effect path and nothing in `carry.lua`
has to know. A game that has not defined that trait does not get encumbrance,
which is the right default — it should not appear because a library was linked.

Container weight is recursive: a backpack inside a chest inside a cart is a
thing players will build the moment containers exist, and a check that looked
one level down would let them carry the world in a satchel.

## API

| Function | |
|---|---|
| `register(item)` / `register_all(list)` | templates |
| `get(id)` / `all()` | the registry |
| `resolve(entry)` | template + instance overrides |
| `spawn(template_id, location, overrides)` | make an instance |
| `move(instance, location)` | `nil` takes it out of the index without destroying it |
| `destroy(instance)` | it and everything in it, plus their object state |
| `get_instance(id)` | |
| `in_room(room_id)` / `find_in_room(room_id, name)` | |
| `contents(container_id)` / `find_in_container(container_id, name)` | |
| `find_by_name(name, inventory)` | search an inventory array |
| `location(kind, id)` / `split_location(loc)` | build and read a location string |
| `count()` | live instances, for `mudstatus` and for a leak test |

`lib/carry.lua`: `find`, `take`, `drop`, `put_in`, `give`, `fire_hook`,
`carried_weight`, `carry_capacity`, `can_carry`, `pack`, `unpack`, `release`.

`lib/equipment.lua`: `equip`, `unequip`, `worn`, `all_worn`, `refresh_slot`,
`refresh_all`, `slot_for`, `is_slot`, `encumbrance`, `SLOTS`.

`components/container.lua`: `is`, `is_closed`, `is_locked`, `set_closed`, `set_locked`,
`can_accept`, `total_weight`, `describe`.

## What it will not do

- **No stacking of instances.** `stackable` and `quantity` are on the template
  and the inventory display groups by name, but two instances are two objects.
  Merging them would mean deciding which one's per-instance state survives.
- **No giving to an NPC.** A quest turn-in and a shopkeeper's appraisal are game
  decisions and belong in an `on_interact` handler, not in a mudlib verb that
  would have to guess which one was meant.
- **No ground items surviving a restart.** That is the memory tier, chosen
  deliberately; an area reset is what puts the world back.
