# Traits — Any Numeric Data, On Any Entity

A trait is a named number on an *entity* — a character, a mob, a sword, a room.
What makes it more than a table field is that its value is *computed* when you
ask: it can be derived from other traits, and it is filtered through whatever
effects the entity is under.

```lua
-- game/traits/core.lua
return {
    { id = "wisdom", kind = "attribute", default = 10 },
    { id = "level",  kind = "counter",   default = 1 },

    { id = "willpower", kind = "derived", depends = { "wisdom", "level" },
      formula = function(t)
          return math.floor((t.wisdom - 10) / 2) + math.floor(t.level / 2)
      end },
}
```

```lua
player:trait("willpower")           --> computed, buffs included
DAEMON.trait.value(player, "wisdom")
DAEMON.trait.set_base(player, "wisdom", 16)
```

`trait()` lives on [`Object`](./object-hierarchy.md), so every kind of object
answers it. That is not an accident of inheritance — it is the point. Stats,
skills, masteries, item durability, spell charges and a room's corruption are
all the same thing, and only one of them is a character statistic.

## Why this exists

The reference point is Evennia's Traits contrib, where every trait stores its
own `mod` field and a buff is a write to the thing it buffs:

```python
self.traits.strength.mod += 2      # when the buff lands
self.traits.strength.mod -= 2      # ...and this had better run
```

Every path that grants a modifier must have a matching path that removes it. Any
route that misses the second one — a disconnect at the wrong moment, an effect
removed twice, a server restart mid-buff — leaves the character permanently
wrong, with no way to tell by looking that anything is off.

**Here a modifier is never stored anywhere.** A buff is an
[Effect](./effects.md), and a trait's value is recomputed from its base whenever
the set of effects changes. There is nothing to unapply, so there is nothing to
get wrong. Remove the effect and the number simply stops being computed that
way.

## The four kinds

The difference that matters is what gets stored.

| Kind | Stores | Value is | Effects may modify |
|---|---|---|---|
| `attribute` | a base | the base, then effects | **yes** |
| `derived` | **nothing** | `formula(deps)`, then effects | **yes** |
| `gauge` | current + a regeneration anchor | the current value, clamped | no |
| `counter` | current | the current value | no |

> [!IMPORTANT]
> **Effects modify `attribute` and `derived` traits. They never modify a `gauge`
> or a `counter`.**
>
> A buff does not modify your current health — it raises `max_hp`, or it heals
> you. Stored-current traits are changed by *events*, not by *modifiers*.
> `DAEMON.effect.define` refuses a `trait:hp` handler by name, at registration
> time, rather than letting you write a buff that silently does nothing.
>
> This is also what keeps value resolution from recursing, and it is why a
> gauge's maximum is an ordinary trait rather than a special field — which in
> turn is why `+10% max health` needs no new mechanism.

## Presence — storage decides what an entity has

An entity does **not** have every trait the game defines. It has the ones it
holds:

| Kind | Present when |
|---|---|
| `attribute`, `gauge`, `counter` | `entity.stats[id]` is a number |
| `derived` | every id in `depends` is present for that entity |
| any, with `always = true` | the entity has been attached |

**Applicability is derived from the data, never declared.** A sword has `dps`
because it has `damage` and `speed`; it has no `willpower` because it has no
`wisdom`. There is no `applies_to` list to maintain and therefore none to rot —
the same reasoning that makes `depends` enforced rather than advisory.

Two consequences, stated rather than left to emerge:

- **Bounds count as dependencies.** `seal()` folds a `max` that names a trait
  into the graph, so a gauge whose bound trait is absent is itself absent. A
  gauge with no ceiling is not the trait that was defined.
- **`always = true`** is the escape hatch for a formula where absent-means-zero
  is a legitimate answer. Expect it to be rare; if it is common, the presence
  rule is the thing that is wrong.

```lua
local sword = { stats = { damage = 12, speed = 2 } }

DAEMON.trait.has(sword, "dps")        --> true   (both deps present)
DAEMON.trait.has(sword, "willpower")  --> false  (no wisdom)
DAEMON.trait.value(sword, "willpower") --> 0     (the default; writes nothing)
#DAEMON.trait.all(sword)              --> 3, not the whole registry
```

