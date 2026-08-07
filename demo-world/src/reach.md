# The Drowned Reach

*An area with no author. Eighty-one rooms on a side, generated as you walk.*

West from the deep water at the far end of the marsh is `reach.0.0`, and from
there the grid goes out to forty in every direction: 6,561 rooms, none of which
exist until somebody stands in one.

```
> west
The Drowned Reach (0, 0)

Grey water in every direction, moving without going anywhere.

A post stands out of the water with a ring bolted to it.

Obvious exits: east, north, south, west
```

```
> north
The Drowned Reach (0, 1)
> north
The Drowned Reach (0, 2)
```

## The id **is** the room

`reach.3.-7` is not a name for a room. It *is* the room — everything about it is
a pure function of the two numbers, so it can be thrown away and rebuilt
identically whenever nobody is looking at it.

That is what makes eviction free, and eviction is what makes an infinite area
possible at all.

> [!IMPORTANT]
> **`evict_virtual` had zero callers.** Not in the mudlib, not in a game, not in
> a test. `world-building.md` said a virtual room is "cached in the registry
> while occupied"; nothing ever un-cached one.
>
> For a small ocean that is a bounded leak. For a grid this size it is
> unbounded: every room anyone ever walked through stayed in `world_d._rooms`
> forever, holding its exits table, its contents, its actions and its
> description closures.
>
> That is why the eviction work was a **prerequisite** for this area rather than
> a cleanup after it.

A room is dropped the moment its last occupant leaves — on the move, and on
disconnect, because logging out at sea is the common case. Its object state goes
with it, deliberately: anything that must persist out here has to live somewhere
that is not a room.

Walk two hundred rooms into the grid and back:

```
> mudstatus
 Lua heap:    12.4 MB / 64 MB (19%)
```

The number does not climb. `tests/mudlib/state_retention.rs` asserts the registry size
is flat after a 200-step walk, and `tests/demo_world/virtual_rooms.rs` asserts a
regenerated room is byte-identical to the one thrown away.

## Deterministic, not random

Two people standing in `reach.4.4` read the same thing, and so do you an hour
later. The variation comes from a hash of the coordinates, not from
`math.random`:

```lua
local function hash(x, y)
    local h = (x * 73856093) + (y * 19349663)
    ...
end
```

> [!NOTE]
> This is the one place in the entire game where the newly-seeded PRNG would be
> **actively wrong**. Everywhere else — combat rolls, loot, weighted echoes —
> a constant seed was the bug and seeding was the fix. Here, a room that reads
> differently to two people standing in it is the bug, and determinism is the
> fix. Same subject, opposite answer, and the difference is whether the value is
> *remembered* anywhere.

## The edge

```
> navigate reach.500.500
There is no room called 'reach.500.500'.
```

The grid has a boundary at forty. A coordinate space with no bound is one where a
typo sends somebody to `reach.99999999.0` and the pathfinder never returns.

The provider also refuses anything that is not two numbers, so a typo does not
become a valid destination:

```lua
local x, y = room_id:match("^reach%.(%-?%d+)%.(%-?%d+)$")
if not M.in_bounds(x, y) then return nil end
```

## Finding your way home

Walking back is fine if you counted. If you did not:

```
> navigate thornhollow.square
Plotting a course over 68 rooms...
The game does not stop while this happens. Carry on.
```

You can keep playing. A moment later:

```
Route found: 31 step(s).
  east, east, east, ..., up, up
Walk it with `navigate walk`.
```

```
> navigate walk
```

### Why this is worth the trouble

A breadth-first search over the world graph is not slow. It is slow *enough*,
and it gets slower every time somebody adds an area — and the shape of the fix
does not change with the size of the problem, so it is worth having before the
problem is bad.

`compute()` hands the job to a worker thread with its own LuaJIT VM. The worker
has **no efuns at all**: it cannot see the world, cannot send anything, cannot
read the database. It gets a copy of the exit graph and returns a list of
directions.

```lua
local id, err = compute("compute.pathfind", "route", {
    graph = DAEMON.world.exit_graph({ expand = ... }),
    from = here, to = destination,
}, { tag = "navigate:" .. char_id .. ":" .. destination, deadline_ms = 3000 })
```

The pathfinder lives in `mudlib/compute/pathfind.lua`, not in the game layer —
breadth-first search over a graph of room ids is not *this game's* algorithm.
What to do with a route is, and that is `game/cmds/navigate.lua`.

### The most important line

```lua
local still, broke = DAEMON.world.still_connected(route.rooms, player)
if not still then
    player:send("The way has changed since you set out — at " .. broke .. ".")
    return
end
```

> [!IMPORTANT]
> **A compute result is a proposal about a world that has since changed**, never
> an authoritative fact. Nothing stopped the game while the job ran — that is
> the entire point of running it off-thread — so between planning and walking, a
> door may have shut, an area may have reset, and a virtual room may have been
> evicted and regenerated.
>
> The check happens at **walk** time rather than on arrival, deliberately. A
> route planned and walked ten minutes apart is the case that matters; checking
> on arrival would only have caught the fast one.

You can see it work. Plan a route through the mine's grille, then shut the
grille, then walk:

```
> navigate collapsed_mine.deep_workings
Route found: 6 step(s).
> areas reset collapsed_mine
> navigate walk
The way has changed since you set out — at collapsed_mine.second_level.
Plan it again.
```

`still_connected` checks the exits still exist **and** that their `check`
functions still pass — which is the half a graph cannot carry. A locked door is
an exit that is there and refuses.

### The graph, and what is in it

```lua
DAEMON.world.exit_graph()
```

Every static room, with exits as plain strings — no Room objects, no closures,
because the marshaller refuses both and a graph that cannot cross the boundary
is a graph nobody can plan over.

**Virtual rooms are included only if they are cached**, which is to say only if
somebody is standing in one. An infinite grid cannot be enumerated, and
pretending otherwise is how a pathfinder comes to hang. Planning a route *from*
inside the reach passes an `expand` option, which asks the provider for
neighbours breadth-first out to a bounded radius.

## What the reach proves

| Feature | Where |
|---|---|
| `register_virtual` | one provider, `reach.X.Y` |
| `evict_virtual` | on last-occupant-leaves, and on disconnect |
| Regeneration from an id | walk away and back |
| Deterministic generation | a coordinate hash, not `math.random` |
| A bounded coordinate space | the edge at forty |
| `compute()` off the game thread | `navigate` |
| `on_compute_result` dispatch | a handler list the game registers with |
| **Revalidation of a stale result** | `still_connected` before walking |
| Worker VMs with no efuns | the pathfinder cannot see the world |
| `compute_cancel` | `navigate cancel` |

## Things to try here

- Walk twenty rooms out, `mudstatus`, walk twenty back, `mudstatus` again.
- `navigate` somewhere and immediately `navigate cancel`.
- Stand in `reach.0.0` and ask an admin to `stat reach.0.0` — then walk away and
  ask again. The room is gone.
- Plan a route, walk one step by hand, then `navigate walk`. It refuses: you are
  not where you were when you planned it.
