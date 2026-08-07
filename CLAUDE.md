# Oxigeon — Project Rules

## Architecture

Oxigeon is a MUD (Multi-User Dungeon) game driver. Rust handles the infrastructure (networking, database, Lua VM), Lua handles all game logic.

**Two Lua layers:**
- `mudlib/` — Core system layer (login, command dispatch, event hooks, base daemons)
- `game/` — Game content layer (rooms, areas, game-specific daemons)

**Daemons** are singleton services registered in the global `DAEMON` table. Convention: files are named `*_d.lua` (e.g. `room_d.lua`, `character_d.lua`).

## Error Handling — Mandatory Practices

### 1. Never Silently Swallow Errors

Every operation that can fail must either:
- Be wrapped in `pcall()` with the error logged, **or**
- Return a status value that the caller checks

Silent failures (empty `if not x then return end` with no logging) are **not acceptable** for operations involving data persistence, world state, or player actions.

### 2. Use journal_d for Structured Error Logging

`log(level, msg)` writes to the server console (Rust tracing). `DAEMON.journal` writes to the **structured journal** (persisted, searchable, queryable by admins).

**Always log critical failures to both:**
```lua
local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        DAEMON.journal.error(message)
    end
end
```

Use `log()` alone for debug/trace level messages. Use both for `warn` and `error` level messages that admins need to see.

### 3. Protect Cleanup Chains

When multiple cleanup steps must all run (e.g. on disconnect: save data → remove from world → clear login state), wrap **each step** in its own `pcall` so a failure in one doesn't prevent the others:

```lua
-- CORRECT: each step is independent
if DAEMON.character then
    local ok, err = pcall(DAEMON.character.unload, char_id)
    if not ok then log_error("unload failed: " .. tostring(err)) end
end
if DAEMON.world then
    local ok, err = pcall(DAEMON.world.remove_character, char_id)
    if not ok then log_error("remove failed: " .. tostring(err)) end
end

-- WRONG: second step skipped if first throws
DAEMON.character.unload(char_id)
DAEMON.world.remove_character(char_id)
```

### 4. Protect Init Loading

When loading daemons and areas in init files, wrap each `require()` in `pcall` so a broken file doesn't crash the entire layer:

```lua
local ok, err = pcall(function() DAEMON.world = require('daemons.world_d') end)
if not ok then log("error", "Failed to load world_d: " .. tostring(err)) end
```

### 5. Validate Inputs in Daemons

Daemon functions that receive IDs or tables should validate before acting:
- Log a warning if called with unexpected arguments
- Return a clear failure value (false, nil) rather than crashing

## journal_d vs audit_d

These are two separate logging daemons with distinct purposes:

### journal_d (`DAEMON.journal`)
**Purpose:** General-purpose structured server log.

Use journal_d for operational events — things a server operator or developer needs to see:
- Daemon load/unload events
- Errors and warnings during gameplay (save failures, missing rooms, bad lfun returns)
- Module hot-reloads
- Performance or state warnings

```lua
DAEMON.journal.error("CHARACTER_D: Failed to save data for char 42")
DAEMON.journal.info("Module reloaded: areas.wizard_workshop")
```

Entries are written to `logs/journal.log` via the Rust `GameLogger`. Searchable by level, readable via `DAEMON.journal.recent(n, level)`.

### audit_d (`DAEMON.audit`)
**Purpose:** Security and compliance audit trail for player/admin actions.

Use audit_d for tracking **who did what** — things a moderation team needs to review:
- Privileged command executions (spawn, ban, grant)
- Permission denials
- Admin actions
- Any command on the audit watch list

```lua
DAEMON.audit.log("cmd.ban", true, "banned user xyz")
DAEMON.audit.after_command("spawn", session_id, args_str, ok, err)
```

Entries are written to `logs/audit.log` via the Rust `GameLogger`. Commands can be added to the watch list with `DAEMON.audit.watch("spawn", "all")`.

**Rule of thumb:** If the question is "what went wrong?" → journal_d. If the question is "who did this?" → audit_d.

