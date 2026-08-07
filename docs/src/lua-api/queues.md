# Action Queues — Roundtime Without a Straitjacket

```lua
DAEMON.queue.define_track("crafting", {
    round_trait = "craft_round_length",
    empty       = "idle",
    resolve     = function(entity, entry) ... end,
})
```

A **track** is a named lane of intent on one entity: a bounded queue, a roundtime
gate, a policy for what an empty queue does, and a short history of what
finished. Combat is the first one. Crafting and gathering are meant to feel like
the same mini-game, so a track is registered rather than hardcoded — and a second
one needs no edit to the mudlib.

## The rule this exists to keep

> **Roundtime never gates a command.**

`look`, `say`, `who`, `score` and walking all work while you are recovering — and
not because of an exemption list. Nothing in command dispatch reads a track, so
they never enter the code path at all. `lib/commands.lua` is untouched by this
whole feature, and `tests/queues.rs` opens by asserting it.

That is a deliberate rejection of the single global "action" cooldown other
engines use, where being mid-swing stops you looking at the room.

## Recovery, not occupation

Two different ideas, and because they are different they need no arbitration:

| | owns | |
|---|---|---|
| **occupation** | `ability_d`'s `cast_time` / `channel` | "you are in the middle of something" |
| **recovery** | a track's roundtime | "this track may not act again for N seconds" |

The only interaction is that a tick skips an entity that is casting — one line,
reading one existing public function.

## Where roundtime lives

`cooldown_d`, under `rt.<track>`. Not a private store:

- every roundtime is under a minute, so cooldown_d's threshold rule already puts
  it in memory and forgets it on restart — exactly right, and free;
- it already handles creatures as well as players, and `evict_owner` already
  cleans it up on disconnect;
- `cooldown` shows it, so "why can't I swing" has an answer with no new code.

Marked **after** the action resolves. A cast marks it at completion, not at the
start — the cast time was already the occupation. **An interrupted cast marks
none**, the same shape and the same justification as the cooldown it also does
not mark.

## Rounds

```lua
roundtime = { rounds = 0.75 }   -- three quarters of this character's round
roundtime = 2                   -- two seconds flat
roundtime = { rounds = { min = 1, max = 2 } }
```

A round is a **derived trait the game defines**, named per track. Define
`round_length` and agility, encumbrance and the wielded weapon all reach
roundtime through the trait graph and the existing `stat_bonus` →
`equip_trait_<id>` machinery, with no new code anywhere:

```lua
{ id = "round_length", kind = "derived", depends = { "dexterity", "encumbrance" },
  min = 1, max = 6, round = "none",
  formula = function(t) return 4 - t.dexterity * 0.05 + t.encumbrance * 0.1 end }
```

An absent trait falls back to the track's configured round **and warns once**.
A silent zero would be a wrong answer, and this project does not ship those.

> [!NOTE]
> Rounds **ceil to whole seconds**, and that is the clock's decision rather than
> a taste one. `os_time()` is integer, so an expiry of `now + 2.25` is observed
> at one-second granularity — the gate would open somewhere in [2s, 3s]
> depending on where inside the second it was marked. A stated 3 beats an
> unstated 2-or-3.

`{ rounds = n }` is a real branch in `Abilities.roll`, **not** a desugar into
`scale`: scaling is additive and a round is multiplicative, so no fixed authored
spec expresses it.

## An ability in roundtime is queued, not refused

**Only roundtime enqueues.** A cooldown, the GCD, a cost, a requirement, an
unknown ability and a mistyped target all still refuse. Short rule, and it keeps
two different pieces of information distinct:

```
cooldown    "Not yet. (12s)"              not this, for a while
roundtime   "You will cleave next. (2s)"  not yet, but soon and certainly
```

The enqueue sits **below target resolution**, so a typo still refuses
immediately rather than queueing something doomed — the invariant inherited from
`spell_d`, asserted again under roundtime.

`use` returns a third value: `"done"`, `"casting"` or `"queued"`. Every existing
caller destructures two.

## The queue

- **Bounded** (default 3). When full the **newest is refused**, not the oldest
  dropped — silently discarding something already committed to is the worse
  failure, and a refusal is information.
- Entries hold the **resolved target entity**, never a name. Re-resolving at
  dequeue would let somebody retarget you by walking a matching creature into
  the room.
- An entry older than 30s is **dropped at dequeue**. A queue stuffed during a lag
  spike replaying a minute later is the commonest way a queue feels broken.
- History keeps ids and numbers only, **no entity references**, so a corpse is
  not retained by the fight that killed it. It is the hook a combo system would
  read; there is no combo system yet.

| event | queue | roundtime |
|---|---|---|
| death | cleared | cleared |
| disconnect / despawn | cleared | already gone |
| flee | cleared | **kept** — fleeing is not a free reset |
| target lost | that entry dropped at dequeue | untouched |

## The empty-queue policy

| | |
|---|---|
| `auto` | keep acting — the combat track's default, and what combat did before a queue existed |
| `idle` | stand there until told otherwise |
| `repeat` | do the last thing again |

An engaged fighter with an empty queue swings every round. That is today's game
exactly; `queue idle` opts out.

## Commands

```
queue                     the combat track
queue <track>
queue clear [<track>]
queue next <ability> [at <target>]
queue auto|idle|repeat
```

## Configuration

```toml
[game]
queue_tick_seconds  = 1    # how promptly the queue notices you are free
queue_max           = 3
queue_history       = 5
queue_stale_seconds = 30
combat_round_seconds = 3   # the fallback round length, not a scheduler interval
```

`combat_round_seconds` kept its name and changed its meaning: it is now the round
length a game falls back to when it defines no `round_length` trait. 0 still
means combat is off.

## What it will not do

- **Gate a command.** Ever.
- **Decide that two tracks conflict.** That is a game's call, and
  `in_combat` / `out_of_combat` requirement predicates already ship.
- **Combos.** The history ring is there; the predicate is not.

## See also

- [Abilities](./abilities.md) — `track` and `roundtime` on a spec
- [Creatures & Combat](./combat.md)
- [State Cache](./state-cache.md) — why a queue is memory-tier
