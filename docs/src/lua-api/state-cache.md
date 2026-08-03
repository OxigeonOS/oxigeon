# State Cache — Tiering by What You'd Mind Losing

Got some game state? Decide how much you would mind losing it, then put it
somewhere that matches.

```lua
-- Never persisted. If the server restarts, the fight is over anyway.
DAEMON.cache.define("combat", { tier = "memory" })

-- Written on a ticker, on disconnect, and on shutdown.
DAEMON.cache.define("quests", { tier = "write_behind", flush_seconds = 30 })

-- Written the instant it changes.
DAEMON.cache.define("entitlements", { tier = "write_through" })

DAEMON.cache.set("quests", char_id, "rats_killed", 7)
DAEMON.cache.get("quests", char_id, "rats_killed")   --> 7
```

That is the whole interface. The interesting part is the choice, not the API.

## Why this exists

Evennia persists every attribute change straight to the database. Every stat
change, every combat hit, every mob swing is a write, and it becomes the
bottleneck. Oxigeon's document store has exactly the same shape and would
develop exactly the same problem.

Measured on this codebase with `cargo bench --bench writes`, net of a 6.7 µs
harness round trip:

| Operation | Cost |
|---|---|
| A plain Lua table assignment | ~0.01 µs |
| `DAEMON.cache.set` (memory) | **0.8 µs** |
| `DAEMON.cache.get` (memory) | 0.5 µs |
| `set_persistent` | 1.7 µs |
| `set_object_state` | 2.3 µs |
| `db_get` (hit) | 30 µs |
| `db_put` (small document) | **84 µs** |
| `db_update` (one field) | 116 µs |

A document write is roughly a hundred times an in-memory one and it happens
**synchronously on the game thread**. One `db_put` costs about the same as a
whole `look`.

The answer is not a faster store — it is writing less often. The same benchmark,
for N changes to one player's state:

| N changes | write-through | write-behind | |
|---|---|---|---|
| 1 | 128 µs | 121 µs | the same, by construction |
| 10 | 1.20 ms | **0.15 ms** | 8× |
| 100 | 23.3 ms | **0.36 ms** | 65× |
| 1000 | 1077 ms | **2.3 ms** | 473× |

Write-behind is not a faster write. One change costs the same either way. What
it buys is that the second, third and thousandth change to the same scope are
nearly free.

> [!TIP]
> **Redis is not the alternative.** Over loopback it is 30–100 µs per round
> trip — the same order as our own `db_put`, and 30× slower than the in-process
> path. Its default persistence is a snapshot every N seconds, which is exactly
> the guarantee a write-behind cache already gives. It earns its keep when
> state must be shared between processes; a single-threaded MUD driver is not
> that.

## The rule

> **Choose the tier by how much you would mind losing it, not by convenience.**

| Tier | Written | You lose | Use it for |
|---|---|---|---|
| `memory` | never | everything, on restart | combat state, aggro, targets, sub-minute cooldowns, short buffs |
| `write_behind` | on a ticker, on disconnect, on shutdown | up to `flush_seconds`, **only on a crash** | effects, quest counters, statistics, reputation |
| `write_through` | immediately | nothing | daily gates, admin actions, entitlements |

On a clean shutdown you lose nothing at any tier: `on_shutdown` flushes
everything before the VM stops. The `flush_seconds` window only applies to a
hard crash or a kill.

### Which home does this state get?

Three questions, in order:

1. **Would you print it on a character sheet?** Would `who`, a stat block or the
   combat rules read it on every action? → **`player.stats` and `SAVE_FIELDS`**,
   saved by [CHARACTER_D](./character-data.md).
2. **Does another subsystem own its lifecycle** — it ticks, it expires, it
   accumulates? → **`DAEMON.cache`, one namespace per subsystem.**
3. **How much would you mind losing it?** → the table above.

> [!WARNING]
> **Stop putting things in `player.custom`.** `custom` is where state goes when
> nobody decided, and it is how you end up rewriting a 64 KB character blob on
> every autosave. The existence of this daemon is the reason to stop growing it.

## What goes where, concretely

**Never persist:**

- Combat health mid-fight, aggro and threat tables, current target, queued
  actions. If the server reboots, the fight is over. (Maximum health is
  progression and belongs on the character.)
- Buffs shorter than `min_lifetime` — see below.
- Session interface state: pager position, an open OLC buffer. Note that
  `page_length` and `color_enabled` are durable *preferences* and correctly live
  on the Player; the scroll position is not.
