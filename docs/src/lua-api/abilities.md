# Abilities — Gameplay as Data

```lua
-- game/abilities/spells.lua. No Lua anywhere in this record.
{
    id       = "emberlance",
    name     = "Emberlance",
    category = "spell",
    summary  = "A line of fire, at one thing.",
    open     = true,
    level    = 1,
    cost     = { mp = 8 },
    cooldown = 4,
    target   = "creature",
    damage   = { min = 8, max = 8, type = "fire",
                 scale = { trait = "spell_power", per = 2 } },
    messages = {
        self   = "{red}You draw a line of fire at $target.{/}",
        room   = "$name draws a line of fire.",
        result = "It takes $dealt.",
    },
},
```

That used to be a function.

## Why this exists

`spell_d` was 177 lines arranging five things the mudlib already had — gauges,
effects, the damage pipeline, cooldowns and the trait graph — into "casting".
Every game that wanted a different arrangement rewrote those 177 lines, and every
spell inside it was a hand-written Lua function.

The arrangement is not what makes one game's magic different from another's. The
*content* is. So the arrangement moved into the mudlib and the content became a
data bag.

The shape is Unreal's Gameplay Ability System, pared down to what a MUD actually
has: no prediction, no replication, no attribute sets separate from the trait
graph, no cue system separate from effects. What survives is the part that
matters — **a designer describes an ability instead of programming one.**

> [!NOTE]
> `ability_d` adds no new state machinery. Costs are gauges spent through
> `trait_d`. Outcomes go through `Mobile:take_damage` and `Mobile:heal`, so
> armour and the effect pipeline meet an ability exactly as they meet a sword.
> Gates are `cooldown_d`. **A channel is an `effect_d` instance.** A cast in
> flight is one `ticker_d` timer and one memory-tier cache key.

## Two halves, and only one of them is code

The same split `effect_d` already draws:

| | lives in | may hold functions | written |
|---|---|---|---|
| **definitions** — the spec | `game/abilities/*.lua`, code | yes | never |
| **grants, casts** | `DAEMON.cache` | no (except the memory tier) | by tier |

A coder builds a library of effects and the occasional `run`. Everything else is
a designer writing records. Of the four shipped spells, two have no Lua at all,
one has a single arithmetic helper, and one is still a program — that spread is
the point.

## The spec

Everything is optional but `id`.

### Identity
| field | default | |
|---|---|---|
| `id` | — | required, unique |
| `name` | `id` | display name |
| `category` | `"ability"` | **a freeform lens that never changes behaviour** — exactly `trait.category`'s rule. It decides which command lists it and prefixes the cooldown key. The moment a category is tempted to *mean* something, that becomes its own declared field |
| `summary` | `""` | one line, for the listing |

### Ownership
| field | |
|---|---|
| `rank_trait` | the trait whose value **is** the rank. Having the trait is knowing the ability — presence decided by storage, never declared. Usually a `kind = "counter", category = "skill", sets = false` trait, the shape `game/traits/skills.lua` already ships |
| `min_rank` | rank at which it becomes usable. Default 1 |
| `open` | known to anyone who passes `requires`. A declared field rather than an implicit rule, because "everyone can cast what they have the level for" is a real classless design and should be one word |
| `grantable` | may be handed out by a source. Default true |

An ability with none of these and no grant is **not known**. There is no implicit
"everybody knows everything".

### Gating

```lua
requires = {
    { kind = "trait", id = "level", min = 3 },
    { kind = "effect_absent", id = "silenced", why = "You cannot speak." },
    { kind = "equipped", slot = "weapon", tag = "sword" },
}
```

A list of maps with a `kind`, never `{ "trait", id = ... }` — a mixed list/map
table is the shape `lua_to_json` refuses and `schema.set` cannot round-trip, and
a spec form that is illegal in half the codebase is a trap even where it happens
to be legal. `level = 3` is sugar for the first row.

`why` overrides the predicate's own refusal, so a designer can say what they mean
without writing a predicate.

Shipped kinds: `trait`, `rank`, `resource`, `effect`, `effect_absent`,
`equipped`, `target_alive`, `target_dead`, `target_hostile`, `target_not_self`,
`in_combat`, `out_of_combat`, and **`check`** — which wraps any existing
`lib/checks.lua` predicate. That bridge is what stops this becoming a second
predicate library.

The game layer adds its own:

```lua
DAEMON.ability.define_check("faction_standing", function(user, target, spec)
    if standing(user, spec.faction) >= (spec.min or 0) then return true end
    return false, "They will not work with you."
end)
```

### Cost

```lua
cost = { mp = 8, stamina = 4 }              -- shorthand
cost = { { trait = "mp", amount = 8 } }     -- long form, when it scales
```

**Gauges only**, refused loudly at define time — the mirror of `effect_d`
refusing a modifier aimed at a gauge, inverted. A cost against an attribute is a
modifier pretending to be a payment, which is the mistake the whole effect design
exists to avoid. Spent with `trait.adjust`, so regeneration settles first and the
cost comes off the value as it is *now*.

