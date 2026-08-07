# The Collapsed Mine

*Six rooms, mostly dark, going down. The one place you need equipment.*

`down` from the smithy.

```
adit --- first level --- second level --- pump house
                              |
                        deep workings --- the sump
```

## Bring a lantern

```
> down
The Mine Adit
> down
It is pitch dark. You can feel a floor under you and nothing else.
You can feel your way down, up.
```

`Room.light_level` had been a field since rooms existed and **nothing read it**.
Every room in the game was equally visible and the field documented an
intention.

```
> use lantern
You open the hood. Warm yellow light fills the space around you.
> look
The First Level

A gallery following the seam, propped every eight feet with timber that has gone
grey. The floor is loose shale and it moves...
You could try: mine the seam
```

The check is in `lib/light.lua` and it is deliberately small: one question, one
answer, one place to change it. Carried counts as well as equipped — insisting a
lantern be in the `light` slot before it lights anything is a rule players
discover by dying in the dark.

## The seam

```
> mine
You get the pick in behind a plate of shale and lever. A hand's worth of ore
comes away with it.

> mine
The seam here is worked out. Give the mine time.
```

**This is the counterpart to the marsh's herb bed, and it is deliberately the
opposite decision.** Everybody shares one seam, so "worked out" is *room* state —
and an area reset refilling it is correct. The herb bed is per character and
must survive a reset. Same shape, opposite tier, twenty minutes apart, so you can
compare them.

The ore is what the smith's daily quest wants.

## The grille — two ways through one door

```
> down
The Second Level
...
The grille is down and the lock is engaged.

> open
The grille is locked. There is a keyhole, and it is the size of a thumb.
```

Two routes. The brass key from under the crypt flagstone:

```
> open
The brass key turns, badly, and the lock gives.
```

or a lockpick from Hobb's *and* thirteen dexterity:

```
> open
You get the pick in and feel the wards, and then the pick comes out again
without them.
```

> [!NOTE]
> A door with exactly one key is a door that is really a switch. Two routes make
> it a decision — go and find the key, or come back when you are better at this
> — and the second route is gated on a **trait**, so a dexterity buff opens it.

The state is on the room, so it stays open. It is also cleared by an area reset,
which is right for a door and wrong for a daily gate: the same distinction as
the seam above.

The exit itself carries the check:

```lua
west = {
    target = "collapsed_mine.deep_workings",
    check = function(player)
        if get_object_state("collapsed_mine.second_level", "door_open") then
            return true
        end
        return false, "The grille is down."
    end,
},
```

A *rich exit* — a table rather than a room id, with a predicate and a refusal
message. It is also why route-planning has to be revalidated: a graph can carry
where an exit goes and cannot carry whether it will open. See
[The Drowned Reach](./reach.md).

## The pump house

`east` from the second level.

```
> look
The Pump House

A chamber cut square around a beam engine three times the height of a person.
Three levers stand in a rack by the wall, each as long as an arm, and each with
a plate above it that the damp has taken.

All three levers stand upright.
You could try: pull <left|middle|right>
```

The order is left, middle, right. The hint is in the room, and it is a hint
rather than an instruction:

```
> look plates
Brass plates, green and unreadable. One of them has been scratched with a tally:
one line, two lines, three lines, left to right.
```

```
> pull left
The left lever goes over and stays over. Something under the floor takes up the
slack.

> pull right
The lever goes over and something under the floor lets go with a sound you feel
rather than hear. The other levers spring back.
```

Wrong lever, back to the start. Get it right and:

```
> pull left
> pull middle
> pull right
The third lever goes over and the pump takes. Water starts moving in the pipes
overhead, and a long way down something that was under water stops being under
water.
```

The room description changes at every step — upright, "1 of the levers are over
and holding", and finally "the engine is working" — and so does the `sound`
property. Both are lfuns reading the same object state.

There is a sixty-second timer: start the sequence and dawdle, and it resets
itself. The timer is armed **by id**, so pulling the first lever twice re-arms
one timer rather than stacking two.

> [!NOTE]
> That detail has history. `ticker_d.remove_by_prefix` once had a bug where
> every per-player timer leaked, and a leaked closure pins its upvalues —
> including the Player object. Arming by id is the shape that does not leak, and
> `tests/demo_world/mine.rs` asserts that two `after` calls with one id leave one timer.

## The Delver

With the pump running, the deep workings drain and a shaft opens.

```
> west
The Deep Workings
...
The water has gone down. There is a shaft in the floor that was not visible
before, going further down.

> down
The Sump
```

Level 15, 260 health, and it hits for 12–22. You will not win this at level 5.

It lays a curse — `delvers_regard`, which `survives_death` and makes everything
find you 15% more easily — and when it dies it leaves a **corpse**:

```
> look
Lying here:
  the Delver's corpse

> examine corpse
the Delver's corpse
  It contains:
    a Delver's claw
    a lump of iron ore
    a lump of iron ore
    a small brass key
```

The third kind of container. Not carried, not fixed, and it goes away — a ticker
rots it after ten minutes, armed by the corpse's own instance id so a second
boss's corpse does not replace the first one's timer.

> [!NOTE]
> A rat drops one thing on the floor; a boss drops six. Six items on a floor is
> a wall of text where a corpse is one line and a decision. `capacity = 0` means
> unlimited, because a boss that dropped eleven things should not silently lose
> the eleventh.

Its death also announces itself to the area:

```lua
DAEMON.event.emit("area.collapsed_mine.delver_slain", { ... })
```

which is `signals.md`'s worked example — a game reacting to a boss dying without
combat knowing it can be reacted to.

## Look closer — the template's own hook

The Delver's corpse did not appear at first. `mob_d.spawn` **replaced** the
template's `on_death` with its own event emitter, so a template could declare
the hook and never see it called.

It wraps now:

```lua
local template_on_death = mob.on_death
mob.on_death = function(self)
    if type(template_on_death) == "function" then
        pcall(template_on_death, self)
    end
    DAEMON.event.emit("mob.died", { ... })
end
```

The template's hook runs *first*, while the world still looks the way it did
when the creature died.

## What the mine proves

| Feature | Where |
|---|---|
| `Room.light_level` | four rooms at 0 |
| Per-instance light state | two lanterns disagreeing |
| Object state as world state | the seam, the door, the levers |
| Rich exits with a `check` | the grille, the shaft |
| A multi-step puzzle with a timed reset | the levers |
| Ticker arming by id | the reset timer |
| `unique` bosses | the Delver |
| A template's own `on_death` | the corpse |
| A **temporary** container | the corpse, which rots |
| `survives_death` | the Delver's Regard |
| Area-scoped events | `area.collapsed_mine.delver_slain` |
| Area reset clearing world state | everything above, and *not* the herb bed |

## Things to try here

- Solve the levers, then `areas reset collapsed_mine`, then look at them again.
- Take the lantern off in the pump house mid-puzzle. The levers still work —
  you can feel for them — but you cannot read the room.
- `mine` the seam, reset the area, `mine` again. Then do the same for the marsh
  herb beds and notice the difference.
- Kill the Delver with `affect learn level 15` if you would rather see the
  corpse than earn it. That is an admin command and the guide is not pretending
  otherwise.
