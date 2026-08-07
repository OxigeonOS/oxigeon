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
whole feature, and `tests/mudlib/queues.rs` opens by asserting it.

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

## How fast anything acts: two layers

```
swing time = round_length / speed
```

Keeping these apart is the whole model, and each answers a different question.

| | | |
|---|---|---|
| `round_length` | a **derived trait** on the entity | how fast this *person* operates |
| `speed` | a field on the **weapon**, or on the creature's template | how many swings fit in one round |

`round_length` gates every action on the track — swinging, casting, drinking,
fleeing — so **armour belongs there**. `speed` only affects swings.

That is why armour counts for more than a weapon without needing a bigger
coefficient: it taxes everything you do, and a greatsword taxes one thing.

### `speed` is a rate, not a duration

`dagger 1.2`, `sword 1.0`, `greatsword 0.7` — higher is faster. So the time one
swing costs is `round_length / speed`, and the dead `weapon.dps` helper
(`avg_damage * speed`) is what proves that was always the intent: damage per
swing times swings per unit time is damage per unit time.

Every authored number already meant the right thing, so none of them changed
when this began to be read.

A creature with nothing in its hands uses its **template's** `speed`, which is
what makes a rat fast and weak rather than merely weak. Without it a creature's
rate came from `round_length` alone, which moves 0.05s per point of dexterity —
so a rat and a bear were within a tenth of a second of each other and no amount
of authoring could separate them.

### Armour is paid for out of strength

`encumbrance` is an ordinary attribute fed by `stat_bonus` on worn pieces, and
`round_length` charges only for what exceeds `strength * 1.5`:

| build | dex | str | encumbrance | round |
|---|---|---|---|---|
| rogue, leather | 16 | 10 | 8 | 2.70s |
| knight, plate | 12 | 18 | 35 | 3.54s |
| average, plate | 10 | 10 | 35 | 4.60s |
| wizard, plate | 10 | 8 | 35 | 4.84s |

A flat penalty per armour class would charge the wizard and the knight the same,
which is the opposite of true — and it is why this is a formula on a derived
trait rather than a number on an item. It is game content: the mudlib ships no
`round_length` and falls back to `game.combat_round_seconds`, saying so once per
track.

A **weapon** can carry a `stat_bonus` too, so a heavy blade that lengthens your
round says so the same way armour does. `speed` is how often it swings; the
bonus is how long a round is.

> [!WARNING]
> **`game.queue_tick_seconds` quantises all of it.** At the 1 second it shipped
> with, a player at 3.0s and a rat at 2.9s came free on the same tick and traded
> blows in perfect lockstep — no bug anywhere, a clock too coarse to tell them
> apart. It is 0.25 now. Anything you want distinguishable has to differ by more
> than one tick.

> [!NOTE]
> **Keep the `round_length` floor at or above the global cooldown.** Below it,
> every point of speed buys nothing, and a player who finds that will read it as
> a bug rather than as a cap.

## Waiting enqueues; being unable refuses

Roundtime and a cooldown are both **waiting**, so both queue. A cost you cannot
pay, a requirement you do not meet, an unknown ability and a mistyped target are
all **unable**, and all refuse — queueing those would promise something that
cannot happen.

```
roundtime   "You will cleave next. (2s)"   waiting  -> queued
cooldown    "You will cleave next. (12s)"  waiting  -> queued
no mana     "You do not have the mana."    unable   -> refused
bad target  "There is no nosuchthing here." unable  -> refused
```

The number is whichever gate clears last, because that is when it will actually
happen.

> [!NOTE]
> **This was cooldowns-refuse**, on the argument that a cooldown says "not this,
> for a while" where roundtime says "not yet, but soon and certainly". That is a
> true distinction and it is not one the player is in a position to make. From
> the seat, `Not yet. (1s)` and `You will emberlance next` are the same
> situation, and getting two behaviours out of one intent reads as the game being
> arbitrary rather than as a considered rule.

Making it safe needed one thing in `queue_d`: a resolver may return `"retry"` to
be **put back at the head** rather than dropped. `advance` pops the entry before
resolving, so without it a queued ability whose cooldown had not yet cleared was
popped, refused and silently lost — the player queues something and nothing ever
happens, which is worse than either behaviour it replaced. The staleness bound
already above it is what stops an entry that can never run blocking the head for
ever.

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