## State: Choose the Tier by What You'd Mind Losing

Evennia persists every attribute change straight to the database, and that is
where its performance goes. A `db_put` here costs ~84 µs against ~0.8 µs for an
in-memory write, synchronously on the game thread. The fix is not a faster
store — it is writing less often.

> **Choose the tier by how much you would mind losing it, not by convenience.**

| Tier | Mechanism | You lose | For |
|---|---|---|---|
| memory | `DAEMON.cache` memory namespace | everything on restart | combat state, aggro, targets, sub-minute cooldowns, short buffs |
| write-behind | `DAEMON.cache` write-behind namespace | up to `flush_seconds`, **only on a crash** | effects, quest counters, statistics |
| character | `player.stats` + `SAVE_FIELDS` via CHARACTER_D | same as above | trait bases, gauge currents, progression |
| write-through | `DAEMON.cache` write-through namespace | nothing | daily gates, admin actions, entitlements |

### Which home does this state get?

Three questions, in order:

1. **Would you print it on a character sheet?** → `player.stats` / `SAVE_FIELDS`.
2. **Does another subsystem own its lifecycle** — it ticks, expires, accumulates?
   → `DAEMON.cache`, one namespace per subsystem.
3. **How much would you mind losing it?** → the table above.

Two corollaries worth stating outright:

- **Do not put per-character state on a room.** `set_object_state` is wiped by
  area resets, which is why a "once per 24 hours" gate stored on a room was
  really "once per 15 minutes". Use `DAEMON.cooldown`.
- **Stop growing `player.custom`.** It is where state goes when nobody decided,
  and it is how a 64 KB character blob gets rewritten on every autosave.

See `docs/src/lua-api/state-cache.md`.

## Permission Strings Have One Shape

Every gated command declares `cmd.<its own verb>`, or `cmd.<verb>.<capability>`
for a sub-power. Efuns are `efun.<name>`, spelled exactly as the Lua global.
Directory rules are `dir.<op>.<root>.<top>` and match the `[directories]` key,
which names its root because the jail has two. Anything that is none of those is
`<thing>.<capability>` — `alert.receive`, `board.moderate`, `channel.staff`.

This is enforced, not suggested, because the alternative already happened:
`setup_roles.lua` granted `cmd.olc` while the command required `olc`, and
`efun.write_file` while `permissions.toml` said `efun.file.write`. Not one of the
builder role's grants matched anything, so the role was decorative and only
account 1's `is_admin` bypass could build. Both halves looked right in isolation.

Two tests, asking different questions:

- `tests/command_layout.rs` — the *shape*: `cmd.<own verb>`. The "own verb" half
  is what stopped `dig` asking for `cmd.olc`, which it did, so `dig` could not be
  granted separately.
- `tests/demo_world/roles.rs` — that somebody can actually *be given* it: every
  permission a command names is granted by some role. It lives with the game
  layer because which roles exist is a game decision.

A command gate and an efun gate are separate and both apply. `cmd.verify` lets
you type the verb; `efun.verify_file` lets mudlib code call the efun.

## The File Jail Has Two Roots

`mudlib/` and `game/`. A path may name one — `write_file("game:areas/crypt/rooms.lua", …)`.
Unprefixed, a **read** searches game-then-mudlib the way `require` does, and a
**write** stays in the mudlib.

- **A file a daemon owns must name its root.** Writes default to the mudlib and
  reads prefer the game layer, so a stray `game/logs/audit_watch.json` would
  shadow the one `audit_d` writes, permanently and silently.
- **The file efuns return failure; they do not raise it.** `pcall(write_file, …)`
  gives `ok = true, err = false` — the call succeeded and the refusal is in a
  return value the `pcall` discarded. `codegen_d` was written that way and
  reported success for refused writes for as long as it existed. Call them
  directly and read both values.

See `docs/src/lua-api/file-access.md`.

## Authored Content Has One Description

`mudlib/schema/{room,item,mob}.lua` say what an authorable thing is: every field
with its type, default, editability and help. Four consumers read it — codegen
emits from it, `olc set` validates through it, `verify` checks against it,
`objdump -s` annotates with it — and none of them holds its own copy.