- Rate-limit counters. Scope them to the account so a character swap does not
  reset them, but a server restart is not something a player can trigger.
- Room occupancy and mob positions — `world_d` and `mob_d` already own those.

**Write-behind:**

- Effects, which is what this was built for.
- Quest progress counters ("7 of 10 rats"): very high write frequency, almost no
  value per write, real annoyance if lost.
- Per-character statistics — damage dealt, deaths, steps, playtime.
- Reputation and faction standing.
- Room or world state that must outlive an area reset but not a restart.

**Write-through:**

- Long cooldowns. One write per player per day is not worth engineering around.
- Anything with consequence: bans, mutes, role grants, a wizard's edit to
  another player.
- Anything gating a scarce or paid entitlement.

> [!IMPORTANT]
> **Never `db_find` a write-behind collection and expect current answers.** Its
> documents are up to `flush_seconds` stale by design. If you need to query,
> call `DAEMON.cache.flush_namespace(ns)` first — or keep the thing you query
> in a write-through namespace.

## Storage shape

Three levels: **namespace**, **scope**, **key**. They map onto a document
collection, a document id, and a top-level field of that document.

```
namespace "effects", scope_prefix "char:", scope 42
    -> collection "effects", document id "char:42"
    -> { regeneration = {...}, insight = {...} }
```

The scope is the flush unit: one flush is one `db_put` of the whole scope,
however many keys changed.

### `DAEMON.cache.define(namespace, spec)`

| Field | Default | Meaning |
|---|---|---|
| `tier` | `"write_behind"` | `memory`, `write_behind` or `write_through` |
| `collection` | the namespace name | must match `^[a-z][a-z0-9_]*$` |
| `scope_prefix` | `""` | prepended to make the document id |
| `owner` | `"none"` | `"char"` flushes and evicts on that character's disconnect |
| `flush_seconds` | 30 | how often a dirty scope is written |
| `min_lifetime` | 0 | entries with less time left than this are never written |
| `evict_after` | 0 | drop an untouched clean scope after this long; 0 = never |
| `preload` | false | load on login rather than on first read |
| `delete_when_empty` | true | an empty scope is deleted rather than stored empty |
| `expiry_of` | — | `function(key, value) -> unix_seconds \| nil` |
| `on_load` | — | `function(scope, data)` after hydration |

Re-defining a namespace is allowed and is a normal workflow — edit
`flush_seconds`, reload the daemon. Anything that changed is journalled, because
it silently changes durability.

### Reading and writing

```lua
DAEMON.cache.get(ns, scope, key)                    -- loads on miss
DAEMON.cache.set(ns, scope, key, value, opts)       -- opts = { expires_at, ephemeral }
DAEMON.cache.delete(ns, scope, key)
DAEMON.cache.incr(ns, scope, key, delta)            -- in memory, not db_incr
DAEMON.cache.has(ns, scope, key)
```

For several keys at once, use `edit` — one dirty mark and one document write
instead of a dozen:

```lua
DAEMON.cache.edit("effects", char_id, function(scope)
    for key, effect in pairs(scope) do
        effect.remaining = effect.remaining - 1
    end
end)
```

> [!WARNING]
> **`get_scope` returns the live table and does not mark it dirty.** Mutating
> what it hands back may never be written. Use `edit` to change a scope, or
> `copy_scope` if you only want to read it somewhere else.

Also: `set_scope`, `merge_scope`, `clear_scope`, `keys`, `scopes`, `exists`,
`preload`.

### Durability

```lua
DAEMON.cache.flush(ns, scope)          -- now, whatever the schedule says
DAEMON.cache.flush_namespace(ns)
DAEMON.cache.flush_owner(char_id)
DAEMON.cache.flush_all({ reason = "shutdown", deadline = t })
DAEMON.cache.tick()                    -- what the ticker calls
DAEMON.cache.verify(ns, scope)         -- would all of this actually write?
```

`evict` writes then forgets; `drop` forgets without writing, and says so.
`write_offline(ns, scope, fn)` loads, edits, flushes and drops in one call, for
touching an offline player without pinning them in memory forever.

## Things worth knowing

