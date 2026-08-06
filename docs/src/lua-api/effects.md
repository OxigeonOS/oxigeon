# Effects — Buffs, Debuffs, and the Event Pipeline

An effect is a temporary thing on a character that gets a say in what happens to
it. When the game is about to do something, it hands the numbers over, every
effect that cares changes them, and the game uses what comes back.

```lua
local ev = DAEMON.effect.run(target, "damage_taken", { amount = 30 })
if not ev.cancelled then
    target:take_damage(ev.amount)
end
```

Defining one:

```lua
DAEMON.effect.define({
    id = "stoneskin", label = "Stoneskin", duration = 60, potency = 5,
    modifiers = { constitution = 2 },
    hooks = {
        damage_taken = { phase = "mult", fn = function(ev)
            ev.scale = ev.scale - 0.15                       -- 15% less damage
        end },
        ["damage_taken#flat"] = { hook = "damage_taken", phase = "reduce",
            fn = function(ev, ctx)
                ev.amount = math.max(0, ev.amount - ctx.potency)   -- and 5 flat
            end },
    },
})
```

## Why this exists

The alternative is storing a modifier on the thing being modified — see
[Traits](./traits.md) for why that goes wrong. Effects replace it: nothing is
ever written to the trait, so nothing has to be unwritten.

But the more interesting half is **ordering**. Those two handlers above, applied
to a 30-point hit, give different answers depending on which runs first:

```
percentage first:  30 × 0.85 = 25.5,  then −5  =  20.5  →  20
flat first:        30 − 5     = 25,    then ×0.85 = 21.25  →  21
```

Nobody should have to know which buff landed first to predict their own health.
So order is a property of the **phase** a handler declares, not of registration:

| Phase | For |
|---|---|
| `pre` | immunity, validation, outright cancellation |
| `add` | flat additions to the base amount |
| `mult` | scaling — handlers add to `ev.scale`, they never touch `ev.amount` |
| — | **the fold**: `amount = amount × (1 + scale)`, exactly once |
| `reduce` | flat reductions, applied after scaling — armour, negation |
| `clamp` | floors and ceilings |
| — | `ev.min` / `ev.max` applied |
| `post` | observation and side effects |

Ties inside a phase break by `order`, then definition id, then application
order. Nothing anywhere iterates with `pairs`, so the same set of effects always
produces the same number.

> [!NOTE]
> **Multipliers add rather than compound.** Handlers accumulate into `ev.scale`
> and the fold happens once, so two +20% buffs are +40%, not +44% — and the
> `mult` phase becomes genuinely order-independent. A game that wants
> diminishing returns changes one function in `mudlib/lib/effects.lua` and
> nothing else.

## Definitions and instances

Keeping these apart is what makes the system saveable.

**Definitions** live in code (`game/effects/*.lua`). They hold functions. They
are never written anywhere.

**Instances** live in the [state cache](./state-cache.md) and are plain data —
no functions, no metatables, nothing that could not survive both
`Player._deep_copy` and a trip through JSON:

```lua
{ def = "regeneration", start = 1754151000, expires = 1754151300,
  source = "potion:regen_draught", stacks = 2, potency = 25,
  caster = 42, last_tick = 1754151297, state = { charges = 3 } }
```

### Definition fields

| Field | Default | |
|---|---|---|
| `id` | — | required |
| `label`, `desc` | the id | shown by `effects` |
| `duration` | — | seconds; nil means permanent |
| `stack` | `"refresh"` | see below |
| `max_stacks` | 1 | |
| `persist` | true | `false` never writes it — for equipment and auras |
| `survives_death` | false | |
| `tick` | — | seconds between `heartbeat` firings |
| `potency` | — | a magnitude handlers can read from `ctx` |
| `condition` | — | a [checks.lua](./daemons.md) predicate; false means it never lands |
| `modifiers` | — | sugar, below |
| `hooks` | — | `{ [name] = { hook, phase, order, fn } }` |
| `on_apply`, `on_refresh`, `on_expire` | — | `function(ctx)` |

