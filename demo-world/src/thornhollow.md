# Thornhollow

*Ten rooms, three files, one area. The town everything else hangs off.*

```
                         smithy ---down---> the mine
                            |
   west gate --- square --- market --- general store (Hobb's)
       |            |          |
    the marsh    stair       apothecary
                    |
                undercroft --- west ---> the workshop
                    |
                  crypt
```

Go east from the workshop entrance, then up twice, and you are in the square.

## One file, and what it cost

The town used to be three room files joined by `ROOM_D.merge` — `square.lua`,
`market.lua` and `undercroft.lua`, with an `init.lua` assembling them. The split
was by **place**, not by size: somebody editing the market should not have to
read the undercroft, and a merge conflict in one should not touch the other.

It is one `rooms.lua` now, and that is a loss taken deliberately.
`areaload.inspect` prefers `init.lua` over `rooms.lua` *unconditionally*. So an
OLC-managed thornhollow with a surviving `init.lua` would have had a generated
`rooms.lua` nothing ever read — every `olc save` writing to a dead file and
reporting success. Being editable from inside the game was worth more than the
three-way split, and `ROOM_D.merge` is still there for an area that wants it and
is not managed.

What comes out is what always came out: a single area, one entry in `areas`, one
`_meta`, one reset.

```
> areas
Areas:
collapsed_mine | The Collapsed Mine | Level: 5-15 | Status: live | Rooms: 6
wizard_workshop | The Wizard's Workshop | Level: 1-5 | Status: live | Rooms: 6
thornhollow | Thornhollow | Level: 1-10 | Status: live | Rooms: 10
greywater_marsh | Greywater Marsh | Level: 3-12 | Status: live | Rooms: 5
```

Ten rooms, one entry. Three files went in and one area came out.

> [!NOTE]
> `merge` keeps the `_meta` from the first source that has one, and `load_area`
> iterates with `ipairs`, so the string key `_meta` is skipped automatically and
> is never mistaken for a room. That is why the metadata can sit in the same
> table as the rooms without a wrapper.

## The people

Seven of them, and between them they are the reason half the `Mobile` class
exists at all.

| Who | Where | What they are for |
|---|---|---|
| **Bellow** the smith | smithy | dialogue, a shop, three quests |
| the **apprentice** | smithy | weighted echoes, including an lfun one |
| **Hobb** | general store | a shop that buys anything |
| the **apothecary** | apothecary | a shop that buys almost nothing |
| two **guards** | west gate | `faction`, `stationary` |
| the **drunk** | tavern | echoes, and a joke that takes three visits |
| the **watchman** | square | `patrol`, `unique` |

Every one of those fields — `dialogue`, `faction`, `stationary`, `unique`,
`echoes`, `patrol` — had existed on the class since it was written and **had no
reader anywhere**. `Mobile:get_dialogue` had no callers at all.

## Talking to people

```
> talk smith
Bellow looks up from the bench. "Aye. Buy something or stand somewhere else."

> ask smith about ore
"Nothing's come out of that mine in two years. What I work now is what I can
buy off the barges, and the barges are getting choosy."

> ask smith about son
The hammer stops. "There isn't one." The hammer starts again.
```

Topics worth trying:

| Person | Topics |
|---|---|
| smith | `ore` `mine` `son` `marsh` `sword` |
| hobb | `credit` `rope` `lantern` |
| apothecary | `poison` `marshroot` `wisp` |
| guard | `stone` |
| drunk | `bell` |
| watchman | `crypt` |
| apprentice | `son` |

`talk smith about ore` works too, because people type both.

### The one that reads the world

Ask the smith about a `sword` empty-handed and then again with something in your
hand:

```
> ask smith about sword
"Unarmed, going west? That's one way to find out how deep it is."

> wield dagger
> ask smith about sword
Bellow glances at what you are carrying. "That'll do for what's out there.
Just about."
```

A dialogue answer can be a **function**, and it is called with both the speaker
and the asker. That second argument is the whole reason to make one a function —
an answer that could only see the NPC would be a slower way of writing a string.

> [!NOTE]
> **This did not work until the town needed it.** `get_dialogue` resolved
> responses through `Object.resolve`, which passes the *object* — so an lfun
> answer received the mob twice and never saw who was asking. It takes
> `(mob, asker)` now.

## The rest of the town

- **[The Square](./thornhollow-square.md)** — the well, the board, the watchman,
  and a room action that shadows a system command.
- **[The Market](./thornhollow-market.md)** — three shops with three different
  personalities, and the economy behind them.
- **[The Undercroft](./thornhollow-undercroft.md)** — the town vault, the crypt,
  and a flagstone that needs muscle.

## What the town proves, all told

| Feature | Where |
|---|---|
| `ROOM_D.merge`, multi-file areas | three files, one `areas` entry |
| `Mobile.dialogue`, lfun answers | every shopkeeper |
| `faction` | the two guards share one, and will not fight each other |
| `stationary` / `unique` / `patrol` / `echoes` | the seven townsfolk |
| Shops, prices, restocking, a ledger | [the market](./thornhollow-market.md) |
| Containers | the backpack you can buy, the vault you cannot carry |
| The document store | the notice board, using every filter operator |
| Channels, and the name-as-verb shortcut | `chat hello` |
| Room tags and the reverse index | `outdoor`, which the weather reads |
| Quests | five of them, from three givers |
