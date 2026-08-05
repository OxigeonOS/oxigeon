# The Market

*A stone arcade with two shops off it, and a third up at the forge.*

`east` from the square. `north` for Hobb's, `south` for the apothecary.

## Three shops, three personalities

```
> north
Hobb's Provisions

> list
Hobb's Provisions
  "Everything's on the shelves. Coin first."

  item                                price  stock
  a coil of hemp rope                    12      6
  a hooded tin lantern                   45      3
  a bundle of hard rations                8     12
  a bent iron lockpick                   30      2
  a battered leather backpack            25      2

You have 0 gold.
```

You have no money. That is the first thing the economy proves:

```
> buy lantern
You cannot afford that — it costs 45 gold.
```

`Player:spend_gold` returns `false` when you cannot afford something, and that
return value was the entire reason it returned anything. Nothing had ever read
it.

Do a quest — [the apothecary's](#quests) is the easiest — and come back.

### The three of them differ where it matters

| Shop | Sells at | Pays | Will buy |
|---|---:|---:|---|
| **Bellow & Son** (smithy) | ×1.0 | ×0.33 | weapons and armour only |
| **Hobb's Provisions** | ×1.0 | ×0.25 | **anything** with a value |
| **The Apothecary** | ×1.2 | ×0.20 | herbs, reagents, potions |

The gap between what a shop charges and what it pays is the gold sink, and it is
a **per-shop** number rather than a constant so that one shop can be a bad place
to sell. Hobb takes anything, which is why he pays badly for it. The apothecary
charges a premium and pays almost nothing, which is what a specialist who knows
you have nowhere else to go looks like in a table.

```
> sell rope            (at the smithy)
They have no use for that.

> sell rope            (at Hobb's)
You sell a coil of hemp rope for 3 gold.
```

The refusal names the reason. A shop that just said "no" is a shop you would try
again.

### Stock, and the one that never comes back

```
> list                 (at the smithy)
  a pitted iron greatsword              200      1
```

One greatsword. Buy it and it is gone for good: the stock line declares
`count = 1, restock = 0`, and those are two different numbers on purpose.
`count` is what the shop opens with; `restock` is what comes back. One number
for both would make "stocks one" and "always has one" the same statement.

Everything else refills on a **task**:

```
> tasks
Tasks:
  board.sweep        Remove expired notices     every 3600s   never   runs 0     ACTIVE
  shop.restock       Restock every shop         every 600s    never   runs 0     ACTIVE
  weather.advance    Advance the weather        every 300s    never   runs 0     ACTIVE

> tasks run shop.restock
```

A raw ticker is anonymous and fire-and-forget. A task has an id you can list,
pause, resume and run on demand — which is what an operator needs at three in
the morning when one periodic job is misbehaving and the rest must keep running.
`task_d` had shipped with no users at all until the shops needed it.

### The ledger

Every transaction is written to the document store, and running totals are kept
with `db_incr` rather than by summing the ledger — two sales in one tick must not
lose one, and summing a growing table to print two numbers gets slower every day
the shop is open.

You cannot see it from a player command. From the admin side:

```
> affect cache
```

or in Lua, `DAEMON.shop.ledger({ char_id = 42 })` and
`DAEMON.shop.totals("thornhollow_smithy")`.

## Buying the things that matter

The lantern is the one purchase this world genuinely requires. The mine is
pitch dark and nothing else lights it.

```
> buy lantern
> use lantern
You open the hood. Warm yellow light fills the space around you.
```

`use` toggles it, and **the lit flag is per instance**. Buy two and light one:
the other stays dark. That is object state keyed on the item's own uuid rather
than a field on the shared template, which would have lit every lantern in the
game at once.

## Equipment

The smithy sells the gear.

```
> buy jerkin
> wear jerkin
You wear a scuffed leather jerkin (chest).

> eq
You are using:

  head      (empty)
  neck      (empty)
  chest     a scuffed leather jerkin  (defense 3)
  ...
```

Try the greatsword:

```
> buy greatsword
> wield greatsword
Requires 16 strength
```

One refusal path, in `lib/requires.lua`, shared by weapons and armour — and read
through the *entity* rather than its stored stats, so a strength buff genuinely
lets you lift it.

Buy the buckler as well and you can see the two-handed rule from both sides:

```
> wear buckler
> wield greatsword          (with the strength for it)
You stop using a small oak buckler.
You wield a pitted iron greatsword (weapon).
```

and then

```
> wear buckler
You stop using a pitted iron greatsword.
You wear a small oak buckler (offhand).
```

Both hands are on the sword, so the shield displaces it and vice versa. The
displaced piece is **named out loud** — "you wield the greatsword" while a shield
silently comes off is how a player loses track of what they are holding.

## What wearing something actually does

The circlet, if you can find one, is the clearest case:

```
> score
  Intelligence      10

> wear circlet
> score
  Intelligence      12  (+2 from effects)
```

That `+2` is not a field on your character. It is an **effect**, applied to the
source `equip:head`, and it is `persist = false` — never written anywhere. What
is saved is that you are wearing a circlet; the bonus is derived from that on
every login.

```
> affect list
  equip_trait_intelligence   equip:head   permanent
```

> [!IMPORTANT]
> **This is the design decision the whole trait system is built on.** A buff is
> never a write to the thing it buffs. Every path that grants a modifier would
> otherwise need a matching path that removes it, and any route that misses the
> second one — a disconnect at the wrong moment, an effect removed twice —
> leaves the character permanently wrong with no way to tell by looking.
>
> Take the circlet off and the number simply stops being computed that way.

## Containers

```
> buy backpack
> put rations in backpack
> examine backpack
a battered leather backpack
  It contains:
    a bundle of hard rations
  Capacity: 1 / 12
```

The backpack holds twelve items and forty units of weight, and refuses by name:

```
There is no room left in it.
It will not take the weight.
```

Two different problems with two different fixes. A player told only "you can't"
will try the same thing again.

Contents survive logging out. They live in the item daemon's location index —
which is memory only, correct for a sword on a floor and wrong for a backpack —
so on save they are folded onto the entry and on load they are put back.

## Quests

Three of the five givers are here or next door.

```
> quest                    (at the apothecary)
On offer here

  roots_for_the_apothecary  Roots for the Apothecary  (level 1)
      Bring the apothecary three bunches of marshroot.
      — the apothecary

> quest accept roots_for_the_apothecary
Accepted: Roots for the Apothecary
```

You cannot take it from across town:

```
> quest accept roots_for_the_apothecary    (from the square)
You would have to ask them yourself.
```

which is what makes a giver more than a label.

The five quests, and what each one is really demonstrating:

| Quest | Giver | Shape | Tier it proves |
|---|---|---|---|
| Roots for the Apothecary | apothecary | collect 3 marshroot | recomputed from your inventory |
| Thin the Crawlers | guard | kill 5 reed crawlers | **write-behind** counter |
| Word to the Deep | smith | visit the pump house | fires on `room.entered` |
| The Day's Ore | smith | bring 1 iron ore, **daily** | **durable** cooldown |
| What Is Down There | smith | kill the Delver | **chained** on a save-field flag |

See [The Collapsed Mine](./mine.md) for the last two, which is where they lead.

> [!NOTE]
> **A quest system needs all three persistence tiers at once**, and choosing
> wrongly for any of them is invisible until it is not.
>
> - "Have I ever finished this" is a forever answer → a character SAVE_FIELD.
> - "How many rats so far" is written on every kill and read almost never →
>   write-behind. A database write per rat is a design mistake.
> - "Have I done this today" must survive an area reset → a durable cooldown,
>   **not** room state.
>
> The fetch quest is the odd one: its progress is recomputed from what you are
> holding rather than incremented, because an item picked up, dropped and picked
> up again is one item and a counter would call it two.
