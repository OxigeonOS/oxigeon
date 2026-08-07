# Spawners — Places That Produce Creatures

```lua
-- game/areas/wizard_workshop/rooms.lua
{
    id    = "wizard_workshop.pantry",
    short = "Reagent Pantry",
    items = {
        nest = "Behind the lowest shelf, a heap of shredded parchment...",
    },

    spawn_max      = 3,
    spawn_interval = 45,
    spawn_table    = {
        { template = "black_rat",    weight = 5 },
        { template = "scrawny_rat",  weight = 3 },
        { template = "muscular_rat", weight = 1 },
    },
}
```

Three ordinary room fields. There is no `spawners.lua` and no fourth generated
kind, which is what lets OLC author one today: `olc set spawn_max 4` and
`olc set spawn_table.black_rat 5` work, `verify` checks the templates exist, and
the generated file round-trips it.

## Not the same thing as `spawn_room`

Both exist, and they answer different questions.

| | |
|---|---|
| `mob.spawn_room` + `mob.count` | a **fixed population**. Bellow is in the smithy and there is one of her. `populate()` tops it up and is idempotent. |
| `room.spawn_*` | a **source**. This nest makes rats, of these kinds, up to this many, over time. |

The difference shows in the cap. `populate()` counts *per template*, so three rat
templates at `count = 2` is six rats — and there is no way to write "six rats of
any kind is too many for one pantry". A spawner's cap spans its whole table, and
that is the thing that could not be said before.

> [!WARNING]
> **A template in a spawn table must not also carry `respawn_time` or
> `spawn_room`.** `mob_d` schedules a respawn when such a creature dies *and* the
> spawner tops up on its own clock, so the room drifts past `spawn_max` one kill
> at a time — slowly enough to read as a balance problem rather than as a bug.
> `verify` reports it.

## Filled at load, a trickle afterwards

The first tick fills the room to `spawn_max` in one go; every tick after that
adds at most one.

A server that has just started should not have empty rooms for
`max × interval` seconds, and a room that has just been cleared should refill at
a rate the player can outrun. Those are different needs and one rule cannot serve
both.

`spawner.fill_all()` is the counterpart to `mob_d.populate()` and is called
beside it, because a spawner cannot fill as its room registers: `areaload` loads
in passes — items, then rooms, then mobs — so at the moment a room is noticed the
creatures it names do not exist yet.

## The cap counts its own kinds

Not every creature present. Counting everything would let a player switch a nest
off by luring something unrelated into the room, and would stop a patrol route
through a spawner room from ever refilling. A rat that wandered in from next door
*does* count, because it is a rat and the cap is about how many rats the room
should hold.

## Weights are relative to each other

`{5, 3, 1}` and `{50, 30, 10}` are the same table. That is worth stating, because
the alternative — weights as probabilities that must sum to one — is a rule
authors get wrong silently, and the silent version of getting it wrong is a
creature that never appears.

## The fields

| | |
|---|---|
| `spawn_max` | most creatures from this spawner alive here at once. Absent or `0` means the room has no spawner |
| `spawn_interval` | seconds between top-ups. One creature per tick, so this is the *rate the room refills*, not how long it takes to fill |
| `spawn_table` | `{ { template, weight }, … }`. A `record_array` keyed on `template` |

Half a spawner — `spawn_max` with no table, or a table with no `spawn_max` — is
reported by `verify`. Both halves look deliberate on their own and together they
do nothing.

## One heartbeat, not one timer each

`spawner_d` registers a single `ticker_d` entry and each spawner keeps its own
`due` timestamp. A hundred nests would otherwise be a hundred entries in
`ticker_d.list()` and a hundred closures, to do work that is a handful of table
lookups.

The index is fed from `world_d.register_room`, beside the `tag_d.index` call and
for the same reason: a room entering the world is the one moment every path goes
through — an area load, an area reset, a `dig`, a virtual room being realised — so
an index fed there cannot drift. A room that loses its spawner drops out on
re-registration rather than lingering.

## Live, not cached

`spawn_max` and the table are read off the **live room** on every tick, so
`olc set spawn_max 4` takes effect immediately, the way every other OLC edit
does. Caching them when the room was first noticed would have made the spawner
the one field a builder has to reload for, in a tool whose whole claim is that
`set` changes the world as you type.

## Testing one

`spawner.tick(room_id)` is one top-up ignoring the clock, and
`spawner.fill(room_id)` goes straight to `spawn_max` — so a test drives a spawner
without waiting on a heartbeat. `spawner._random` is the seam for the weighted
pick, the way `combat_d._roll` is for the dice.

## See also

- [Prototypes](./prototypes.md) — what the three rats inherit from
- [Creatures & Combat](./combat.md) — `mob_d`, and the fixed-population path
- [OLC](./olc.md) — `olc set spawn_table.<template> <weight>`