- **Component fields live in the component file**, beside the `from_data` that
  reads them, discovered the way `is`/`order` already are. Never a central list;
  see the trait rules above for why.
- **The flat authoring form is the interchange format**, not the built object.
  `Weapon{…}` → `Item:new` + `from_data` is one-way, so an Item cannot be written
  back to a file. OLC reads and writes the *input* to that.
- **`schema.set` is the only string-to-value converter.** A second one disagrees
  with the first eventually, and the disagreement surfaces as a field that
  round-trips wrong six months later rather than as an error.
- **A field no schema names is kept and reported, never dropped.** Silently
  losing a field nobody declared is indistinguishable from a typo.
- **`lfun = true` is a flag, not a type.** A room's `description` is prose
  whether written out or computed; a function is legal content and makes the
  field `lossy`. `type = "lfun"` is different — a field OLC may never set.

OLC regenerates `rooms.lua`/`items.lua`/`mobs.lua` wholesale. That is only safe
because `custom.lua` — hand-written, never read or written by OLC — holds
everything that cannot be expressed as data. See `docs/src/lua-api/olc.md`.

## Negotiated Client State Lives on the Session

`TelnetConnection.capabilities` is what negotiation writes; `Session.capabilities`
is what the mudlib reads through `get_session`. They are two structs on two
objects, and `driver.rs::publish_capabilities` is the only thing joining them —
call it after every negotiation and subnegotiation.

Nothing did, for the life of the project. `Session.capabilities` sat at
`Default::default()`, so `gmcp_supported` was false for every session ever, every
`gmcp_d` sender returned at its first guard, and **no GMCP reached any client** —
while `Core.Hello`, which the driver sends directly, made the link look healthy.
`window_width` was nil at the same time, so output was wrapped to a default
regardless of the terminal.

The lesson generalises: **a capability discovered by the network layer is not
state until something copies it to where the game looks.**

## A Prototype Is Resolved Before Anything Sees It

`schema defaults ← prototype chain ← the area's data file ← custom.lua`. Each
layer is more specific and more hand-written than the last, and `areaload`
flattens the first two into the third *before* `patch.apply` and before anything
is registered. A registered template is therefore what it has always been, which
is why `mob_d`, `item_d`, `combat_d` and every spawn path needed no changes.

- **Prototype-resolve, then custom-patch.** The other order leaves a strike the
  patch does not mention sitting in the datum as an uninterpreted `"@none"`, and
  runs the patch merge before `components` is resolved — so a `map` field gets
  replaced where it should have merged, silently.
- **The draft OLC holds is the override set**, seeded from the file and never
  from the live object. Seeded the other way the first `olc save` writes every
  inherited value out and the record stops tracking its parent, with an enormous
  diff nobody reads. `olc show` prints effective values; those are different
  things on purpose.
- **Never subtract, never infer intent from value equality.** A builder who sets
  a field equal to the inherited one means "this is mine now". `olc thin` is the
  only thing that removes it, and a human asks for it.
- **`verify` asks two questions.** Duplicate ids, `lossy`, `unknown` and the
  chain are properties of the *file*; validation, exits, references and traits
  are properties of the *world the next reload builds*. A child missing a field
  it inherits is not an error, and an inherited function does not belong in
  `custom.lua` — it is already in a hand-written file.
- Arrays replace rather than union, and `"@none"` is the one delete sentinel.
  `custom.lua` still has none: there the generated file is the whole truth, so
  "take it out in OLC" is always available, and here it is not.

See `docs/src/lua-api/prototypes.md`.

## An Ability Is Five Existing Systems Arranged

`ability_d` owns no state machinery. Costs are gauges through `trait_d`; damage
and healing go through `Mobile:take_damage`/`heal`, so armour and the effect
pipeline meet an ability exactly as they meet a sword; gates are `cooldown_d`; a
channel **is** an `effect_d` instance; a cast in flight is one `ticker_d` timer
and one memory-tier cache key.