> [!IMPORTANT]
> **A value stored under an undefined trait id is inert, not an error.** A save
> holding `swordsmanship` before that trait is defined reads as absent, and
> starts answering the moment the definition lands. A broken or not-yet-loaded
> trait file must not take a character down, the same way a broken area file
> does not.

**Reading an absent trait never materialises it.** `value` answers with
`def.default` so arithmetic stays safe, and writes nothing; `has` is how you ask
the other question. A `value` that materialised would quietly turn every entity
dense again, and the only symptom would be the performance regression sparseness
exists to avoid.

## Learning and forgetting

`set_base` on a trait the entity does not have **creates** it. That is the whole
mechanism — there is no separate grant path, because presence is decided by
storage rather than declared anywhere.

```lua
DAEMON.trait.has(player, "swordsmanship")            --> false
DAEMON.trait.set_base(player, "swordsmanship", 1)    -- learned
DAEMON.trait.has(player, "swordsmanship")            --> true
DAEMON.trait.forget(player, "swordsmanship")         -- unlearned, and the
                                                     -- mastery derived over it
                                                     -- goes with it
```

A derived trait is not stored, so it cannot be forgotten directly — remove one
of the traits it reads instead. `forget` says so rather than failing silently.

## Defining a trait

```lua
{
    id       = "max_hp",
    label    = "Max Health",      -- shown by `score`
    kind     = "derived",         -- what is stored, and how it is computed
    category = "stat",            -- what this number IS; decides which command shows it
    group    = "derived",         -- which heading it sorts under, inside that command
    depends  = { "constitution", "level" },
    formula  = function(t) return 50 + t.constitution * 5 + (t.level - 1) * 10 end,
    min      = 1,                 -- a number, or another trait's id
    max      = nil,
    round    = "floor",           -- "floor" | "ceil" | "round" | "none"
    default  = 0,
    hidden   = false,
    sets     = { "character" },   -- which seed sets start an entity with it
    always   = false,             -- present on every attached entity
}
```

Register them from `game/init.lua`, then seal:

```lua
DAEMON.trait.define_all(require('traits.core'))
DAEMON.trait.define_all(require('traits.skills'))
DAEMON.trait.seal()
```

### The three axes: `kind`, `category`, `group`

They look similar and they genuinely separate. Three different questions were
hiding in one field before `category` existed:

| Axis | Answers | Who reads it |
|---|---|---|
| `kind` | what is stored and how it is computed — attribute / derived / gauge / counter | the engine; load-bearing |
| `category` | what this number *is* in the game's vocabulary — stat, skill, resource, condition, reputation | commands, deciding what to show |
| `group` | which heading it sorts under *within* one command | that command's renderer |

`swordsmanship` is a `counter`; `sword_mastery` is a `derived` percentage over
it. Different `kind`, same `category = "skill"`, and both belong under
`group = "weapon"`. No single field expresses that.

```lua
{ id = "swordsmanship", kind = "counter",   category = "skill", group = "weapon",
  min = 0, max = 100, sets = false }
{ id = "strength",      kind = "attribute", category = "stat",  group = "attributes" }
{ id = "durability",    kind = "gauge",     category = "condition", sets = { "item" } }
```

Three rules keep `category` from becoming a second `kind`:

- **It is freeform**, like a permission string or an area name. The mudlib
  defines no closed list; a game invents `reputation` or `mastery` without
  touching the driver.
- **It defaults to `"stat"`**, so every trait defined before the field existed
  keeps appearing in `score` with no edit.
- **It never changes behaviour.** It is a lens for commands, nothing more. The
  moment a category is tempted to *mean* something — "skills advance by use" —
  that belongs on the spec as its own declared field (`advances = "use"`), not
  implied by a string. Adding a category must not be able to break anything.

**Commands name what they show**, rather than the trait naming where it goes:

| Command | Shows |
|---|---|
| `score` | `category == "stat"`, grouped by `group` |
| `skills` | `category == "skill"`, grouped by `group` |
| `traits` (admin) | everything — `kind`, `category`, `sets` and presence |
| `traits defs` (admin) | the whole registry, present or not |

