# Shops & the Economy

```
> list
Bellow & Son, Smiths
  Bellow does not look up. "On the wall. Prices are the prices."

  item                                price  stock
  an apprentice's dagger                 15      4
  a scuffed leather jerkin               40      3
  a pitted iron greatsword              200      1

You have 250 gold.

> buy dagger
You buy an apprentice's dagger for 15 gold. You have 235 left.
```

`Item.value` and `Player:award_gold` / `spend_gold` existed and had no shop to
meet. `spend_gold` returning `false` when you cannot afford something was the
entire reason it returned anything, and nothing had ever read it.

`shop_d` is in the **mudlib** because the mechanism — a stock list, a price, a
restock, a ledger — is the same for every game. Which shops exist, what they
sell and what they say is content, and lives in an area file.

## Declaring a shop

```lua
-- game/areas/thornhollow/shops.lua
return {
    {
        id        = "thornhollow_smithy",
        name      = "Bellow & Son, Smiths",
        room      = "thornhollow.smithy",
        keeper    = "town_smith",              -- a mob template, for flavour
        greeting  = "\"On the wall. Prices are the prices.\"",
        buy_rate  = 1.0,                       -- what it charges, × item.value
        sell_rate = 0.33,                      -- what it pays
        buys      = { "weapon", "armour" },    -- tags it will take; "*" = anything

        stock = {
            { item = "apprentice_dagger", count = 4 },
            { item = "iron_lockpick",     count = 2, price = 30 },
            { item = "iron_greatsword",   count = 1, restock = 0 },
        },
    },
}
```

Register **after** the rooms, because `register` indexes by room and a shop
pointing at a room that does not exist yet is a shop nobody can find and no
error anywhere:

```lua
DAEMON.shop.register_all(require('areas.thornhollow.shops'))
```

### `count` and `restock` are two different numbers

`count` is what the shop opens with. `restock` is what comes back. A unique for
sale declares `count = 1, restock = 0` — one exists, and when it is gone it is
gone. One number for both would make "stocks one" and "always has one" the same
statement, and they are not.

### Prices, and the gold sink

One number on the item (`value`) and two rates on the shop:

| | |
|---|---|
| sells at | `value * buy_rate`, minimum 1 |
| pays | `value * sell_rate`, minimum 1, and **0** for anything it will not take |

The gap is the gold sink. It is a per-shop number rather than a constant so one
shop can be a bad place to sell — Thornhollow's apothecary buys at 0.2 and sells
at 1.2, which is what a specialist who knows you have nowhere else to go looks
like in a table.

A price of at least 1 is not fussiness: a free item is a way to farm gold by
selling it back.

## Where the state goes

Three different answers, chosen by the rule in
[state-cache.md](./state-cache.md):

| | Home | Why |
|---|---|---|
| the shop's definition | memory, from the area file | regenerated on every boot anyway |
| current stock levels | memory, refilled by a task | a shop that forgets it sold three daggers over a restart is behaving correctly — the restock would have refilled them |
| the purchase ledger | the **document store**, written through | "who bought what for how much" is the one thing here nobody wants to lose |

## The restock task

Through [`task_d`](./daemons.md) rather than a raw ticker, which is what
`task_d` is for and what nothing was using it for:

```lua
DAEMON.task.schedule{
    id       = "shop.restock",
    interval = 600,
    label    = "Restock every shop",
    func     = function() return DAEMON.shop.restock_all() end,
}
```

A raw ticker is anonymous and fire-and-forget. A task has an id you can list,
pause, resume and run on demand — which is what an operator needs at three in
the morning when one periodic job is misbehaving and the rest must keep running.

```
> tasks
  shop.restock    Restock every shop        every 600s, run 4 times
  board.sweep     Remove expired notices    every 3600s, run 1 time

> tasks run shop.restock
```

Configure the interval with `game.shop_restock_seconds`, which needs no Rust
change because `[game]` accepts keys the driver has no opinion about.

## The ledger

Every transaction is written, and the running totals are kept with `db_incr`
rather than by summing the ledger — two sales in the same tick must not lose one
to a read-modify-write, and summing a growing table to print two numbers gets
slower every day the shop is open.

```lua
DAEMON.shop.ledger({ char_id = 42 })
DAEMON.shop.ledger({ kind = "buy", gold = { [">"] = 100 } },
                   { sort = "at", order = "desc", limit = 20 })

DAEMON.shop.totals("thornhollow_smithy")
--> { buy_gold = 70, buy_count = 3, sell_gold = 0, sell_count = 0 }
```

This is the first real consumer of `db_insert` / `db_find` / `db_incr` outside a
test of the [document store](./document-store.md) itself.

## Commands

| Command | |
|---|---|
| `list` (`wares`, `shop`) | what is for sale, at what price, how many left |
| `buy <item> [count]` | a trailing number is a count, so `buy leather backpack 2` works |
| `sell <item>` | at the shop's rate, and refused by name when it will not take it |

A sold-out line is **shown** rather than hidden: "it is not here right now" and
"they never had one" are different answers, and a player who cannot tell them
apart will keep coming back to check.

## API

| Function | |
|---|---|
| `register(spec)` / `register_all(list)` | |
| `get(id)` / `all()` / `in_room(room_id)` | |
| `stock(shop_id)` | `{ item_id, item, price, quantity }` per line |
| `find_in_stock(shop_id, name)` | the same matcher every other verb uses |
| `price_of(shop, item_id, line)` / `offer_for(shop, item)` | |
| `buy(player, shop_id, name, count)` | `-> ok, why, sale` |
| `sell(player, shop_id, name)` | `-> ok, why, sale` |
| `restock(shop_id, initial)` / `restock_all()` | |
| `ledger(filter, opts)` / `totals(shop_id)` | |

## Events

```lua
"shop.bought"  -- { shop, char_id, item, count, gold }
"shop.sold"    -- { shop, char_id, item, gold }
```

## What it will not do

- **No haggling, no reputation pricing.** `buy_rate` is per shop, not per
  customer. A game that wants a discount for a faction can listen to
  `shop.bought` and refund, or wrap `buy` — that is content.
- **No shop inventory of instances.** The shelf holds *counts of a template*, so
  something sold to a shop does not come back as the same object. A shop that
  resold your enchanted sword would need per-instance stock, and per-instance
  stock is a container — which the game already has.
- **No currency other than gold.** One number on the Player.
