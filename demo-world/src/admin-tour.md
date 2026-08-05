# Admin Tour

*The same world, from the other side.*

The first account created is the superuser and bypasses every permission check,
so if you made the first account these all work. If not, ask whoever did for the
`admin` role.

## Seeing the world

```
> areas
> stat thornhollow.square
> stat benchuser
> objdump benchuser
```

`stat` is the one you will use while walking. It takes a player, a room id, a
creature in the room by prefix, or an item template — and it reads traits
through `:trait()` rather than the stored table, so it shows what is *true*
rather than what is stored. `max_hp` is derived and is not stored at all;
reading `stats.max_hp` reported 0 for every character until this was fixed.

`objdump` is the other half of that pair: `stat` is the readable summary,
`objdump` is the dump. Use it when something is behaving as though a field you
set is not there.

```
> objdump                    the room you are standing in
> objdump mephit             a creature, by keyword, anywhere
> objdump lantern            an item instance you are carrying
> objdump template:iron_ore  a template that has never been spawned
```

Every dump ends with **Raw fields** — every key on the table, sorted, nested one
level, cycles marked rather than followed. That section exists because a curated
dump can only show the fields somebody thought to list, and the reason you are
running `objdump` is usually that the interesting one is not among them.

Traits come through `DAEMON.trait.all` rather than off `stats`, so a derived
trait appears at all and a buffed one shows base and effective side by side.

> [!NOTE]
> It resolved **two** things until recently: an online player by exact name, and
> a room by exact id. Everything else got "Player or room not found" — including
> `objdump rat` about a rat standing in front of you. A room dump also listed
> characters and scenery but neither live creatures nor items on the floor,
> because those live in their daemon's location index rather than on the room
> table, so `objdump` and `look` disagreed about what was in the room.

## Traits, three ways

```
> score          category == "stat"
> skills         category == "skill"
> traits         everything, grouped by category
> traits defs    the whole registry, present or not
```

That last one is the discoverability answer. A trait in a category no command
names appears nowhere — which is the correct default, because a new category
should not silently leak into `score` — and `traits defs` is where you find it.

```
> traits defs
Defined traits (22)
  id                 kind       category     group        sets
  strength           attribute  stat         attributes   character
  swordsmanship      counter    skill        weapon       -
  luck_seed          attribute  stat         derived      character   hidden
```

The three axes are separate on purpose: `kind` is what the engine does with it,
`category` is what it *is*, `group` is where it sorts inside one command.
`swordsmanship` is a counter and `sword_mastery` is derived — different kind,
same category, same group, and no single field expresses that.

## Driving the systems

```
> affect                     the whole menu
> affect traits              every trait, base and effective
> affect damage 30 magic     through the real pipeline
> affect heal 500
> affect apply stoneskin 600
> affect list
> affect learn strength 18   create a trait you do not have
> affect unlearn strength
> affect xp 450              which will level you
> affect settle              settle regenerating gauges now
> affect cache               state cache statistics
```

`affect` is the diagnostic window and, deliberately, the injection point the
real-mudlib tests use — there is no other verb in the game that deals damage on
demand.

The one worth trying twice is mitigation:

```
> affect damage 30
30 requested, 30 dealt.

> wear jerkin
> affect apply stoneskin 600
> affect heal 500
> affect damage 30
30 requested, 17 dealt.
```

30 × 0.85 = 25.5, minus stoneskin's flat 5 and the jerkin's 3, floored to 17.
The percentage applies **first** because of its phase, not because of when it
was applied — swap the order you acquire them in and the answer is the same.

## Roles

```
> role list
> role perms builder
> role who benchuser
> role grant somebody builder
> role allow builder cmd.example
> role refresh somebody
```

Granting takes effect **now**, not on their next login, and the message says so
because everyone assumes otherwise:

```
> role grant alice builder
alice now holds 'builder'. It takes effect now, not on their next login.
```

`has_permission` reads a per-session cache seeded at `enter_game_session`, so
anything that changes what somebody may do has to say so. `assign_role`,
`revoke_role`, `grant_permission` and `revoke_permission` all resync;
`role refresh` is the explicit escape hatch for anything they cannot see.

```
> finger alice
alice
  Account        alice
  Created        2026-08-03T21:14:02Z
  Superuser — bypasses every permission check.
  Roles          builder, player
```