A trait in a category no command names appears nowhere until someone writes the
command. That is the correct default — a new category should not silently leak
into `score` — and `traits` is always there to find it.

### Seed sets

`sets` decides what an entity **starts** with. It is a creation-time
convenience: after seeding, storage is the truth.

```lua
DAEMON.trait.attach(entity)              -- lifecycle only; gives nothing
DAEMON.trait.seed(entity, "character")   -- write the character set's defaults
DAEMON.trait.seed(sword,  "item")
```

| `sets` value | Seeded by |
|---|---|
| omitted | `{ "character" }` — the migration default, so nothing moved |
| `"item"` or `{ "character", "mob" }` | those sets |
| `false` or `{}` | **nothing**, deliberately |

Saying nothing and saying nothing-at-all are different answers, and the
difference is load-bearing. A skill uses `sets = false`: not having
swordsmanship until you learn it is the point of sparse traits, so a skill has
to be able to say "no set" rather than be tagged into one nobody happens to call
`seed` with — that would make its absence an accident of which call sites exist.

> `sets` is not folded into `category`, though they correlate. `sets` decides
> what gets *seeded*; `category` decides what gets *shown*.

`seal()` works out the evaluation order and reports anything broken. A trait
that depends on something undefined, or that sits in a cycle, is marked failed
and answers with its default — a broken trait file must not take the server
down, the same way a broken area file does not.

### Dependencies are declared, and the declaration is enforced

A derived formula receives a proxy, and reading a trait it did not list raises:

```
trait 'willpower' read undeclared dependency 'strength' (add it to depends)
```

That is deliberate. `depends` is what the cycle detector reasons about, so if it
were allowed to rot the detector's answer would be a lie. Bounds count too: a
trait whose `max` is another trait depends on it, and `seal()` folds that in for
you.

A cycle is reported as a **path**, because "there is a cycle somewhere in your
thirty traits" is not something anyone can act on:

```
TRAIT_D: dependency cycle: willpower -> wisdom -> insight -> willpower
```

## Gauges and regeneration

```lua
{ id = "hp", kind = "gauge", max = "max_hp", min = 0,
  regen = { rate = 1, per = 3, target = "max", offline = false } }
```

**There is no timer.** Regeneration is computed from a stored timestamp when
someone looks — so a thousand idle players cost nothing at all, and a player who
was away for an hour gets exactly an hour's worth on their next prompt.

The arithmetic carries its remainder. At one point per three seconds, ten
elapsed seconds earn three points and consume nine; the tenth second stays in the
anchor and counts toward the next point. Nothing is ever lost to rounding,
however often it is called.

Two rules fall out of that, and both are load-bearing:

- **A settle that earned nothing writes nothing.** The prompt settles every
  gauge on every command; if that reported a change each time, it would dirty
  every online player's state several times a second and undo the entire
  write-behind design.
- **Reaching the target re-anchors.** Otherwise a player at full health banks an
  hour of credit while idle and dumps it the instant something hits them.

`offline = false` re-anchors on login, so three days away does not arrive as a
full bar.

## Where the numbers live

Trait state is stored on `entity.stats`, which
[CHARACTER_D](./character-data.md) already saves — so a trait costs no new
persistence path at all.

```lua
player.stats = {
    strength = 12, wisdom = 14, level = 3,   -- attribute bases
    hp = 74, mp = 50,                        -- gauge currents
    _at = { hp = 1754151000 },               -- regeneration anchors
    -- max_hp, willpower: absent. Derived, never stored.
}
```

> [!WARNING]
> **`entity.stats[id]` is the stored value, not the true one.** For an attribute
> under a buff they differ, and for a derived trait there is nothing there at
> all. Read `player:trait(id)` or `DAEMON.trait.value(player, id)`.

An item's traits live in exactly the same place, and `Object:new` copies a
`stats` table out of the data it is built from:

```lua
{ id = "rusted_shortsword", short = "a rusted shortsword",
  stats = { durability = 40, damage = 6, speed = 2 } }
```

## Cost

