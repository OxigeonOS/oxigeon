# Where Everything Lives

Every file this world is made of, and — the more interesting question — **which
layer it is in and why**.

> [!IMPORTANT]
> **The rule.** `mudlib/` is anything a second game would want unchanged.
> `game/` is this game.
>
> The test is never size or subject. It is: would another game want this file as
> it stands, or would it want a different one? A pathfinder is mudlib; the
> command that decides what to do with a route is game. `Mobile.aggressive` is
> mudlib; the daemon that reads it is not.

## The world

```
game/areas/
├── wizard_workshop/
│   ├── _meta.lua        managed = "olc.v1" — the gate on every OLC write
│   ├── rooms.lua        6 rooms, the cauldron puzzle, the scrying mirror
│   ├── items.lua        the potions and the vial, plus all the equipment
│   ├── mobs.lua         three kinds of rat, and a mephit
│   └── custom.lua       the puzzle: pouring, collecting, the teleport
├── thornhollow/
│   ├── _meta.lua
│   ├── rooms.lua        10 rooms — square, market and undercroft in one file
│   ├── items.lua        provisions, potions, the vault
│   ├── mobs.lua         seven townsfolk
│   ├── shops.lua        three shops — registered against rooms, not in them
│   └── custom.lua       the well, the notices, the flagstone, Bellow's lfun
├── greywater_marsh/
│   ├── _meta.lua
│   ├── rooms.lua        five rooms
│   ├── mobs.lua         lurkers, crawlers, the Wisp
│   └── custom.lua       every description — they are all lfuns
└── collapsed_mine/
    ├── _meta.lua
    ├── rooms.lua        six rooms, the door, the levers, the seam
    ├── items.lua        ore, the pick, the claw, the corpse
    ├── mobs.lua         crawlers and the Delver
    └── custom.lua       the lever puzzle, and the two exits with a `check`
```

**Four files, and the split is the point.** `rooms.lua`, `items.lua` and
`mobs.lua` are OLC-owned and rewritten wholesale by `olc save`; `custom.lua` is
hand-written and OLC never reads or writes it. That is what lets every area here
be edited from inside the game without a regeneration silently deleting the room
actions and lfun descriptions that make them worth shipping.

Two consequences visible above:

- **`gear.lua` is gone.** It held ten items and `items.lua` appended it, because
  the loader has five entry names and anything else has to be pulled in by one
  of them. It worked, and it also meant ten items `olc list` could not see and
  `olc save` would not have written back.
- **Thornhollow is one `rooms.lua`.** It was `init.lua` merging `square.lua`,
  `market.lua` and `undercroft.lua`, which split the town by *place* so three
  builders need never touch the same file. `areaload.inspect` prefers `init.lua`
  over `rooms.lua` unconditionally, so a generated `rooms.lua` beside a
  surviving `init.lua` would never be read — every save writing to a file the
  loader ignores. That was the price.

`shops.lua` being separate is worth a sentence: a shop is a registration
*against* a room rather than a property of one, which lets a shop move without
editing a room and a room be rebuilt without losing its shop. It is not a
`GENERATED` kind, so OLC leaves it alone.

## The game's own systems

```
game/
├── init.lua              registers everything, each step in its own pcall
├── setup_roles.lua       which roles exist and what they carry
├── daemons/
│   ├── weather_d.lua     what the sky is doing — reeds, shutters, fog
│   ├── level_d.lua       the experience curve
│   ├── spell_d.lua       a "spell" vocabulary over the engine's `ability`
│   ├── reach_d.lua       the virtual provider — names a room id and an area
│   └── gmcp_game_d.lua   Game.Quest
├── traits/
│   ├── core.lua          attributes, gauges, derived, hidden, and the four
│   │                     combat traits that turn defence channels on
│   ├── skills.lua        five skills, in no seed set
├── effects/
│   ├── core.lua          the worked examples, plus wardskin
│   ├── marsh.lua         poison, chill, the Wisp's mark
│   └── mine.lua          the Delver's Regard
├── abilities/
│   ├── spells.lua        four spells, one per mechanism
│   └── techniques.lua    cleave — the whole spec surface in one record
├── prototypes/beasts.lua   what "a beast in this game" means
├── body/creatures.lua      humanoid, beast, insectile, amorphous
├── quests/thornhollow.lua  five quests, one per persistence shape
└── cmds/
    ├── quest.lua  quests.lua  cast.lua  navigate.lua
```

