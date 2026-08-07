# Body Layouts — What a Creature Is Made Of

```lua
-- game/body/creatures.lua
return {
    layouts = {
        humanoid = {
            features = { "hands", "feet", "eyes" },
            parts = {
                { id = "head",  size = 8,  height = 95, slot = "head",
                  vulnerable = { piercing = 0.25 } },
                { id = "chest", size = 30, height = 70, slot = "chest" },
                { id = "legs",  size = 20, height = 30, slot = "legs" },
                { id = "feet",  size = 5,  height = 5,  slot = "feet" },
            },
        },
    },
}
```

A layout gives a blow somewhere to land, so a helm protects a head and not a
shin, and so an ability can require hands.

> [!IMPORTANT]
> **Optional by absence.** A creature with no layout behaves exactly as it did
> before layouts existed: no location is chosen, **no roll is consumed choosing
> one**, `ev.hit_slot` is nil, and the per-slot armour guard is skipped. There is
> no `if layouts_enabled` anywhere — the nil *is* the compatibility path.

**The mudlib ships no layouts.** A humanoid is game content; a mudlib that
shipped one would be asserting that its creatures have hands.

## The fields

| | |
|---|---|
| `size` | hit weight. **Need not sum to anything** — making a builder balance a column of integers is a tax with no benefit |
| `height` | 0–100, a percentage of *this creature's own* height, so one layout serves a halfling and a giant |
| `slot` | which equipment slot armours this part. **Absent means it cannot be armoured** — a tail, an insect leg |
| `features` | free text an ability can require. On the layout or on a part; both are read |
| `vulnerable` | `damage_type -> fraction`. **Proportional**, deliberately unlike armour's flat `resist`. `+0.25` is a quarter more |
| `multiplier` | a flat multiplier for every damage type on this part |
| anything else | **kept, and it rides through onto the hit result** — so there is no closed field list to rot |

## Attaching one

Three rungs, first match wins:

1. `body` on the creature — a `mob` schema field, so OLC can author it.
2. `race`, if it names a layout. This finally makes that field mean something.
3. `game.combat_default_body`, unset by default.

…then nil, and nil is legal.

## Where a blow lands

```
reach = attacker_height * 1.15 + weapon_length
high  = reach / defender_height * 100
```

Parts inside the window are candidates, weighted by `size`. **If nothing is in
reach, the lowest part is returned** — a halfling with a dagger hits a giant's
shins rather than missing the whole creature. Either height missing disables the
filter entirely, which is the ordinary case for a game with no `height` trait —
and `height` is a trait, because it is a number on an entity, which is the
definition of one.

A ranged weapon skips the window: an arrow reaches the head of anything.

## Armour follows the location

The worn piece records which slot it covers, and the protection handler ignores a
blow that landed somewhere else:

```lua
if ev.hit_slot and state.slot and ev.hit_slot ~= state.slot then return end
```

**When `ev.hit_slot` is nil this is skipped** — and it is nil for every call the
game makes today: `affect damage`, an ability's damage, a poison tick, and every
fight against a creature with no layout.

Armour gained a second, proportional field beside the flat ones:

| | |
|---|---|
| `defense` | flat reduction, `reduce` phase |
| `resist` | flat, per damage type |
| `absorb` | **a fraction**, per damage type, `mult` phase — so it lands *before* the flat reduction, which is the ordering `effects.md` already argues for |
| `shield` | whether this can be blocked with; feeds combat's `block` channel |

`combat_absorb_cap` (0.75) bounds it: flat reduction is uncapped, but a fraction
is not, or enough plate becomes immunity.

## Features and abilities

```lua
requires = { { kind = "body_feature", feature = "hands" } }
```

The payoff for declaring features: an ability that needs hands says so, and a
snake cannot use it.

## Weapons gained two fields for this

| | |
|---|---|
| `length` | reach in centimetres. A spear gets at a giant's chest where a dagger gets at its shins |
| `parry` | whether it can be parried with. A crossbow is not a parrying implement |

## Linting

`verify prototypes` reports layout problems alongside prototype ones — a part
with no size can never be hit, a height outside 0–100 is meaningless, a layout
with no usable parts is dropped. One bad layout never takes the others with it.

## See also

- [Creatures & Combat](./combat.md) — the pipeline this feeds
- [Items, Equipment & Containers](./items.md) — slots, and what armour carries
- [Abilities](./abilities.md) — `body_feature` requirements
