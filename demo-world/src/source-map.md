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
│   ├── rooms.lua        6 rooms, the cauldron puzzle, the scrying mirror
│   ├── items.lua        the potions and the vial
│   ├── gear.lua         weapons, armour and containers — the equipment showcase
│   └── mobs.lua         rats and a mephit
├── thornhollow/
│   ├── init.lua         ROOM_D.merge over the three files below
│   ├── square.lua       square, smithy, tavern, west gate
│   ├── market.lua       arcade, general store, apothecary
│   ├── undercroft.lua   stair, undercroft, crypt
│   ├── items.lua        provisions, potions, the vault
│   ├── mobs.lua         seven townsfolk
│   └── shops.lua        three shops — registered against rooms, not in them
├── greywater_marsh/
│   ├── rooms.lua        five rooms, all lfun descriptions
│   └── mobs.lua         lurkers, crawlers, the Wisp
└── collapsed_mine/
    ├── rooms.lua        six rooms, the door, the levers, the seam
    ├── items.lua        ore, the pick, the claw, the corpse
    └── mobs.lua         crawlers and the Delver
```

`shops.lua` being separate from `market.lua` is worth a sentence: a shop is a
registration *against* a room rather than a property of one, which is what lets a
shop move without editing a room and a room be rebuilt without losing its shop.

## The game's own systems

```
game/
├── init.lua              registers everything, each step in its own pcall
├── setup_roles.lua       which roles exist and what they carry
├── daemons/
│   ├── aggro_d.lua       whether an aggressive creature attacks
│   ├── weather_d.lua     and what the sky is doing
│   ├── level_d.lua       the experience curve
│   ├── quest_d.lua       what a quest is
│   ├── board_d.lua       the notice board
│   ├── spell_d.lua       casting
│   ├── reach_d.lua       the virtual provider
│   └── gmcp_game_d.lua   Game.Quest
├── traits/
│   ├── core.lua          22 traits: attributes, gauges, derived, hidden
│   ├── skills.lua        five skills, in no seed set
│   └── broken_example.lua  deliberately broken; loaded only by a test
├── effects/
│   ├── core.lua          the worked examples, plus wardskin
│   ├── marsh.lua         poison, chill, the Wisp's mark
│   └── mine.lua          the Delver's Regard
├── spells/core.lua       four spells, one per mechanism
├── quests/thornhollow.lua  five quests, one per persistence shape
└── cmds/
    ├── board.lua  quest.lua  quests.lua  cast.lua  navigate.lua
```

**Every one of these daemons is a policy decision.** `aggro_d` is the clearest:
the driver ships `Mobile.aggressive` and the `room.entered` event and takes no
position on what should happen next. `level_d` is the same shape — the mudlib
owns `award_xp` and the `xp_gained` pipeline; the *curve* is content.

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

1. `game/areas/thornhollow/square.lua` — what an area file looks like.
2. `game/daemons/aggro_d.lua` — what a game daemon looks like, and the layer
   argument stated in a header comment.
3. `mudlib/lib/carry.lua` — the shape of a mudlib library, and why five verbs
   share one.
4. `mudlib/daemons/trait_d.lua` — the most load-bearing file in the driver, and
   the one whose header explains the most.
5. `tests/world_graph.rs` — the shortest demonstration of why walking a world
   catches what testing it does not.