### Cooldown

```lua
cooldown = 4
cooldown = { seconds = 90, durable = true, shared = "school.fire" }
gcd = false
```

The key is `<category>.<id>`. `shared` is used verbatim, so two abilities naming
`school.fire` gate each other with no new mechanism. The tier is `cooldown_d`'s
existing duration rule: under a minute it is a game mechanic, over a minute it is
a promise to the player.

### Targeting
| field | |
|---|---|
| `target` | `none` \| `self` \| `creature` \| `ally` \| `any` \| `item` — resolved by the daemon, so every ability refuses the same way rather than each inventing a message |
| `default_target` | `"combat"` falls back to what you are already fighting |
| `allow_dead` | |

### Timing

```lua
cast_time = 2
channel   = { duration = 6, tick = 2 }
interrupt = { on_damage = true, on_move = true, threshold = 5 }
```

### Outcomes

```lua
damage   = { min = 4, max = 9, type = "fire", scale = { trait = "rank", per = 1.5 } }
heal     = { min = 13, max = 13, to = "self" }
apply    = { { effect = "weakened", to = "target", duration = 12, chance = 0.4 } }
remove   = { { effect = "poison", to = "self", count = 1 } }
engage   = true
messages = { begin, self, room, target, result, fail }
```

**Fixed resolution order**, so two abilities with the same fields always behave
the same way: roll → announce → `remove` → `damage` → `heal` → `apply` →
`result` → `run` → `engage`.

Messages substitute `$name $target $amount $dealt $healed $rank $why`. An unknown
token is **left alone** rather than erased: "You strike $victim" is a typo
somebody can see and fix; "You strike " is a bug they will stare at.

### Escape hatches

`run(ctx)`, `on_begin`, `on_tick`, `on_interrupt`, `on_complete`. `ctx` carries
`{ user, target, ability, rank, power, dealt, healed, spent, reason }`.

## Numbers: three shapes, no formula strings

```lua
6                                                     -- a number
{ min = 4, max = 9 }                                  -- a range
{ min = 8, max = 8, scale = { trait = "spell_power", per = 2 } }
{ min = 10, max = 10, scale = { { trait = "rank", per = 2 },
                                { trait = "level", pct = 5 } } }
function(ctx) return 2 + math.floor(ctx.power / 3) end
```

`per` is flat per point; `pct` is percent of the rolled base per point.
`trait = "rank"` is a pseudo-trait resolving to the ability's own rank, so a
designer scales with rank without knowing which trait backs it — and it still
works for an ability that was granted and has no trait at all.

> [!WARNING]
> **Not a formula string.** Parsing `"6 + spell_power * 2"` needs either a
> hand-rolled expression parser — a second string-to-value converter, which
> `CLAUDE.md` forbids by name — or `load()` on author text, which is a sandbox
> hole. The precedent is already functions: `trait.formula`, `effect.condition`,
> the room lfun pattern. And `base + stat * n` is what a damage formula almost
> always is, so `{ trait, per }` covers the half that has to survive being
> written by somebody who does not write Lua.

## The order things happen in

```
 1 known                       6 cooldowns: own, shared, global
 2 rank >= min_rank            7 resolve the target
 3 not already busy            8 target requirements
 4 requirements                9 can you afford it
 5 ────────────── nothing has been spent to this point ──────────────
10 cast time or channel: begin, and return
11 commit: spend, mark
12 outcomes
```

Step 7 after step 6 and before step 9 is `spell_d`'s discipline kept verbatim:
**the target is resolved before anything is spent, so a mistyped name does not
cost mana.**

### A cast that spans time

> **Cost at the start. The ability's own cooldown at completion. The global
> cooldown at the start.**

- **Cost at the start**, because that is what makes an interrupt *cost*
  something. A cast you can begin for free and abort for free is not a risk — it
  is a free oracle: start it, see whether the creature turns, cancel.
- **Cooldown at completion**, because a cooldown rate-limits the *outcome*, not
  the attempt. Marking it up front punishes an interrupted cast twice.
- **GCD at the start**, because it is the one gate that exists to rate-limit
  *inputs*. That is why it is a separate field.

**Nothing is refunded on an interrupt.** A game that wants a partial refund
writes three lines of `trait.adjust` in `on_interrupt` — the mudlib takes no
position on what is policy.

**Every requirement is re-checked at completion.** Three seconds is enough for
the target to die, to walk out, or for the caster to be silenced. A failing
re-check is an interrupt: the cost is already spent and the cooldown is not
marked.

## A channel is an effect

`ability_d` generates one effect definition per channelling ability,
`channel_<id>`, lazily and idempotently — the same trick `lib/equipment.lua` uses
for `equip_trait_<id>`, and for the same reason: an effect's hooks are fixed at
define time.

Everything a channel needs, `effect_d` already has:

| a channel needs | `effect_d` |
|---|---|
| a timed thing attached to an entity | the instance model |
| something every N seconds | `tick` and the shared heartbeat, carrying the remainder |
| to end and know *why* | `on_expire(reason)` — `"timeout"` is completion, anything else is an interrupt |
| to end for a player typing nothing | the sweep, which exists for precisely this |
| to end on death and on logout | `death_d`'s clear, `character_d.unload`'s detach |
| to be visible | the `effects` command |

The cost: tick granularity is the shared `effect_heartbeat_seconds`, so
`channel.tick` rounds up to a multiple of it.

**A cast time is not an effect.** It is one deadline with no intermediate
behaviour, and `ticker_d` does that in one line.

## Interrupts

Two edits outside this daemon, both small and both direct calls:

- `Mobile:take_damage` calls `ability.on_damaged`. Not an event, because
  `event.emit` on every hit of every fight is a table and a dispatch on the
  hottest loop in the game. Not an effect hook either — a `post`-phase handler
  only runs for an entity that *has* an effect on it, and a caster part-way
  through a plain `cast_time` has none, so it would silently not fire for
  exactly the case it is for.
- `movement.move` calls `ability.on_moved`, and **never refuses the move**: a
  channel that pinned you in place until it finished would be a trap, not a cost.

`DAEMON.ability.cancel(entity, reason)` is the public door for everything else.

## Two ways to have an ability

```lua
DAEMON.ability.grant(entity, id, opts)
DAEMON.ability.revoke(entity, id_or_source, opts)
DAEMON.ability.set_source_abilities(entity, source, specs) -> added, removed
```

`set_source_abilities` mirrors `effect_d.set_source_effects` field for field and
contract for contract: *the abilities from this source are now exactly these*.
Idempotent, so it is safe on every login and every slot change without working
out what it did last time.

| namespace | tier | for |
|---|---|---|
| `abilities` | write_behind, `owner = "char"`, preloaded | a bare grant — a quest reward, a tome. Nothing rebuilds it, so it has to be written |
| `abilities_fast` | memory, `owner = "none"` | anything reconciled from a source that is itself saved: equipment, a form, later a class. What is worn is saved; the grant is derived from it, and the derived copy is the only one that can be wrong |

**Rank folds by `math.max`.** A sword granting Cleave at rank 2 must not *reduce*
a swordmaster already at 5, and picking it up must not drop its floor for
somebody at 0. It is the only rule right in both directions.

Equipment hooks in through `lib/equipment.lua:refresh_slot`, from an item field:

```lua
grants_abilities = { "cleave" }
```

Same `"equip:<slot>"` source key as the effect auras, so taking it off revokes by
the same reconciliation, and `refresh_all` on login rebuilds it with no new call.

## Creatures

A creature uses abilities through the same call. Its cooldowns go to a third
`cooldown_d` namespace and are **memory-only by construction, not by threshold**:
a mob instance id is `mob:<seq>` from a sequence that restarts with the process,
so a durable one would come back after a reboot attached to a different creature.

## Commands

| | |
|---|---|
| `perform <ability> [at <target>]` | aliases `ability`, `perf`. **Not `use`** — `cmds/use.lua` has meant "use an item" since items existed |
| `abilities [category]` | what you know, with cost, readiness and where it came from |
| `cast <spell>` | this game's spell-flavoured alias, unchanged |
| `affect grant/ungrant/abilities` | admin, under `cmd.affect` |

## Spells

`game/daemons/spell_d.lua` is now a vocabulary over this: a spell is an ability
with `category = "spell"`. It translates the four legacy fields (`cost` a bare
mana number, `level` a bare minimum, `cast` a function of
`(player, target, power)`) and projects them back out, so anything that read
`spell.cost` still reads a number.

## Configuration

```toml
[game]
ability_sweep_seconds = 2   # completes a cast whose timer never fired
ability_gcd_seconds   = 0   # a global cooldown; 0 disables it
```

The GCD is off by default: it is a design decision about pacing, not a property
of having abilities at all.

## Classes

There is no class system, deliberately — classless games have to work. A future
one needs nothing new here: a class is a source and a set of rank traits, so
`class_d.set_class(entity, "warden")` would call `set_source_abilities(entity,
"class:warden", specs)` and `trait_d.set_base` for the ranks it opens.
`ability_d` never learns the word "class", which is also why a classless game
needs no stubs.

## What it will not do

- **No formula strings.** Write a function.
- **No prediction or rollback.** A MUD does not need them.
- **No cost against anything but a gauge.**
- **No more than one working at a time.** Something that should run alongside
  another ability is an effect, not a cast.
- **No refunds.** That is policy, and `on_interrupt` is three lines away.

## See also

- [Traits](./traits.md) — where a rank lives, and how `spell_power` reaches a fireball
- [Effects](./effects.md) — the pipeline an ability's damage goes through, and what a channel *is*
- [Creatures & Combat](./combat.md) — `take_damage`, and starting a fight
- [State Cache](./state-cache.md) — why a cast is memory and a grant may not be