**A flush is one `db_put` of the whole scope, never a patch.** `db_update` is an
RFC 7396 merge, and RFC 7396 expresses deletion as a JSON `null` — which a Lua
table cannot hold. A merge-based flush could add and change but never *remove*,
so an effect that ended would linger in the stored document forever while memory
disagreed. `db_put` makes the document exactly the memory view. It is also
cheaper (84 µs against 116 µs).

**Values are checked when you write them, not when they are flushed.** The
driver's `lua_to_json` refuses six kinds of value — mixed list/map tables,
functions, NaN and infinity, non-string keys, more than 64 levels of nesting,
more than 100 000 values — and `set` refuses those immediately, naming the
field. Discovering an unserializable value inside `on_shutdown` is the worst
possible moment to discover it. The `memory` tier skips the check entirely,
which is what lets an aggro table hold live object references.

**A scope is a document, and a document has a 64 KB ceiling.** The cache keeps a
running estimate, warns at 48 KB, and refuses the write that would cross the
limit — naming the tenant rather than raising during a flush.

**A write that fails is retried, and a write that keeps failing is
quarantined.** After three failures a scope is dropped from scheduling and
reported in `stats()`, but kept in memory so the game carries on.

**Memory is the authority between flushes.** Despite the name, dropping a dirty
scope loses data. There is deliberately no `clear_all`.

## Cooldowns

`DAEMON.cooldown` is a small daemon on top of the cache, and the worked example
of the tier rule.

```lua
if not DAEMON.cooldown.ready(player.char_id, "manasteel") then
    local hours = math.ceil(DAEMON.cooldown.remaining(player.char_id, "manasteel") / 3600)
    player:send("The vault wards still recognise you. Try again in " .. hours .. "h.")
    return
end
-- ... give the reward ...
DAEMON.cooldown.mark(player.char_id, "manasteel", 24 * 3600)
```

| Function | |
|---|---|
| `mark(char_id, what, seconds, opts)` | `opts.durable` overrides the tier choice |
| `remaining(char_id, what)` | seconds, 0 means ready |
| `ready(char_id, what)` | |
| `expires_at(char_id, what)` | the raw timestamp |
| `clear` / `clear_all` / `list` | |

It stores **expiry, not last-claimed**, so a check never needs to know the
duration — and changing a cooldown from 24 hours to 12 does not retroactively
rewrite everyone's remaining time into something wrong.

The tier is chosen by duration: at least `game.cooldown_durable_seconds`
(default 60) goes to disk, anything shorter lives in memory.

> [!NOTE]
> **Why a threshold rather than a required flag.** Explicit usually beats
> implicit, but here the two mistakes are not equally loud. Forget the flag on a
> two-second ability cooldown and you write to disk on every use by every
> player — a slow leak that surfaces months later as "the game feels sluggish"
> with nothing obvious to blame. Forget it on a daily reward and the gate resets
> on the next restart, which a player reports the same day. The default belongs
> on the side of the loud failure. The rule to remember:
>
> **Under a minute it is a game mechanic. Over a minute it is a promise to the
> player.**

## Configuration

```toml
[game]
cache_flush_seconds      = 5    # how often dirty scopes are considered
cache_flush_budget       = 32   # scopes per tick, bounding the hitch
cache_evict_seconds      = 900  # idle eviction for unowned scopes
cooldown_durable_seconds = 60   # the tier threshold
```

`cache_flush_seconds` is the scheduler's granularity, not the flush interval —
each namespace has its own. Scopes are spread across their interval by a
deterministic jitter, drained oldest-dirty-first, and bounded per tick: 200 dirty
scopes at once would be about 20 ms of game thread, which is a visible hitch, so
the budget spreads it over a few ticks instead.

## Relationship to CHARACTER_D

[CHARACTER_D](./character-data.md) is already a write-behind cache — an
in-memory `Player` flushed by the autosave ticker, on disconnect and on
shutdown. This daemon sits beside it rather than replacing it, because
CHARACTER_D caches live objects with a `to_save()` projection rather than plain
values, and folding them together would mean a bug in either was a bug in both.
They should converge eventually. They have not yet.

## What it will not do

- **No queries.** `scopes(ns)` lists what is in memory, not what is stored. A
  write-behind collection cannot answer a live question by definition.
- **No cross-scope transactions.** A flush is one document at a time.
- **No durability on a crash.** That is the contract, not an oversight: you may
  lose up to `flush_seconds` if the process dies. If that is unacceptable for a
  particular thing, it is a write-through thing.
- **No sharing between processes.** That is what Redis is for, and this codebase
  does not need it.