- **Resolve the target before spending anything.** A mistyped name must not cost
  mana. It is the first thing a growing flow loses.
- **Cost at the start of a cast, the ability's own cooldown at completion, the
  global cooldown at the start.** A cast you can begin and abort for free is a
  free oracle; a cooldown rate-limits the outcome, not the attempt; the GCD is
  the one gate that rate-limits inputs. Nothing is refunded on an interrupt —
  that is policy, and `on_interrupt` is three lines away.
- **A cost may only name a gauge**, refused at define time. The mirror of
  `effect_d` refusing a modifier aimed at one.
- **Definitions are code and may hold functions; grants and casts are data.**
  Anything reaching `DAEMON.cache` is ids, numbers and timestamps — an entity in
  an effect instance's `caster` fails `lua_to_json` and the apply is refused with
  no reason, because the refusal comes from the cache write.
- **No formula strings.** A number is a number, a `{min,max[,scale]}` table, or a
  function. An expression parser is a second string-to-value converter, and
  `load()` on author text is a sandbox hole.
- Rank folds by `math.max` across the trait and every grant, so a sword that
  grants an ability can raise a floor and never lowers a ceiling.

See `docs/src/lua-api/abilities.md`.

## Roundtime Belongs to a Track, Never to a Command

`look`, `say` and `who` work while you are recovering, and not by an exemption
list: nothing in command dispatch reads a track, so they never enter the code
path. A track is a named lane of intent — combat first, crafting and gathering
later — with its own queue, its own gate and its own idea of a round.

- **Recovery is not occupation.** `ability_d`'s `cast_time` owns "you are busy";
  a track's roundtime owns "this lane may not act again yet". Because they are
  different they need no arbitration — a tick skips an entity that is casting,
  and that is the whole interaction.
- **Roundtime is a `cooldown_d` entry under `rt.<track>`.** Always under a
  minute, so the existing threshold rule already puts it in memory and forgets
  it on restart, `evict_owner` already cleans it up, and `cooldown` already
  answers "why can't I swing".
- **Only roundtime enqueues.** A cooldown, a cost, a requirement and a mistyped
  target all still refuse. The enqueue sits below target resolution, so a typo
  still costs nothing.
- **`{ rounds = n }` is multiplicative and `scale` is additive**, so it is a real
  branch and never a desugar. A round is a derived trait the *game* defines; an
  absent one falls back and warns, because a silent zero is a wrong answer.

## Today's Combat Formula Is the New Default, Not a Second Path

`clamp(60 + (a_dex - d_dex) * 3, 5, 95)` is `clamp(BASE + (A - D) * STEP, FLOOR,
CEIL)` with the shipped configuration and both ratings falling back to
dexterity. There is no `if legacy then` anywhere, and the prettier ratio form was
rejected because it silently rebalances a shipped game.

- **The margin is out of 100, not out of the threshold.** A 95% attack rolling 3
  is a decisive blow; a 10% attack rolling 3 is a graze. Normalising inverts
  both. What a degree is *worth* is game content — the mudlib ships one band at
  power 1.0 so damage is unchanged until a game says otherwise.
- **A defence channel is a registry entry, and presence is decided by storage**:
  an entity has one iff it holds that channel's trait. The allocation is
  ordinary attributes, so effects modify it for free and no new store exists.
  Holding none gives one implicit dodge worth the whole pool, which is the
  no-configuration path.
- **A body layout is optional by absence.** No layout means no location, **no
  roll consumed choosing one**, and `ev.hit_slot` nil — so the per-slot armour
  guard is skipped, which is every call the game makes today.

## A Line Is Authored Once and Read Per Viewer

`"$Actor $actor.v(draw) a line of fire at $target."` renders three ways. `$` was
chosen so it *subsumes* the substitution `ability_d` already did rather than
competing with `{colour}` — `lib/color.lua` matches `{(.-)}` over the whole
string, so a role and a colour tag would be indistinguishable in the source.

- **An unknown token survives verbatim.** "You strike $victim" is a typo somebody
  can see; "You strike " is a bug they will stare at. A table nothing can name
  survives too, which is the general form of the `$target` bug that shipped.
