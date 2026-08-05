# Greywater Marsh

*Five rooms of grey water. Where the weather is, and where things bite.*

`west` from the square to the gate, `west` again onto the stone.

```
causeway head --- causeway mid --- stilt village --- deep water
   (to town)           |
                   herb beds
```

## The walk

```
> west
The West Gate

Two posts and a rail that has not been a gate for some years. Past it the road
stops pretending and becomes a causeway of laid stone running west into
Greywater...

> talk guard
"Stay on the stone."

> ask guard about stone
"The causeway. It's laid on piles down to the hard bottom. Step off it and
you're in six feet of water and eleven feet of everything else."
```

He is not being decorative.

```
> west
The Head of the Causeway
...
You could try: wade off the causeway

> wade
You put a foot off the causeway. The reed mat holds for a moment and then does
not, and you are in to the chest in water the temperature of a cellar.
```

That costs you eight health and gives you marsh fever.

## The weather

Read a marsh room twice, a few minutes apart:

```
The laid stone begins here and runs west in a line so straight it is obviously
older than the town...

The sky is a washed-out white, and for once it is dry.
```

```
...

Fog stands on the water in walls. Ten feet, and then nothing.
```

And in fog, the room says something it does not otherwise say:

```
The next cairn is a suggestion. The one after it is not there.
```

Six states — clear, overcast, drizzle, rain, storm, fog — walking to a
*neighbour* every five minutes rather than jumping, because a system that can go
from clear to storm reads as broken rather than dramatic.

### Two things about how that works

**Nothing is pushed.** A room's description is an lfun; it asks the weather
daemon what the sky is doing at the moment somebody looks. There is no
subscription list, no per-room state, nothing to keep in step. It is the same
mechanism the workshop's cauldron uses, pointed at a different question.

**The tick uses an index, not a walk.** The weather has to reach outdoor rooms
and only outdoor rooms. Walking every room in the world and testing each one is
O(the whole world) every five minutes, forever, to find the seven that are
outside:

```lua
for _, room_id in ipairs(DAEMON.tag.find("room", "outdoor")) do
```

That is a lookup. `tag_d` exists for exactly this and this is its first
consumer — rooms carry `tags`, the index is fed as they are registered, and it
answers the *backward* question that a per-room list cannot.

### Weather changes the light

```
> stat greywater_marsh.causeway_head
  Light        3
```

but in fog:

```lua
Room:effective_light()   --> 1
```

`light_level` is what the room *is*; `effective_light` is what it is *like*, and
a game daemon gets to have an opinion about the difference. Fog takes two off an
outdoor room and does nothing at all under the chapel.

## Aggression

The marsh is where `Mobile.aggressive` finally means something.

```
> west
The Causeway
> west
The Stilt Village

a marsh lurker turns to look at you.
```

Three seconds later it attacks. The delay is long enough to read the room and
turn round, which is the difference between a threat and an ambush.

`aggressive` had been on every mob template since the class was written and
**nothing read it**. What reads it now is `game/daemons/aggro_d.lua`, and it is
in the game layer on purpose:

> [!NOTE]
> The driver ships the flag and the `room.entered` event and takes no position
> on what should happen. Whether an aggressive creature attacks, how long it
> waits, whether it cares about level or faction, and whether it gives up when
> you flee are all *game* decisions. A different game wants a different file,
> not a configuration option.

This game's policy: attack after three seconds, ignore anyone more than eight
levels above you (a rat suiciding into a passing archmage is comedy once and
tedium twice), never attack your own faction, and assist a faction-mate who is
already fighting.

## The Wisp, and damage types

`west` again, to the deep water, is a level 10 unique that deals **magic**.

```
> attack wisp
```

That is the one fight in the game where what you are wearing matters in a
specific way rather than a general one.

| Against | leather jerkin (def 3) | warded cloak (def 1, magic resist 6) |
|---|---:|---:|
| a sword | −3 | −1 |
| the Wisp | −3 | **−7** |

