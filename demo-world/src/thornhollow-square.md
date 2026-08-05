# The Square

*The middle of everything. A well, a notice board, and a chapel with no bell.*

```
> look
Thornhollow Square

The square is a lopsided rectangle of packed earth with a stone well off centre,
as if the town grew around the water rather than the other way about...

Obvious exits: down, east, north, south, west
Smell: Woodsmoke, wet stone and something sour off the marsh.
Sound: The creak of the well rope and two people arguing about a goat.
You could try: read the notices, drink from the well
```

## The well — a room action that steals a verb

```
> drink
You haul up the bucket and drink. The water tastes faintly of iron.

> drink
The water is cold and clear, but you have had your fill. (298s)
```

`drink` is a **system command** — it is how you drink a potion. Standing here,
it is the well instead.

That is dispatch order, and it is deliberate: room actions are checked before
system commands, because you are somewhere and where you are should win. Walk
one room north and `drink` is a potion again.

> [!NOTE]
> **The five-minute wait is a cooldown, not a flag on the room.** That looks
> like a detail and is the single most-repeated lesson in this codebase.
>
> Per-character state on a room is wiped by an area reset. A "once per day"
> gate stored as room object state is really "once per fifteen minutes", and
> the bug is invisible until somebody times it. `DAEMON.cooldown` is keyed by
> character, and a cooldown longer than sixty seconds is written through to the
> database, so it survives a reset *and* a restart.
>
> The marsh has the 24-hour version of this. See [Greywater Marsh](./marsh.md).

## The notice board

```
> board
Nothing on the board.

> board post news Ore | The mine is shut and the smith is unhappy about it.
Posted as 357bac5f-e75a-45db-adc5-45aa616ab4a4.

> board
Notice board (1)

  id       cat    subject                          by           when
  357bac5f news   Ore                              benchuser    just now
```

The listing shows the first eight characters of the id. `board read`, `edit` and
`remove` take either that or the whole thing — and an **ambiguous** prefix is
refused rather than guessed, because picking one of two silently is the kind of
wrong that only shows up as somebody deleting the wrong notice.

```
> board read 357bac5f
> board search ore
> board trade
> board mine
> board remove 357bac5f
```

Four categories: `news`, `trade`, `help`, `rp`.

### What the board is really for

Oxigeon ships a document store with twelve `db_*` efuns. Before the board,
**three** of them had a caller anywhere in the game — `db_get`, `db_put` and
`db_delete`, all from the state cache. The entire query half had never been used
by game code.

The board uses all of it:

| Efun | What the board does with it |
|---|---|
| `db_insert` | posting, with a generated id |
| `db_find` | listing, searching, and by-author — with `like`, `in`, `>`, `<=` and `exists` |
| `db_count` | "3 notices" without materialising three notices |
| `db_get` | reading one |
| `db_incr` | **view counts**, atomically |
| `db_update` | editing, as a recursive merge |
| `db_unset` | removing a field outright |
| `db_delete` | taking a notice down |
| `db_exists` | "is that still there" without deserialising it |

The `db_incr` one is worth a sentence. Two people opening the same notice in the
same tick must not lose a count to a read-modify-write, and an atomic increment
is the operation that makes that true without the game thread needing a
transaction.

And `db_unset` exists because **Lua tables cannot hold `nil`**, so RFC 7396's
delete-by-null is unreachable through `db_update` from Lua. Removing a field
needs its own verb wherever "absent" and "false" are different states — which is
exactly the same reason `Player:clear_quest_flag` exists alongside
`set_quest_flag`.

## The watchman

He walks a route: square → market → west gate → square → undercroft stair, one
room every thirty seconds. Stand still and he will come past.

He is `unique`, which means `populate()` can run any number of times and there
will still be one of him. Try it — `areas reset thornhollow` re-populates, and
he does not double.

## Channels

```
> channel list
> channel join chat
> chat hello
[Chat] You: hello
```

`chat` works as a **verb** because you are subscribed to a channel with that
name. The shortcut is checked after room actions and before system commands,
which is the right place: a room with a `search` action and a channel called
`search` should give you the room's.

There is a `staff` channel too. It is gated on a permission you do not have.

```
> channel join staff
You do not have permission to do that.
```

## Look closer — the stair

```
> down
The Undercroft Stair
```

That worked for the first time recently. `down` was in `movement.OPPOSITES` and
used by this room from the day it was written, and there was **no `down`
command**. Nor `up`, `in` or `out`.

Every test that visited the undercroft got there with `goto`. The stair was
described, listed in the exits, and unwalkable.

`tests/world_graph.rs` now cross-references every direction any room uses
against the command registry:

```rust
for dir in used.split(',') {
    assert!(registry.split(',').any(|c| c == dir),
        "rooms use '{dir}' as an exit and no command walks it");
}
```
