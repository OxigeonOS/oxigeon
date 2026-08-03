# Traits — Character Attributes

A trait is a named number on a character. What makes it more than a table field
is that its value is *computed* when you ask: it can be derived from other
traits, and it is filtered through whatever effects the character is under.

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
player:stat("willpower")            --> computed, buffs included
DAEMON.trait.value(player, "wisdom")
DAEMON.trait.set_base(player, "wisdom", 16)
```

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

## Defining a trait

```lua
{
    id      = "max_hp",
    label   = "Max Health",      -- shown by `score`
    kind    = "derived",
    group   = "derived",         -- how `score` groups it
    depends = { "constitution", "level" },
    formula = function(t) return 50 + t.constitution * 5 + (t.level - 1) * 10 end,
    min     = 1,                 -- a number, or another trait's id
    max     = nil,
    round   = "floor",           -- "floor" | "ceil" | "round" | "none"
    default = 0,
    hidden  = false,
}
```

Register them from `game/init.lua`, then seal:

```lua
DAEMON.trait.define_all(require('traits.core'))
DAEMON.trait.seal()
```

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
> all. Read `player:stat(id)` or `DAEMON.trait.value(player, id)`.

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

A recompute walks the whole entity in dependency order rather than tracking
which trait invalidated which. At a couple of dozen traits that is a couple of
dozen arithmetic expressions, only when something actually changed, and it is a
great deal less code than the alternative.

Time never invalidates the memo directly. The only way the clock can change a
value is by expiring an effect, and that is one cached comparison.

## API

| Function | |
|---|---|
| `define(spec)` / `define_all(list)` | register |
| `seal()` | order the graph, report what is broken |
| `value(entity, id)` | the effective value |
| `base(entity, id)` | what is stored |
| `set_base(entity, id, n)` | attributes and counters |
| `set_cur(entity, id, n)` | gauges and counters, clamped |
| `adjust(entity, id, delta)` | settles first, then applies |
| `touch(entity)` | settle regenerating gauges |
| `all(entity)` | every trait, for `score` |
| `attach(entity)` / `detach(entity)` | lifecycle |
| `bump(entity)` / `bump_all()` | invalidate |
| `errors()` | what `seal` refused |

`Mobile:stat(id)` is the ergonomic form and works on mobs as well as players.

## Migration

Adding traits changed two things about existing characters:

- **`Mobile:new` no longer filters stats through a fixed list of nine keys.**
  Any other key was silently dropped on load even though `to_save` had written
  it — a trait named `wisdom` would have vanished on every login.
- **A stored value for a trait that is now derived is dropped on attach.** A
  saved `max_hp` would shadow the formula forever. The gauge is then clamped
  into whatever range the formula now gives.

## What it will not do

- **No per-trait invalidation.** Whole-entity recompute, because the entity is
  small and the bookkeeping would not be.
- **No string traits.** A trait is a number. Descriptive bands over a value are
  a rendering concern.
- **No modifiers on gauges or counters.** See above; this is the design, not a
  gap.
