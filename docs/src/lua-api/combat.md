# Creatures & Combat

```
> look
The Pantry
Shelves sag under jars of things best left unidentified.
Obvious exits: north
a grey rat is here.
a grey rat is here.

> attack rat
You attack rat!
You hit rat for 4 damage.
rat hits you for 3 damage.
```

Two daemons: `mob_d` owns creatures, `combat_d` owns fights.

## Why this exists

Mostly so the [trait](./traits.md) and [effect](./effects.md) systems have
something real to act on. An attack resolves through the same pipeline
everything else uses, which means "take 15% less damage" is visible in the
numbers a player sees rather than only in a test.

It is deliberately small, and the list of what it does not do is part of the
design: no initiative, no groups, no positioning, no ranged weapons, no spell
casting, no aggro, no directional fleeing. One attacker, one target, one shared
round timer. Anything more is a game's decision, not a driver's.

## Creatures

Templates are plain data, authored per area exactly as rooms and items are:

```lua
-- game/areas/wizard_workshop/mobs.lua
return {
    {
        id          = "workshop_rat",
        name        = "rat",
        short       = "a grey rat",
        description = "A scrawny grey rat with matted fur and clever, wary eyes.",
        -- `max_hp_flat`, not `max_hp`: see "Authored health" below.
        stats       = { hp = 24, max_hp_flat = 24, strength = 6, dexterity = 12,
                        constitution = 8, level = 1 },
        damage      = { min = 2, max = 5 },
        xp_award    = 12,
        spawn_room  = "wizard_workshop.pantry",
        count        = 2,
        respawn_time = 120,
        loot_table  = { { item_id = "empty_vial", chance = 0.35 } },
    },
}
```

Register and populate from `game/init.lua`:

```lua
DAEMON.mobs.register_all(require('areas.wizard_workshop.mobs'))
DAEMON.mobs.populate()
```

A template is shared and never mutated; `spawn` turns one into a live `Mobile`
with its own id, its own health and its own effects. Wounding one rat does not
wound every rat.

`populate()` is idempotent — a template already at its `count` is left alone —
so it is safe to call on an area reset without the world filling up with rats.

> [!NOTE]
> **Mobs are never saved.** If the server restarts, the rat is a new rat. That
> is the [durability rule](./state-cache.md) applied rather than an oversight,
> and it is why nothing in `mob_d` touches the database.

### Authored health — `max_hp_flat`, not `max_hp`

A creature's maximum health goes in **`max_hp_flat`**. Writing `max_hp` does
nothing at all, and used to do nothing silently.

`max_hp` is [derived](./traits.md): `50 + constitution * 5 + (level - 1) * 10`.
A derived trait stores nothing, and `attach` deletes any value it finds under
one — a real migration, because `max_hp` used to be stored and a saved number
would shadow the formula forever. So a template that said `max_hp = 24` had that
value erased at spawn, and the rat came out at 90. Every mob in the game was
between 1.1× and 5.8× the toughness its own file claimed.

The numbers were not reachable by tuning either. The formula starts at 50, so
the weakest creature the curve can describe has **55** hit points — a 24-point
rat could not be expressed at all. The curve is shaped for a character, and a
rat is not a level-1 player.

```lua
stats = { hp = 24, max_hp_flat = 24, constitution = 8, level = 1 }
--> max_hp = 24, and the rat spawns at 24/24
```

Leave it out and the curve applies, which is what every player does:

```lua
stats = { hp = 40, constitution = 12, level = 3 }
--> max_hp = 130, from the formula
```

`max_hp_flat` is an ordinary attribute, so a boss buff is an ordinary effect:

```lua
{ id = "swollen", modifiers = { max_hp_flat = 50 } }
```

> [!IMPORTANT]
> Because `max_hp` *depends* on `max_hp_flat`, an entity needs it present to
> have `max_hp` — and a gauge whose ceiling is absent is itself absent, so it
> would have no `hp` either and `is_alive()` would say it is already dead.
> `DAEMON.trait.seed(entity, "character")` is what puts it there. Both real
> paths already do this (`lib/player.lua` on load, `mob_d.spawn` on spawn); a
> hand-built combatant in a test needs to do it too.