**Every one of these daemons is a policy decision, and the test is whether it
*names things*.** `weather_d` names reeds and shutters. `reach_d` names a room
id and an area. `level_d`'s `THRESHOLDS` is a design document as much as a
table — the mudlib owns `award_xp` and the `xp_gained` pipeline; the *curve* is
content.

`aggro_d` used to head this list as the clearest example, and it was the wrong
one: it named nothing but two tunable constants, so it is in the mudlib now,
along with `board_d` and `quest_d`. A driver that ships `Mobile.aggressive` and
the `room.entered` event with nothing that reads them is not taking no position,
it is shipping a hole.

## What the driver provides

```
mudlib/
├── init.lua              hooks, daemon loading, the disconnect chain
├── login.lua
├── lib/
│   ├── object.lua        `trait()` lives here, not on Mobile
│   ├── item.lua  weapon.lua  armor.lua  container.lua  requires.lua
│   ├── carry.lua         moving an item between floor, character, container
│   ├── equipment.lua     slots, requirements, `equip:` effect sources
│   ├── light.lua         whether you can see
│   ├── mobile.lua  player.lua  room.lua  movement.lua
│   ├── checks.lua        predicates for conditions and gates
│   ├── commands.lua      the dispatcher
│   └── colour, strings, tables, json-safety, persistence
├── daemons/              25 of them — see docs/src/lua-api/daemons.md
├── compute/pathfind.lua  breadth-first search, on a worker thread
└── cmds/                 ~60 commands
```

### The ones added for this world

| File | Why it is mudlib rather than game |
|---|---|
| `lib/carry.lua` | five verbs asking four questions between them; written five times they drift |
| `lib/equipment.lua` | slots and requirements are the same everywhere |
| `lib/container.lua` | a container is a container |
| `lib/light.lua` | one question, one answer, one place to change it |
| `daemons/item_d.lua` (instances) | "where is this thing" is universal |
| `daemons/shop_d.lua` | stock, price, restock, ledger — the *mechanism* |
| `daemons/tag_d.lua` | a reverse index over tags |
| `compute/pathfind.lua` | BFS over a graph of room ids is not this game's algorithm |
| `cmds/up.lua` `down.lua` `in.lua` `out.lua` | directions the driver already knew about |

## Configuration

| File | What the demo world needs from it |
|---|---|
| `config/server.toml` | `start_room`, `respawn_room`, and `[game]` is open for anything else |
| `config/permissions.toml` | `/areas` gated on `dir.write.areas` — a live rule, not a comment |
| `config/driver.toml` | port 4000, sqlite, log level |

`[game]` accepting unknown keys is what stopped `death_d` — a *mudlib* file —
having one game's room written into it.

## The tests behind the guide

Everything this book claims is asserted somewhere. The files most specific to
the world:

| File | What it pins |
|---|---|
| `world_graph.rs` | no dangling exits, no accidental one-way links, every area reachable from the start room, every direction has a command, **and the guide's opening route** |
| `thornhollow.rs` | the multi-file area, dialogue, factions, echoes, tags, room-action precedence |
| `marsh.rs` | weather in the prose, poison on the heartbeat, conditions, the daily gate surviving a reset |
| `mine.rs` | darkness, the door, the levers, the corpse, the reset contrast |
| `virtual_rooms.rs` | generation, determinism, the graph, revalidation, pathfinding end to end |
| `quests.rs` | all three persistence tiers |
| `shop.rs` / `board.rs` | the economy and the document store |
| `levelling.rs` | the curve, the event, and the level gates becoming reachable |
| `state_retention.rs` | nothing leaks when things stop existing |

That last one is the one to run if you change anything about how objects are
destroyed.

## Reading order, if you are new to the codebase

1. `game/areas/greywater_marsh/rooms.lua` and its `custom.lua` — what an area
   is, and where the line between data and code falls.
2. `game/daemons/reach_d.lua` — what a game daemon looks like, and the layer
   argument stated in a header comment: it names a room id and an area, so no
   other world could want it unchanged.
3. `mudlib/lib/carry.lua` — the shape of a mudlib library, and why five verbs
   share one.
4. `mudlib/daemons/trait_d.lua` — the most load-bearing file in the driver, and
   the one whose header explains the most.
5. `tests/demo_world/world_graph.rs` — the shortest demonstration of why walking a world
   catches what testing it does not.
