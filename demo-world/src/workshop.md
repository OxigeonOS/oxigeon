# The Wizard's Workshop

*Six rooms. Where you start, and the oldest content in the game.*

```
                    archive
                       |
   pantry ---------- laboratory ---------- scrying chamber
                       |
                   entrance --- east ---> the town undercroft

              treasure vault  (no exits; you leave by touching the orb)
```

This area predates everything else. It was written when the driver had rooms,
object state and room actions and very little else, and it has been left almost
exactly as it was — it is the fixture the real-mudlib tests lean on, and a
fixture that changes is not a fixture.

The one thing that *is* new is the door east.

## The walk

```
> look
Entrance to the Workshop

You stand in a circular foyer choked with decades of dust. A heavy oak door,
banded with iron and etched with faded protective wards, stands ajar to the
east — the wards on it went out a long time ago...

You could try: search
```

```
> search
You rummage through the moth-eaten robes hanging from the coat hooks.
Dust cascades from the fabric. You find nothing but the faint scent of lavender.
```

Nothing. That is the point of it — a room action with no reward, so you learn
that `search` is a thing rooms can offer before it matters.

```
> north
The Alchemical Laboratory
...
You could try: examine <something>, search, pour <color>, collect
```

Four actions in one room. `search` here **does** give you something:

```
> search
You rummage through the cluttered workbench...
You find a small vial of swirling red liquid!
You find a small vial of shimmering blue liquid!
You find a small vial of bubbling green liquid!
```

## The cauldron

Three potions, one cauldron, one order. The order is red, blue, green — and the
laboratory will not tell you, so this is the one puzzle in the game you are
expected to fail at least once.

```
> pour red
> pour blue
> pour green
> collect
```

Get it wrong and the cauldron explodes for 15% of your maximum health, then
resets itself. Get it right and you can collect the result — if you are carrying
the empty vial from the archive.

```
> north
The Forbidden Archive
> take
You take the empty crystal vial.
```

Drink the purple potion and you are teleported to the Treasure Vault, which has
no exits at all. You leave by touching the orb.

## What it proves

| Feature | Where |
|---|---|
| **Room actions** | `search`, `pour`, `collect`, `gaze`, `read`, `take`, `touch` — seven verbs that exist only in specific rooms |
| **Object state** | the cauldron's progress is `set_object_state(room_id, "cauldron_potions", n)`, and it survives a `reload` |
| **lfun descriptions** | the laboratory's `description` is a *function*: the cauldron paragraph changes with the puzzle state, and the room is never told to update |
| **lfun scenery** | `look cauldron` is also a function, and gives a different answer at each step |
| **Teleporting items** | the purple potion's `on_drink` calls `player:move_to()` |
| **A room with no exits** | the vault, which you can only leave the way you came |
| **A spawner** | the pantry's rat nest: three kinds, up to three at once, one more every 45 seconds |
| **A prototype chain** | those three rats are four lines each — `{ id, prototype }` — and inherit everything else |

## The nest

Behind the lowest shelf of the pantry: *a heap of shredded parchment, gnawed cork
and one entire glove. It is warm.*

It is three fields on the room, and nothing else:

```lua
spawn_max      = 3,
spawn_interval = 45,
spawn_table    = {
    { template = "black_rat",    weight = 5 },
    { template = "scrawny_rat",  weight = 3 },
    { template = "muscular_rat", weight = 1 },
},
```

Three things are worth noticing.

**The cap is across kinds.** Three rats of *any* mix is the limit. The older way
of putting creatures in a room — `spawn_room` and `count` on each template —
counts per template, so three rat templates at `count = 2` is six rats and there
is no way to say what the room should hold. See
[Spawners](../book/lua-api/spawners.html).

**The kinds are a prototype chain.** `black_rat`, `scrawny_rat` and
`muscular_rat` are each two lines in `mobs.lua`:

```lua
{ id = "black_rat", prototype = "vermin.rat.black" },
```

Everything else — the name, the health, the damage, the loot table, the body
layout — comes from `vermin.rat` and its three children in
`game/prototypes/beasts.lua`. The scrawny one names four stats and keeps the rest,
because `stats` is a schema `map` and a patch of a map merges key by key.

**It is authored data, so it is editable in the game.** `olc set spawn_max 4` and
`olc set spawn_table.black_rat 8` both work, and `verify` checks that the
templates exist and that none of them *also* respawns on its own — which would
feed the room from two sources and drift it past its cap one kill at a time.

> [!NOTE]
> **The lfun pattern is the thing to take away here.** A room's description can
> be a string or a function returning one, and `Object.resolve` does not care
> which. Nothing pushes an update when the cauldron changes; the room simply
> asks, when somebody looks. That is the same mechanism the marsh uses for
> weather two areas from here, and it scales to "the description depends on
> anything at all" without a single subscription.

## Look closer — the door east

Until recently this area had **no exit to anywhere**. The entrance's only exit
was north into the laboratory, and the vault's exit table was empty.

The workshop is also the **start room**. Every new character arrived in a sealed
pocket of six rooms and could not walk to a single other area in the game.

Nothing caught it. The area's own tests passed, because they test the puzzle.
The town's tests passed, because they reach the town with `goto` — an admin
command. The whole demo world was unreachable by a player, and every test was
green.

What catches it now is `tests/demo_world/world_graph.rs`, which floods the exit graph from
the configured start room and asserts every area is reached:

```rust
for area in ["wizard_workshop", "thornhollow", "greywater_marsh", "collapsed_mine"] {
    assert!(reached.contains(area), "'{area}' cannot be walked to from {start}");
}
```

It also asserts that no exit dangles, that every exit with a known opposite has a
return, and — the one that found the second bug — that **every direction an area
file uses has a command behind it**. `up`, `down`, `in` and `out` were in
`movement.OPPOSITES` and used by rooms from the beginning, and had no verbs. The
stair down from Thornhollow Square had never been walkable.

> [!IMPORTANT]
> A test suite that moves with `goto` never opens a door. Both of these bugs
> were invisible for exactly that reason, and both were found within a minute of
> trying to write down a walkthrough.

## Things to try here

- `gaze` in the scrying chamber, more than once.
- `taste` in the pantry. There are rats in there; most are level 1 and will not
  bother you unless you bother them. The red one will.
- `attack rat`, to see a fight before anything can hurt you.
- Kill all three rats and stand there. One comes back every 45 seconds, up to
  three, and then it stops — see [the nest](#the-nest), below.
- `pour blue` first, on purpose, and read what the cauldron says at each step —
  the room's *description* changes, not just the reply.
- `reload areas.wizard_workshop.rooms` after solving half the puzzle. The
  cauldron keeps its progress: object state is driver-side and survives a Lua
  reload, which is exactly what it is for.