`armour.resist` is looked up by the damage type on the event. A cloak that
blunted everything would just be armour; one that blunts *magic* is a decision
about what to carry into which area, and it is four lines of data:

```lua
Armor{ id = "warded_cloak", slot = "back", defense = 1,
       resist = { magic = 6 } }
```

The Wisp drops the cloak a quarter of the time and a silver dagger — `damage_type
= "magic"` — one time in seven. Which means the answer to the Wisp is either
armour taken off the Wisp or a weapon taken off the Wisp.

It also marks you:

```
The light turns towards you, and stays turned.
```

`wisp_mark` is `survives_death`. Dying clears your effects — except the ones
that say otherwise, because a curse you can remove by walking into a rat is not
a curse.

## The herb beds — the daily gate

`north` from the middle of the causeway.

```
> gather
You go in to the elbow and come out with a fistful of pale forked root,
warm to the touch and smelling of the bottom.

> gather
The bed is picked over. Give it a day. (24h)
```

This is the single most important line in the demo world, and it looks like
nothing.

> [!IMPORTANT]
> **The bug this exists to prove fixed.** A "once per 24 hours" gate stored as
> room object state is really "once per fifteen minutes", because that is how
> often an area resets and a reset wipes object state.
>
> Per-character state does not belong on a room. This is `DAEMON.cooldown`,
> keyed by character, at 24 hours — which is over the durable threshold, so it
> is written through to the database and survives a reset *and* a restart.
>
> Test it: `areas reset greywater_marsh` and try `gather` again. It still
> refuses. Then go and do the same thing to the mine's ore seam, twenty minutes
> away, and watch that one come back — because *that* one is shared world state
> and refilling it is correct.

Two gates, opposite answers, both right. The rule is not "always use cooldowns";
it is *choose the tier by how much you would mind losing it*.

## Marsh fever

```
> effects
Currently affecting you:
  Marsh Fever          2m45s      Something from the water is in you.
```

Damage over time, and there is **no timer behind it**. Every ticking effect in
the game is driven by one shared heartbeat; each instance earns whole ticks from
its own `last_tick` and advances by exactly the ticks it fired, so a coarse
heartbeat loses no accuracy and one that skipped a beat catches up.

Two global timers for the entire effect system, whatever is running on whom.

It is also not a modifier. Poison deals *damage* — through `take_damage`, so
armour and resists get their say — rather than editing your health, because
effects modify attributes and derived traits and never gauges. A poison that
edited your current health would need to be unapplied symmetrically, and that is
the whole class of bug the design avoids.

The apothecary sells the antidote:

```
> drink antidote
You swallow the antidote. It tastes of the marsh, which is somehow worse than
the poison.
```

It removes marsh fever **by name**. A blanket clear would strip your blessings
too, which would be a trap wearing a helpful label.

## What the marsh proves

| Feature | Where |
|---|---|
| lfun descriptions reading a daemon | every room, on every look |
| lfun `sound`, not just `description` | the causeway head, in fog |
| `tag_d` reverse index | the weather tick, over `outdoor` |
| `Room:effective_light` | fog making an outdoor room dim |
| `Mobile.aggressive` | the lurkers, the crawlers, the Wisp |
| `unique` | one Wisp, however often `populate` runs |
| `damage_type` vs `armour.resist` | the Wisp against the warded cloak |
| `on_combat` | the lurker's bite, which poisons |
| Tick effects on the shared heartbeat | marsh fever |
| `survives_death` | the Wisp's mark |
| A **durable** cooldown | the herb beds |
| Named effect removal | the antidote |

## Things to try here

- Stand in one room for twenty minutes and watch the weather move. It announces
  itself only to people who are outdoors.
- `wade` deliberately, then walk back to town and buy an antidote. It is the
  cheapest way to see the whole poison → cure loop.
- Kill five reed crawlers for the guard's quest and watch the counter — it comes
  from the `mob.died` event, so combat never learns that quests exist.
- Fight a lurker wearing nothing, then wearing the jerkin. Same creature,
  visibly different numbers.