`score` renders every trait; the prompt renders four on every single command.
Values are memoized per entity and invalidated by a generation counter, so a
repeat read is two integer comparisons and a table lookup. Measured with
`cargo bench --bench writes`:

| | Cost |
|---|---|
| `value` — memo hit | **0.2 µs** |
| `value` — full recompute (11 traits) | 11 µs |
| `touch` — settle an idle gauge | 1.9 µs |
| `effect.modify` — no effects | 0.25 µs |

A recompute walks **the entity's own traits** in dependency order, not the
registry's. That is the thing that makes hundreds of defined traits affordable:
a player with 40 skills out of 400 defined traits pays for about 50, not 400,
and a sword pays for three.

The present set is cached per entity beside the value memo, guarded by the same
two counters — `set_base` bumps the entity, `define` bumps the generation — so a
new key on an entity already invalidates it. Unlike the value memo it needs no
expiry check: an effect ending changes what a trait is *worth*, never whether
the entity *has* one.

Time never invalidates the value memo directly. The only way the clock can
change a value is by expiring an effect, and that is one cached comparison.

`tests/mudlib/trait_sparsity.rs` pins this behaviourally rather than by timing: define
200 derived traits, hand an entity the two inputs one of them needs, and assert
that exactly **one** formula runs. A timing assertion would be flaky; a call
count is exact.

## API

| Function | |
|---|---|
| `define(spec)` / `define_all(list)` | register |
| `seal()` | order the graph, report what is broken |
| `value(entity, id)` | the effective value; the default when absent |
| `base(entity, id)` | what is stored |
| `has(entity, id)` | does the entity hold it at all |
| `present(entity)` | its trait ids, in dependency order |
| `forget(entity, id)` | take a trait away — unlearning, a stripped enchantment |
| `set_base(entity, id, n)` | attributes and counters; **creates** an absent trait |
| `set_cur(entity, id, n)` | gauges and counters, clamped |
| `adjust(entity, id, delta)` | settles first, then applies |
| `touch(entity)` | settle regenerating gauges |
| `all(entity, category)` | what the entity holds, optionally one category |
| `categories(entity)` | which categories those fall into |
| `attach(entity)` | lifecycle — cheap enough for every item instance |
| `seed(entity, set)` | write one named set's defaults |
| `detach(entity)` | drop the memo |
| `bump(entity)` / `bump_all()` | invalidate |
| `defs()` / `get_def(id)` / `errors()` | the registry, and what `seal` refused |

`Object:trait(id)` and `Object:has_trait(id)` are the ergonomic forms, and work
on every kind of object.

## Migration

- **`Mobile:new` no longer filters stats through a fixed list of nine keys.**
  Any other key was silently dropped on load even though `to_save` had written
  it — a trait named `wisdom` would have vanished on every login.
- **A stored value for a trait that is now derived is dropped on attach.** A
  saved `max_hp` would shadow the formula forever. The gauge is then clamped
  into whatever range the formula now gives.
- **Existing characters need nothing for sparseness.** They already have all
  eleven traits materialised, so every one is present and behaviour is
  identical.
- **`Mobile:stat` is now `Object:trait`.** A hard rename with no alias: this
  codebase deleted `create_sandboxed_env` to have one boundary rather than two,
  and a compatibility alias is the same debt.
- **`Mobile.skills` is deleted.** It existed as a parallel `skill -> level` map
  precisely because traits could not be sparse. `Player:from_save` migrates a
  saved `skills` table into `stats` on load; `skills` has left `SAVE_FIELDS`.
  Storage moved, nothing is lost, and a skill gains clamping, bounds and a
  derived mastery for free.

## What it will not do

- **No `applies_to` list.** Presence comes from storage, so there is no
  declaration to keep in sync with reality.
- **No string traits.** A trait is a number. Descriptive bands over a value are
  a rendering concern.
- **No modifiers on gauges or counters.** See above; this is the design, not a
  gap.
- **No dependency index on derived traits.** Building the present set tests each
  derived def's `depends`, which is O(derived defs) — and derived traits are the
  small population, because skills and masteries are counters. Worth indexing
  only if that stops being true.