`modifiers = { strength = 2, max_hp = "+10%" }` desugars into ordinary
`trait:<id>` handlers at definition time — a number becomes an `add` handler, a
percentage becomes a `mult` one, and both scale with the stack count. Authors
get a table; the runtime keeps one mechanism.

### Stacking

| `stack` | Behaviour | Same effect means |
|---|---|---|
| `refresh` *(default)* | extends the expiry; `on_refresh` fires | same definition **and source** |
| `stack` | increments `stacks` up to `max_stacks` | same definition |
| `independent` | a separate instance on its own clock | never |
| `ignore` | refuses while one is running | same definition |
| `replace` | the old one expires, the new one lands | same definition |

Re-applying never *shortens* an effect: a weaker second cast cannot cut a
stronger first one short.

## Passive modifiers are the same pipeline

A trait's value runs through the hook family `trait:<id>`:

```lua
DAEMON.trait.value(player, "strength")
--> effectively: run(player, "trait:strength", { amount = base })
```

So a `+2 strength` ring and a `−15% damage` buff are written the same way, and
things a static modifier table cannot express become ordinary:

```lua
hooks = { ["trait:strength"] = { phase = "add", fn = function(ev, ctx)
    -- a berserker bonus, which no declarative table could say
    if ctx.entity:trait("hp") < ctx.entity:trait("max_hp") / 2 then
        ev.amount = ev.amount + 5
    end
end } }
```

Derived traits read *effective* dependency values, so a modifier flows through
the whole dependency graph: +2 constitution raises max health without either
effect or trait knowing about the other.

The cost objection is answered by memoization and a per-scope handler index
rather than by a second mechanism. With no effects, `modify` is a nil check that
returns the number it was handed — 0.25 µs, no allocation.

## Sources and lifecycle

The `source` string carries a scheme that says how the effect ends:

| Prefix | Ends when | Persisted |
|---|---|---|
| `potion:`, `spell:` | it times out | yes |
| `equip:<slot>` | the slot changes | no |
| `room:<room_id>` | you leave | no |
| `quest:`, `admin:` | explicitly | yes |

The last three all use one primitive:

```lua
DAEMON.effect.set_source_effects(entity, "equip:head", { { def = "hearty" } })
```

*The effects from this source are now exactly these.* It adds what is new,
removes what is gone, and leaves the rest alone — so it is safe to call on every
login and every change without working out what it did last time.

## Expiry

Both lazily and on a sweep, because they answer different questions.

**Lazily**, on any read: an expired instance is dropped and stops modifying
anything, guarded by one cached comparison so the common case is cheap.

**On a sweep** (`effect.sweep`, every `game.effect_sweep_seconds`): because
`on_expire` has to fire and its message has to be sent even for a player who is
typing nothing. "The stone sheen fades from your skin."

**Ticking** effects are driven by one shared heartbeat
(`game.effect_heartbeat_seconds`), never a timer per effect. Each instance earns
whole ticks from its `last_tick` and advances by exactly the ticks it fired — the
same carry-the-remainder rule regeneration uses, so a coarse heartbeat loses no
accuracy and one that skipped a beat catches up.

Two global timers for the whole system, whatever is running on whom.

## Persistence

Instances live in two cache namespaces: `effects` (write-behind) and
`effects_fast` (memory, for anything with `persist = false`).

`effects` sets `min_lifetime = 30`, which is where a requirement becomes a
property of the tier rather than a special case:

> An effect with less than 30 seconds left is never written. Not because writing
> it would be slow — because by the time the server came back it would have
> expired anyway, so persisting it either does nothing or resurrects something
> that should be gone.

A twenty-second haste is fully live in memory and never touches the database. An
hour-long blessing is written on the next flush and survives a restart. Both
fall out of the same rule.

## A channel is an effect

Worth knowing because it is not obvious from either side: when an ability
declares `channel = { duration = 6, tick = 2 }`, `ability_d` generates one
effect definition for it — `channel_<ability id>`, made lazily, `persist =
false` — and applies it to the caster. There is no separate channel mechanism.

Every single thing a channel needs, this daemon already had:

| a channel needs | here |
|---|---|
| a timed thing attached to an entity | the instance model |
| something every N seconds | `tick`, and the heartbeat's carry-the-remainder rule |
| to end, and to know *why* | `on_expire(reason)` — `"timeout"` is completion, anything else is an interrupt, which is exactly the distinction |
| to end for a player typing nothing | the sweep, which exists for precisely this |
| to end on death and on logout | `death_d`'s `clear`, `character_d.unload`'s `detach` |
| to be visible | the `effects` command, with no special case |
| re-entrancy safety | the per-scope guard and the depth cap |

So a channel shows up in `effects` like anything else, and the cost of the choice
is one line of arithmetic: tick granularity is the shared
`effect_heartbeat_seconds`, so `channel.tick` rounds up to a multiple of it.

A **cast time** is not an effect. It is one deadline with no intermediate
behaviour, and `ticker_d` does that in a line.

This is the same trick `lib/equipment.lua` uses for `equip_trait_<id>`: an
effect's hooks are fixed at define time, so one generated definition per subject
is the established answer here.

See [Abilities](./abilities.md).

## Worked example: the four requirements

```lua
-- 1. 20% more experience, stacking three times
{ id = "insight", duration = 3600, stack = "stack", max_stacks = 3,
  hooks = { xp_gained = { phase = "mult", fn = function(ev, ctx)
      ev.scale = ev.scale + 0.20 * (ctx.stacks or 1)
  end } } }

-- 2 and 3. 15% less damage, and negate 5 from each hit
{ id = "stoneskin", duration = 60, potency = 5,
  hooks = {
      damage_taken = { phase = "mult", fn = function(ev)
          ev.scale = ev.scale - 0.15 end },
      ["damage_taken#flat"] = { hook = "damage_taken", phase = "reduce",
          fn = function(ev, ctx)
              ev.amount = math.max(0, ev.amount - ctx.potency) end },
  } }

-- 4. heal 2% of missing health every tick
{ id = "regeneration", duration = 300, tick = 3,
  hooks = { heartbeat = { phase = "post", fn = function(ev, ctx)
      local e = ctx.entity
      local missing = e:trait("max_hp") - e:trait("hp")
      if missing > 0 then
          e:heal(math.max(1, math.floor(missing * 0.02 * ev.ticks)))
      end
  end } } }
```

Number four heals through `entity:heal`, which runs the `heal_received`
pipeline — so a healing-amplification buff composes with it for free, and the
re-entrancy guard is what stops that becoming a loop.

## API

| Function | |
|---|---|
| `define(spec)` / `define_all(list)` | register |
| `apply(entity, id, opts)` | `opts = { source, duration, potency, stacks, caster, state }` |
| `remove(entity, id_or_key, opts)` | |
| `clear(entity, opts)` | `opts.keep_survivors` honours `survives_death` |
| `active(entity)` | live instances, expired ones dropped on the way |
| `has(entity, id)` | |
| `run(entity, hook, ev)` | the pipeline; returns the same table |
| `modify(entity, hook, n)` | the cheap form for one number |
| `set_source_effects(entity, source, specs)` | idempotent |
| `next_expiry(entity)` | what TRAIT_D's memo watches |
| `sweep()` / `heartbeat()` | the tickers |
| `attach(entity)` / `detach(entity)` | lifecycle |

## Things worth knowing

**Cancellation stops the chain.** Set `ev.cancelled` and optionally `ev.reason`;
remaining handlers are skipped and the caller is expected to check.

**A handler that raises does not break the others.** Every one runs inside a
`pcall`, the error is journalled naming the definition, and the pipeline carries
on with what it had. One area author's mistake must not break combat for
everyone.

**Re-entering the same hook on the same entity is refused**, with a depth cap of
8 for cross-hook chains. A silent infinite loop on the game thread is much worse
than a logged refusal.

**An expired instance can reach the database** if it expires between the sweep
and the flush. That is harmless — the load path filters by expiry before
anything else looks at it.

## What it will not do

- **No effects on rooms or items.** Effects attach to a Mobile — a player or a
  mob. A room aura is a `room:` sourced effect on everyone in it.
- **No conditional expiry beyond time and source.** `condition` is checked when
  the effect lands, not continuously.
- **No modifiers on gauges or counters.** See [Traits](./traits.md).
