# Open Findings — Durability & State Tiering

Three connected items, discovered while designing a per-player cooldown. They
are ordered by dependency, not severity: item 1 was a live bug and a
prerequisite for item 3.

**All three are now closed.** The work is recorded in
`docs/src/changelog.md`, and the design pages are
`docs/src/lua-api/state-cache.md`, `traits.md`, `effects.md` and `combat.md`.

---

## 1. ✅ Clean shutdown never saves anything

**Status:** fixed — 2026-08-02. Recorded in `docs/src/changelog.md`.

What landed, in the order the analysis called for — **dispatch → break → wait**,
not break → drop:

- `LuaCommand::Shutdown` dispatches `on_shutdown` under the engine's system
  identity (as `TimerFired` does) before breaking the loop.
- `Driver::run` calls `ScriptEngine::shutdown_within` on the Ctrl+C path
  instead of leaving it to `Drop`. The Lua thread signals a channel when its
  loop ends, which is what makes the wait bounded — a `JoinHandle` cannot be.
- The bound is `game.shutdown_timeout_seconds` (default 30). On expiry the
  driver logs an error naming the config key and exits anyway.
- `Drop for ScriptEngine` still sends, as a backstop for teardown paths that
  never asked politely (failed startup, a test dropping its VM, a panic).
- `mudlib/init.lua` defines `on_shutdown`, running the existing autosave task
  through `pcall`; declared in `types/oxigeon.lua` and documented in
  `docs/src/lua-api/events.md`.

`tests/clean_shutdown.rs` covers the dispatch, the identity (both the efun and
directory gates), the bound holding against a hook that does not return, and —
through `boot_real_mudlib` — a `pagesize` change reaching the database only
because the shutdown saved it.

Items 2 and 3 followed on top of it; the analysis in each is kept below as the
record of why they were built the way they were.

---

## 2. ✅ `COOLDOWN_D` — per-player gates that outlive an area reset

**Status:** built — 2026-08-02

### The problem

A repeatable room resets on its own clock (`game.area_reset_seconds`, default
900) so its resources come back — that is intended and correct. But a *player*
gate ("once per 24 hours") has a different lifetime and must not inherit the
room's.

`game/areas/wizard_workshop/rooms.lua` currently stores per-character progress
on the **room's** object state:

```lua
get_object_state(VAULT_ID, "manasteel_taken_" .. tostring(player.char_id))
```

`world_d` clears room object state on every area reset, so the gate is
effectively "once per 15 minutes". Not because the reset is wrong — because
per-character state is living on a room.

### Four lifetimes, four homes

| What | Lifetime | Home |
|---|---|---|
| Is the node depleted right now? | Until area reset | `set_object_state` — already correct |
| Has this character *ever* done it? | Forever | `player.quest_flags` — already in `SAVE_FIELDS` |
| When may this character do it *again*? | Until a wall-clock deadline | Document store (`db_*`) |
| Transient server-lifetime state | Until restart | `set_persistent` — exists, currently unused |

### The daemon

Store **expiry**, not last-claimed, so the check never needs to know the
duration:

```lua
-- mudlib/daemons/cooldown_d.lua
local M = {}

local function key(char_id) return "char:" .. tostring(char_id) end

--- Seconds until `what` is available again. 0 means ready now.
function M.remaining(char_id, what)
    local rec = db_get("cooldowns", key(char_id))
    local expires = rec and rec.data[what]
    if not expires then return 0 end
    return math.max(0, expires - os_time())
end

function M.ready(char_id, what)
    return M.remaining(char_id, what) <= 0
end

function M.mark(char_id, what, seconds)
    local expires = os_time() + seconds
    -- db_update is a recursive merge, so it sets this one key and leaves the
    -- player's other cooldowns alone. It returns false when the document does
    -- not exist yet — the first cooldown this character has ever had.
    if not db_update("cooldowns", key(char_id), { [what] = expires }) then
        db_put("cooldowns", key(char_id), { [what] = expires })
    end
end

return M
```

Call site becomes two independent questions:

```lua
if not DAEMON.cooldown.ready(player.char_id, "manasteel") then
    local hours = math.ceil(DAEMON.cooldown.remaining(player.char_id, "manasteel") / 3600)
    player:send("The vault wards still recognise you. Try again in " .. hours .. "h.")
    return
end
-- ... give the reward ...
DAEMON.cooldown.mark(player.char_id, "manasteel", 24 * 3600)
```

### Why the document store and not the alternatives

- **`player.custom.cooldowns`** would work and needs no daemon — `custom` is
  already in `SAVE_FIELDS`. But character data is only flushed on autosave or
  disconnect, so a crash between claiming and saving hands the reward back:
  crash-farmable. It also cannot answer "who is on cooldown right now". Fine
  for a once-ever flag; weak for a timed one. **Note this weakness is item 1's
  bug, not a property of the approach** — it gets much better once shutdown
  flushes.
- **`set_persistent`** dies on restart, so a reboot returns everyone's daily.

### Write cost is a non-issue here

One write per player per day. At 1,000 players that is 1,000 × ~101 µs ≈ 0.1
seconds of writes **per day**. See item 3 for where writes *do* matter.

### Scope note

Migrating the vault is a change to game content and is the author's call. The
daemon can land without touching `wizard_workshop`.

---

## 3. ✅ A write-behind tier is missing (and Redis is not the answer)

**Status:** built — 2026-08-02. The block cleared when item 1 landed.

### The concern

Evennia's Attribute system persists every attribute change to the database.
Every stat change, every combat hit, every mob swing is a write, and it becomes
the bottleneck. Oxigeon's document store has the same shape and could develop
the same problem if used for high-frequency state.