| Function | |
|---|---|
| `register(template)` / `register_all(list)` | |
| `spawn(template_id, room_id)` | returns the Mobile |
| `despawn(mob, { respawn = true })` | |
| `in_room(room_id)` | every live mob there, stably ordered |
| `find_in_room(room_id, name)` | prefix match — what `attack rat` uses |
| `move(mob, room_id)` | |
| `populate()` | spawn everything to its declared count |
| `describe_room(room_id)` | the lines `look` appends |

## Fights

```lua
DAEMON.combat.engage(attacker, target)
```

Both sides are engaged, so the target fights back — unless it is already busy
with someone else. One shared ticker (`game.combat_round_seconds`, default 3)
resolves every active pair; `attack` also swings immediately, rather than making
a player wait out a round for something they just chose to do.

A round is:

1. **To hit** — 60%, adjusted by the difference in dexterity, clamped to 5–95%
   so nothing is ever certain in either direction.
2. **Damage** — a wielded weapon's `roll_damage()`, else the template's own
   spread, else bare hands scaled by strength.
3. **The defender's `damage_taken` pipeline**, which is where mitigation happens.
4. **`TRAIT_D.adjust(hp, -n)`**, which settles regeneration first.
5. **Death**, if that took them to zero.

Death gives experience through `Player:award_xp` — so an experience buff applies
and combat does not have to know it exists — drops loot from the template, and
schedules the mob's respawn. A player's death is `death_d`'s business, which is
already listening for the event.

| Function | |
|---|---|
| `engage(attacker, target)` | `-> ok, reason` |
| `disengage(entity)` | |
| `disengage_all(char_id)` | everything fighting them stops too |
| `is_fighting(entity)` / `target_of(entity)` | |
| `attack_once(attacker, target)` | one exchange, for a caller that wants control |
| `round()` | what the ticker calls |

> [!IMPORTANT]
> **Combat state is memory-tier and never written.** A target, an engagement,
> who swung last — if the server restarts the fight is over, which is the
> correct answer. Writing any of it would be exactly the mistake the state cache
> exists to prevent.

### Deterministic tests

`DAEMON.combat._roll` is the only source of randomness, and it is replaceable:

```lua
-- always hit, always for the top of the range
DAEMON.combat._roll = function(n) if n == 100 then return 1 else return n end end
```

A test that depends on `math.random` is a test that fails one morning for no
reason.

## Commands

| Command | |
|---|---|
| `attack <target>` (`kill`, `k`) | engage and swing |
| `flee` (`retreat`) | break off; both sides stop |
| `score` (`sc`, `stats`) | every trait, base and effective |
| `effects` (`buffs`, `affects`) | what is on you, and for how long |
| `affect …` | admin: apply, damage, heal, xp, settle, traits, cache |

`affect` is the diagnostic window onto all of this — and the injection point the
real-mudlib tests use, since there is no other verb in the game that deals
damage on demand.

## Configuration

```toml
[game]
combat_round_seconds = 3
```

Set it to 0 to disable the round ticker entirely — which is what the test
harness does, so a fight never resolves in the background of an unrelated test.

## What it will not do

- **No initiative or turn order.** Every engaged pair resolves once per round,
  in a stable order.
- **No group combat or tanking.** One attacker, one target. *Assist* is
  possible but is not the driver's: `combat.started` is emitted, and a game
  daemon that wants a guard's faction to join in listens to it.
- **No fleeing to a direction, no pursuit.** `flee` ends the fight where you
  stand.
- **No corpses.** Loot falls on the floor of the room the fight was in — see
  [Items](./items.md). It used to go straight to the killer, and that was never
  a design decision: nothing in the mudlib could put an item in a room.
- **No aggression policy.** `aggressive` is read now, but by the *game* layer.
  The driver ships the flag and the `room.entered` event; whether an aggressive
  creature attacks, how long it waits, whether it cares about level or faction,
  and whether it gives up when you flee are content decisions. See
  `game/daemons/aggro_d.lua` for one game's answer — a different game writes a
  different file rather than configuring that one into shapelessness.
