# The Undercroft

*Under the chapel. Dry, which nobody can explain.*

`down` from the square, then `down` again.

```
> look
The Undercroft

A vaulted room the footprint of the chapel above it, kept dry by something
nobody in the town can explain and nobody wants explained. The town's strongbox
sits against the north wall on a plinth...

Obvious exits: east, up, west
Lying here:
  the town strongbox
```

## The vault — a container that stays put

```
> examine strongbox
the town strongbox
  An iron-banded chest the size of a coffin for a short person. It is bolted to
  the plinth, which answers the first question anyone asks about it.
  Weight: 400
  It is empty.
  Capacity: 0 / 40
```

Four hundred units of weight. You are not carrying that anywhere, and the
description says so rather than the game refusing with a shrug.

```
> put rope in strongbox
You put a coil of hemp rope in the town strongbox.

> up
> down
> examine strongbox
  It contains:
    a coil of hemp rope
```

It is still there. A room's contents are not your inventory, and the vault is the
third kind of container this world has:

| | | |
|---|---|---|
| **carried** | the backpack | goes where you go; contents saved with you |
| **fixed** | the vault | stays in the room; contents are memory-tier |
| **temporary** | the Delver's corpse | appears on death, rots on a timer |

All three are the same component with different numbers. What is *in* one is
never part of the component — contents live in the item daemon's location index,
keyed `item:<uuid>`, so `put` and `get from` are the same code path as `drop` and
`get`.

## The crypt

`east`, and it is dark.

```
> east
It is pitch dark. You can feel a floor under you and nothing else.
You can feel your way west.
```

You can still *walk*, and the exits are still listed. Being in the dark is a
situation rather than a wall — a movement system that refused would make a
lantern a key rather than a light.

With a lantern lit:

```
> use lantern
> look
The Old Crypt

The older part, and it does not match: the vaulting here is round rather than
pointed, and the floor is flagstones rather than beaten chalk...
You could try: pry the flagstone
```

```
> pry
You get your fingers under the edge and heave. It does not move, and your
fingers say enough about that.
```

Thirteen strength. This is the one place in town that cares how strong you are,
and it is gated on a **trait** rather than an item — read through
`player:trait("strength")`, so a buff counts.

Get it up and there is a brass key underneath, which is one of the two ways into
the mine's locked grille.

```
> pry
The flagstone comes up with a sound like a held breath let go.
Underneath, in a hollow the size of a hat, is a small brass key.

> pry
The flagstone is already up. The hole under it is empty.
```

The second `pry` remembers. That is object state on the room — and unlike the
well's cooldown, this one **should** be cleared by an area reset, because it is
world state rather than per-character state. Both kinds are in this town, twenty
feet apart, doing the right thing for opposite reasons.

## Look closer — the door west

```
> west
Entrance to the Workshop
```

This is the door that was missing. It is described from both sides — an oak door
in the undercroft's west wall, ajar; a heavy oak door east of the workshop
foyer, wards long since gone out — and `west` and `east` agree about which way
it goes.

That last part sounds too obvious to mention. It was not: when the door was
first added it was `west` from the undercroft and `south` from the workshop, so
walking through it and turning round did not bring you back. The audit test
caught it before anyone walked it:

```
AUDIT ONEWAY: thornhollow.undercroft west -> wizard_workshop.entrance (no east back)
```

`tests/world_graph.rs` asserts every exit with a known opposite has a matching
return. One-way exits are legitimate — a trapdoor, a teleport — but they should
be *chosen*, and an accidental one makes an area reachable and not leavable.

## Things to try here

- Put something in the vault, `quit`, log back in, and check it is still there.
- Take the lantern *out* and try `examine strongbox` in the dark. Looking closely
  at something needs light too — the check is before every branch of `look`, not
  just the room description.
- `stat thornhollow.undercroft` as an admin, to see the object state a room is
  carrying.