The superuser flag is shown separately because it is an **account** flag rather
than a role, and cannot be granted or revoked with `role`.

## Observability

```
> mudstatus
═══════════════════════════════════════════
 Oxigeon MUD — Server Status
═══════════════════════════════════════════
 Uptime:      2h 14m 3s
 Players:     1 online
 Connections: 1 total sessions
 Areas:       4 loaded
              collapsed_mine, wizard_workshop, thornhollow, greywater_marsh
 Rooms:       27 loaded
 Tickers:     3 active
 Tasks:       3 scheduled
 Events:      10 with listeners
 Daemons:     33 loaded

 Lua heap:    1.8 MB / 64 MB (3%)
```

```
> mudstatus gc
 Full collection: 0.4 MB reclaimed in 3.1 ms, heap now 1.4 MB
```

> [!NOTE]
> **Nothing measured the heap before.** There were zero `collectgarbage` calls
> anywhere and no GC configuration, so LuaJIT ran at its default pause of 200 —
> the heap roughly doubles before a full cycle — against a 64 MB ceiling. The
> signature under pressure is latency spikes first and catchable allocation
> errors second, surfacing in whatever code happened to allocate rather than in
> the code responsible.
>
> These counters exist so that any later GC tuning is justified by a number.
> **Do not tune without one.**

### The heap drill

Record the heap, do something that used to leak, record it again:

1. `mudstatus` — note the number.
2. Walk twenty rooms into the Drowned Reach and back.
3. Kill things in the mine and let them respawn a few times.
4. `reload daemons.quest_d` two or three times.
5. `mudstatus gc`, then `mudstatus`.

It should come back close to where it started. A monotonic climb across all
three is the signature that object-state leaks, uncached virtual rooms and
closure retention on hot reload produce, and it is the only way to tell them
apart from an ordinary working set.

## Logs

```
> journal
> journal 20 error
> audit
> audit watch spawn
```

Two logs, two questions:

- **journal** — *what went wrong?* Daemon loads, errors, hot reloads, warnings.
- **audit** — *who did this?* Privileged commands, permission denials, admin
  actions.

Break something on purpose to see the first one work:

```
> reload areas.thornhollow.nonexistent
```

## Tracing

```
> trace time
> look
> trace timings
```

Per-command timings from a ring buffer. Nothing in this world should exceed
about 2 ms; `look` in a room with creatures and ground items is the expensive
one, and it is not expensive.

```
> trace calls
> cast emberlance at rat
> trace show
```

## Building

```
> olc sandbox
> dig north entrance
> dig east side_room
> verify areas/sandbox/entrance.lua
> reload areas.sandbox.entrance
> north
```

`dig` makes a room, links it, adds the **return exit**, and writes both rooms to
disk as ordinary Lua data files that you can open in an editor. That round trip
is the design: a world in a database is uneditable by every tool that already
exists and unreviewable in a diff.

`verify` compiles without executing, which is the habit worth forming — a syntax
error in a generated area is better caught before a reload takes the area down.

The driver book has the rest, under *OLC — Building In-Game*. (Not linked: it is
a separate mdbook, and a relative link out of this one would go nowhere in the
rendered HTML.)

## Resets

```
> areas reset collapsed_mine
> areas reset greywater_marsh
```

Watch what changes and what does not. The mine's puzzle, door and ore seam all
clear — world state, and refilling it is correct. The marsh's herb gate does
not, because it is per character and lives in a durable cooldown.

That contrast is the single most-repeated lesson in this codebase, and it is
twenty minutes' walk apart so you can check both in one sitting.

## Snooping

```
> snoop alice
> snoop
```

Everything sent to alice is mirrored to you, prefixed. Two refusals are
structural rather than polite: you cannot snoop yourself (the mirror would feed
itself) and you cannot make a chain (a cycle is an infinite loop on the game
thread). Snooping is audited.

## Shutdown

Ctrl+C. The driver dispatches `on_shutdown` and **waits** for it, bounded by
`game.shutdown_timeout_seconds`. Everything in the write-behind cache is
flushed, every loaded character is saved, and nothing is lost.

`kill -9` is the other drill: you lose up to `autosave_seconds` of progress and
nothing earlier. Both are worth doing once.