### Measured (release build, through the real VM)

Net of the 7.4 µs harness round trip:

| Tier | Net cost | vs in-memory |
|---|---|---|
| `set_persistent` (in-memory Lua table) | **2.7 µs** | 1× |
| `set_object_state` (in-memory Lua table) | **3.6 µs** | 1.3× |
| `db_get` (SQLite, WAL) | 25 µs | 9× |
| `db_put` small document | **101 µs** | 37× |
| `db_update` one field | 133 µs | 49× |

A document write costs ~37× an in-memory write and is **synchronous on the game
thread**. One `db_put` roughly doubles the cost of a `look` (~75 µs). At ~10
attribute writes per actor per combat round with 70 actors, that is ~35 ms/sec
of game thread — survivable, but the cliff is visible, and the ceiling is about
10,000 writes/sec with the thread doing nothing else.

*(These numbers came from a throwaway measurement. If this item is taken up,
add them as a `cargo bench` group so they stay re-measurable — the same
discipline `benches/dispatch.rs` applies to the JIT numbers.)*

### Why not Redis

- **It is not faster.** Redis over loopback is ~30–100 µs per round trip — the
  same order as our own `db_put` at 101 µs, and 10–30× slower than the 2.7 µs
  in-process path.
- **It does not improve durability.** Redis's default persistence is RDB
  snapshots every N seconds — *exactly* the guarantee an in-process write-behind
  cache gives. Switching to AOF with fsync puts you back at disk speed, which is
  what you were escaping.
- **It costs infrastructure.** A second process to install, run, authenticate
  and monitor, on servers that may be someone's spare VPS.

Redis earns its keep when state must be shared across processes or machines. A
single-threaded MUD driver is not that.

### What is actually missing

Not a faster store — a **write-behind** one. The codebase already has the
pattern: `CHARACTER_D` is an in-memory cache flushed by the autosave ticker.
What is missing is a general version.

The win is not a faster write. It is **collapsing N writes-per-change into one
write-per-interval per scope**: 700 in-memory changes a round (~1.9 ms) plus one
flush per character per 30 s.

### Design sketch — pure Lua, no Rust

`set_persistent` is an unused, un-namespaced in-memory KV that survives hot
reload. With `db_*` and the ticker that is the whole substrate:

- Reads: memory first, fall back to `db_get` and populate.
- Writes: memory only, plus mark the scope dirty.
- Flush: one `db_put` per dirty **scope** (not per key) on a ticker, on session
  disconnect for that player's scope, and on `on_shutdown`.
- Documented contract: **you may lose up to `flush_seconds` of writes on a hard
  crash.**

### Why it is blocked

A cache that is never flushed on shutdown is strictly worse than writing
through. Item 1 must land first — otherwise every clean restart silently
discards the cache.

### The most important part is a rule, not a mechanism

Evennia's mistake was giving *every* attribute the same durability. Most
high-frequency state should never be persisted at all:

> Choose the tier by **how much you would mind losing it**, not by convenience.

- Combat HP mid-fight, aggro tables, positions, buff timers → **never persist**.
  If the server reboots, the fight is over. `set_object_state` at 3.6 µs is the
  right home.
- Character progression → write-behind.
- A 24-hour cooldown → straight to the document store; one write per player per
  day is not a problem worth engineering around.

This rule belongs in `CLAUDE.md` and `docs/src/lua-api/document-store.md`
whether or not the write-behind tier is ever built.


---

## What was built

`DAEMON.cooldown` stores expiry rather than last-claimed, across two tiers
chosen by duration: at least `game.cooldown_durable_seconds` (default 60) is
written through immediately, anything shorter lives in memory. The threshold
rather than a mandatory flag because the two mistakes are not equally loud —
forgetting it on a two-second ability cooldown is a slow leak nobody diagnoses,
forgetting it on a daily reward is reported the same day.

`DAEMON.cache` is the general write-behind tier, with three tiers and the rule
in `CLAUDE.md`. Its first tenants are effects and cooldowns; traits went to
`player.stats` instead, since CHARACTER_D is already a write-behind cache with
the right flush points.

The numbers item 3 asked to be made re-measurable are now `cargo bench --bench
writes`, and they came out close to the throwaway ones:

| | Throwaway | Measured |
|---|---|---|
| in-memory write | 2.7 µs | **0.8 µs** (`cache.set`) |
| `db_get` | 25 µs | 30 µs |
| `db_put` small | 101 µs | 84 µs |
| `db_update` one field | 133 µs | 116 µs |

And the payoff, for N changes to one player's state:

| N | write-through | write-behind |
|---|---|---|
| 10 | 1.20 ms | **0.15 ms** |
| 100 | 23.3 ms | **0.36 ms** |
| 1000 | 1077 ms | **2.3 ms** |

The bench's control counts document writes rather than comparing times, because
a timing alone cannot tell "write-behind is fast" from "write-behind is not
writing". It earned its keep immediately: it caught the byte estimate counting a
key's name on every write instead of only the first, which would eventually have
made a long-lived scope refuse every write with a size complaint that was not
true.

### Still open from the original sketch

- The manasteel vault in `game/areas/wizard_workshop/rooms.lua:310` still stores
  per-character progress on the room's object state. `DAEMON.cooldown` is what it
  should use; migrating it is a change to game content and remains the author's
  call.
- `character_d` was not folded into `cache_d`. They are the same mechanism
  implemented twice and should converge, but CHARACTER_D caches live `Player`
  objects with a `to_save()` projection rather than plain values, and merging
  them now would mean a bug in either was a bug in both.