- **English asks the agreement question twice.** `plural` is agreement when the
  *pronoun* is the subject; `collective` is agreement when the *name* is. A
  they/them person takes "they swing" and "Ash swings", so one flag cannot do it.
- **An entity with no gender is `it`, unless it is a player, and then `neutral`.**
  Nothing sets `gender` at character creation, so that is the default path.
- Which name a creature gets in a sentence is `game.display_name_prefers`, not a
  rule: prose wants `short`, hack-and-slash wants `name`.

## Lua Coding Conventions

- Use `\r\n` for player-facing text sent via `send()`
- Use `[[ ]]` for multi-line description strings
- Daemon files: `*_d.lua` naming convention
- Room IDs: `area_name.room_name` dotted notation
- All MUD objects inherit from `Object` (`game/lib/object.lua`)
- Use data-oriented `from_data()` / `load_area()` for authored rooms; builder for dynamic generation
- Properties support the lfun pattern — strings or functions returning strings, resolved via `Object.resolve()`
- Object state uses `set_object_state(id, key, value)` / `get_object_state(id, key)` efuns (or `Object:set_state(key, value)`)
- Timers go through `DAEMON.ticker` — never raw `schedule_timer` unless building a daemon
- Commands are auto-loaded from paths in `game.command_paths` config
- Character state goes through CHARACTER_D (in-memory cache), not raw efuns
- Traits are read through `entity:trait(id)` or `DAEMON.trait.value` — `entity.stats[id]`
  is the *stored* value, which for a buffed or derived trait is the wrong answer
- A trait is any numeric datum on any entity, not a character statistic. Presence
  is decided by storage, never declared: an entity has a stored-kind trait when
  there is a number for it, a derived one when everything it reads is present.
  Never add an `applies_to` list — that is the thing that rots
- Effects never modify a gauge or a counter; they modify attributes and derived
  traits. To raise a gauge's ceiling, modify the trait that is its `max`
- A value written to `DAEMON.cache` must survive `lua_to_json`: no functions, no
  mixed list/map tables, no NaN. The memory tier is the exception

## Testing

Run `cargo test` before committing. All tests must pass. Current count: 1196 on the default `lua55`, green on both it and `--no-default-features --features luajit`.

Do not pin a number that is really a property of the daemon roster. A logpoint
test asserted `#ids == 2` on `ticker_d.list()`, which meant adding a heartbeat to
any daemon failed a test about whether the debugger could see a stack frame. Same
for `tostring` of a float: 5.5 prints `1.0` and LuaJIT prints `1`, so a printed
number in an assertion fails one of the two builds for no reason anyone will
enjoy finding.

`cargo test` does not build `oxigeon-compute` — it is a separate workspace member that links LuaJIT unconditionally, and cargo unifies features across one invocation. The harness builds it on demand into `target/compute-worker/`.

### A mudlib test must not depend on this game

`tests/demo_world/` holds everything asserting the shipped content and is
deleted along with `game/`. Everything else goes in `tests/` and, if it needs a
world at all, uses `RealVm::boot_with_fixture_world` rather than Thornhollow.
The check:

```bash
# `git stash push <path>` only reverts changes — it does not remove the
# directory, so it never tested anything. Move them out of the tree instead.
mkdir ../away && mv game ../away/ && mv tests/demo_world ../away/
cargo test --no-fail-fast
mv ../away/game . && mv ../away/demo_world tests/ && rmdir ../away
```

See `docs/src/testing.md`.

### Test the real VM, not a helper beside it

Two security controls once shipped broken because their tests exercised a
helper in isolation while production took a different path — the sandbox was
never applied to the VM the engine builds, and the instruction limit was parsed
and never read. Both suites were green the whole time.

`tests/common/mod.rs` boots a real `ScriptEngine` and runs probe Lua *inside
it*. Any test of a security boundary — the sandbox, the instruction budget,
permission gating, the auth path — goes through that harness, so it asks what
game code can actually do rather than what a function does when called
directly. See `tests/sandbox_reality_check.rs` for the shape.
