# Demo Game — Feature Coverage Matrix & Build Order

## Context

Oxigeon's driver and mudlib are substantial. The game layer is not.

| Layer | Contents |
|---|---|
| `mudlib/` | 23 daemons, 49 commands, 18 libs, 1 component, 2 tasks — 94 files |
| `game/` | 6 files, 989 lines: one area (6 rooms), 7 items, 2 mob templates, 10 traits, 6 effects |

The driver has far more capability than any content exercises. Concretely: the
entire `Weapon`/`Armor` half of the object model has **zero instances** anywhere;
`Mobile:get_dialogue`, `get_skill`/`set_skill` and every `Player:*_quest_flag`
method have **no callers**; `lib/checks.lua`'s predicate library is unused;
`aggressive` is on every mob template and nothing reads it; `on_gmcp` only logs.

Worse, some capability has no game-side path at all. **There are no ground
items.** No `get`, `drop`, `wear`, `wield`, `give`, `put`, `use`. Items exist as
templates in a registry and as entries in `player.inventory`; nothing puts one on
a floor. Combat loot "goes straight to the killer" because there is nowhere else
for it to go.

The same is true on the Rust side. Of 74 registered efuns, a large block has
**no caller in `mudlib/` or `game/` at all**: the entire RBAC management family
(`create_role`, `assign_role`, `grant_permission`, `refresh_permissions`, … —
only `has_permission` is used, so roles must be provisioned out-of-band), nine
of the twelve `db_*` efuns (only `db_get`/`db_put`/`db_delete`, all from
`cache_d`), plus `compute`, `this_session`, `get_account`, `append_file`,
`delete_file`, `file_exists`, `os_clock`. A 906-line document store's entire
query and atomic-merge half has no game-side consumer.

A demo game is the instrument that finds out which capabilities work end-to-end,
which are half-wired, and which are missing. This plan is that instrument,
specified as a coverage matrix (every feature → the content that proves it) plus
a phased build order.

**Decisions taken:** build a new multi-area world and keep `wizard_workshop` in
place as the existing regression fixture; build the missing mudlib verbs the
content depends on, sized to what the showcase actually exercises.

---

## Part 1 — Gaps that block content (build these first)

These are not "nice to have". Each one is a wall the demo hits immediately.

| # | Gap | Evidence | What to build |
|---|---|---|---|
| G1 | **No ground items** | no `ground`/`floor` reference anywhere in `mudlib/`; `item_d` is `register/get/resolve/find_by_name/all` only | Item *instances* with a location. Extend `item_d` with `spawn(template_id, location)`, `move(inst, location)`, `in_room(room_id)`, `find_in_room`. Follow `mob_d`'s instance model exactly — it already solves this shape (`mudlib/daemons/mob_d.lua`). |
| G2 | **No `get`/`drop`** | `mudlib/cmds/` has `inventory` and `drink`, nothing else in the `items` category | `mudlib/cmds/get.lua`, `drop.lua`, `put.lua`, `give.lua`, `use.lua`, `examine.lua`. Fire the documented events `item.picked_up` / `item.dropped` / `item.used` and the `Item.on_pickup`/`on_drop`/`on_use` hooks — all already declared, none called. |
| G3 | **No equipment verbs** | `Mobile.equipment` is a `slot → item_id` map nothing writes | `wear.lua`, `wield.lua`, `remove.lua`, plus an `equipment` command. Must call `Requires.met(item, entity)` (`mudlib/lib/requires.lua`) and route bonuses through `DAEMON.effect.set_source_effects(entity, "equip:"..slot, ...)` with `persist = false` — the documented pattern in `docs/src/lua-api/effects.md`. This is what makes the `armour` component's `stat_bonus` and `resist` real. |
| G4 | **Armor never mitigates** | `combat_d.round()` runs the `damage_taken` pipeline, but nothing registers an armour handler | An `equip:` effect per worn piece contributing a `reduce`-phase handler and a `resist` lookup by `damage_type`. Proves the phase ordering the effects doc argues for. |
| G5 | **No containers** | not documented, not implemented | `Item` subclass or component with `capacity`, `contents`, `closed`/`locked`. Needed for `put`, a bank vault, and a corpse. |
| G6 | **`help` is a stub** | `mudlib/cmds/help.lua:1` — hardcoded list of ~20 of 49 commands; advertises a `stat` command that does not exist | Generate from `M.category` + `M.summary`, which every command already sets. Add the missing `stat.lua`. The file's own comment (line 14) asks for this. |
| G7 | **`death_d` hardcodes a game room** | `respawn_room = "wizard_workshop.entrance"` in the mudlib layer | Move to `game.respawn_room` config with the current value as fallback. |
| G8 | **Aggro is dead data** | `docs/src/lua-api/combat.md`: "`aggressive` is on the template and nothing reads it yet" | A game-layer aggro handler on `room.entered`. Belongs in `game/daemons/`, not the driver — combat.md is explicit that this is a game's decision. |
| G9 | **No shops** | only `Item.value` and `Player:award_gold`/`spend_gold` | `mudlib/daemons/shop_d.lua` + `list`/`buy`/`sell`. Restock on a `task_d` task. |
| G10 | **No quest system** | `quest_flags` and `quest:` effect source exist; nothing else | `game/daemons/quest_d.lua` + `quests`/`quest` commands. Game layer — quest design is content. |
| G11 | **No NPC conversation** | `Mobile.dialogue` and `get_dialogue` unused | `talk.lua` / `ask <npc> about <topic>`. |
| G12 | **No emote/socials** | — | `emote.lua` (`:`) at minimum. |
| G13 | **`/areas` is world-writable** | the `"/areas" = { write = "dir.write.areas" }` rule is commented out in `config/permissions.toml` | Uncomment it, so the builder-role showcase is a real permission test rather than a no-op. |

### Driver-side defects found while scoping (fix in Phase 0)

| # | Defect | Evidence |
|---|---|---|
| D1 | **`list_dir` is registered twice and the jailed one loses.** `register_io_efuns` (`efuns.rs:181`) installs the permission-checked, path-jailed version from `efuns_io.rs:236`; `register_utility_efuns` (`efuns.rs:185`) then **overwrites it** at `efuns.rs:788` with a version that joins `rel_path` directly onto the mudlib and game roots — **no jail, no permission check**. `list_dir("../../..")` escapes. Impact is limited to enumerating `.lua` filenames, not reading them, but `file-access.md` and `sandboxing.md` both claim traversal prevention "for all file efuns". This is the same failure shape as the sandbox and instruction-limit bugs CLAUDE.md's testing section was written about: the correct implementation existed and production never reached it. | `src/core/scripting/efuns.rs:758-788` |
| D2 | **`logging.file` is parsed and ignored.** `main.rs` builds a `tracing_subscriber::fmt()` with no file writer; only `logging.level` is read. Setting it produces no file and no warning. | `config/driver.toml`, `src/main.rs` |
| D3 | **`config()` is an 18-key hardcoded allowlist**, not a generic reader (`efuns.rs:690-751`). Any new config key the game layer needs — including G7's `game.respawn_room` — requires a Rust edit. Worth generalising now rather than once per phase. | `src/core/scripting/efuns.rs:690` |
| D4 | **Nothing invalidates a session's permission cache.** `refresh_permissions` is unused from Lua *and* has no test. `has_permission` reads a cache populated at `enter_game_session`, so a role change never takes effect for an online player. Phase 6's RBAC showcase is the thing that surfaces this. | `src/core/scripting/efuns.rs` (permissions), grep of `mudlib/`+`game/` |
| D5 | **MCCP2 is negotiated but never performed.** `flate2` with `zlib-rs` is a declared dependency that appears **nowhere in `src/`**; `mccp2_active` is never set true. MCCP3's option constant has no handler. | `Cargo.toml:35`, `src/core/network/telnet/` |
| D6 | **`DatabaseBackend::Postgresql` has no runtime path.** Only the sqlite Diesel feature is enabled and `driver.rs` calls `get_sqlite()` unconditionally; selecting it logs "PostgreSQL" then misbehaves. | `src/domain/db/connection.rs`, `src/driver.rs` |
| D7 | **The Lua PRNG is never seeded, so every boot replays the same sequence.** `math.randomseed` appears nowhere in `mudlib/`, `game/`, `src/` or `tests/`, and LuaJIT starts from a constant. Verified: two fresh VMs both return `794206293` for the first `math.random(1, 1e9)`. That means identical combat to-hit and damage rolls, identical loot outcomes, identical weighted echo choices and identical virtual-room description variation on every restart. Seed in Rust at VM construction rather than in `mudlib/init.lua`, so it covers every VM the engine builds — compute workers have their own VMs and are the ones meant to run simulations. `DAEMON.combat._roll` stays overridable, so tests remain deterministic by choice rather than by accident. | `src/core/scripting/engine.rs`, `src/core/compute/vm.rs` |

D1 and D3 block Phase 0. D2 and D4 are small and worth taking with them. D5 and
D6 are decisions to make, not necessarily work to do — either implement, or
demote them from config to documented non-features.

### Memory retention & GC visibility (fix in Phase 0)

The Lua GC will not fall over from normal gameplay — LuaJIT handles far larger
heaps than a MUD produces, and the design already avoids the expensive mistakes
(weak-keyed memo tables at `trait_d.lua:88-89`, one dependency proxy per
recompute rather than per trait at `trait_d.lua:289`, timestamp-based
regeneration so idle players allocate nothing, effect instances as nine plain
fields, the write-behind cache collapsing 1000 changes into one serialisation).

What does matter is **unbounded retention**, which raises the cost of every mark
phase, and the fact that **nothing measures any of it**.

| # | Issue | Evidence |
|---|---|---|
| L1 | **Object state leaks on mob despawn.** `_object_state_store` is a plain Lua table in `_G` keyed by object id (`efuns.rs:832`). Mob instance ids are `"mob:" .. seq` (`mob_d.lua:114-115`) — monotonic and never reused. `despawn` detaches effects, traits and combat (`mob_d.lua:168-170`) but never calls `clear_object_state(mob.id)`, so every mob that ever had state written leaves a permanently retained sub-table, and respawn churns ids forever. The only pruning anywhere is `world_d.lua:281-283` on area reset, which covers rooms in a registered area source — not mobs, not items, not virtual rooms. **Fix:** `clear_object_state(mob.id)` in `despawn`; audit item instances (G1) for the same shape before they exist. | `mudlib/daemons/mob_d.lua:160-175` |
| L2 | **`evict_virtual` has zero callers** — not in `mudlib/`, `game/`, `tests/` or `src/`. `world_d._rooms` accumulates every virtual room ever generated, each holding its exits table, contents, actions and description closures. `world-building.md` says a virtual room is "cached in the registry while occupied"; nothing un-caches it. Bounded for an ocean, **unbounded for the `reach.X.Y` grid in Phase 4** — so this is a prerequisite for that phase, not a cleanup after it. **Fix:** evict on last-occupant-leaves in `world_d.move_character`/`remove_character`, and clear the room's object state with it. | `mudlib/daemons/world_d.lua:120-126` |
| M1 | **No heap or GC visibility, and no GC tuning.** Zero `collectgarbage` calls anywhere; no Rust-side GC configuration, so LuaJIT runs default pause=200 — the heap roughly doubles before a full cycle. With `lua_memory_mb = 64` enforced at `engine.rs:140`, a live set nearing ~32 MB grows into the ceiling; LuaJIT runs an emergency full collection before failing, so the signature under pressure is **latency spikes first, catchable allocation errors second**, surfacing in whatever code happened to allocate rather than the code responsible. **Fix:** add `collectgarbage("count")` and a GC-time counter to `server_info()`, surfaced through `mudstatus`. Roughly twenty lines, and it turns everything above from argument into measurement. | `src/core/scripting/engine.rs:138-143` |

> **Do not tune GC parameters as part of this.** Defaults are usually right and
> tuning blind makes things worse. M1 exists so that any later `setpause` /
> `setstepmul` change is justified by a number.

One cost worth pricing rather than assuming: **`lua_instruction_limit` disables
the JIT, which also disables allocation sinking.** Sinking and store-sinking are
trace optimisations, so with `jit.off()` temporaries the compiler would have
elided are genuinely allocated. The 2-7% in `performance.md` is CPU measured on
single-command criterion runs — that methodology cannot see GC pressure, which
appears as amortised pauses under sustained multi-player load. The limit is
probably still the right default; the point is that its cost has a second
component nobody has measured. M1 plus the Phase 7 load drill is what would
price it.

The shape is not new here. The changelog's `ticker_d.remove_by_prefix` bug
("every per-player timer leaked") was a retention leak, and a leaked closure
pins its upvalues — including the Player object.

> Note on scope: G9–G12 are game-layer systems by the project's own rule
> (`docs/src/rust-api/extending.md`'s "what needs Rust and what doesn't"), so
> they go in `game/daemons/` and `game/cmds/`. G1–G8 and G13 are mudlib/driver
> concerns.

---

## Part 2 — Coverage matrix

Every documented subsystem, the demo content that exercises it, and what
observing it proves. Anything with **no vehicle today** is a coverage hole.

### 2.1 World & rooms

| Feature | Where | Demo vehicle | Proof |
|---|---|---|---|
| Data-oriented areas, `load_area` | `room_d` | every new area | rooms exist and `look` renders |
| `ROOM_D.merge`, multi-file area | `room_d` | `thornhollow/` split into `square.lua`, `market.lua`, `undercroft.lua` + `init.lua` | one area id, three files |
| `_meta` (name/title/author/level/status) | `world_d` | every area; one left `status = "draft"` | `areas` command lists them |
| lfun properties (string \| function) | `Object.resolve` | marsh descriptions keyed on weather; mine descriptions keyed on light | text changes without a reload |
| `light` 0–3, dark rooms | `Room` | mine levels 2–3 at `light = 0` | `look` refuses without a lit lantern |
| `smell` / `sound` | `Room` | marsh and undercroft | present in `look` |
| Scenery `items` | `Room` | every room gets ≥2 | `look <keyword>` |
| Room actions, dispatch precedence | `commands.lua` | a `pull` lever in the mine that shadows nothing, plus a room-scoped `search` | action beats system command |
| Exits, `movement.lua`, opposites | `world_d` | full compass + up/down/in/out in town | movement messaging in both rooms |
| **Virtual rooms** (`register_virtual`, `evict_virtual`) | `world_d` | `reach.X.Y` — an infinite drowned marsh grid, own provider daemon | walk past the static edge; `virtual_prefixes` lists it |
| Area reset (`reset_area`, `area_reset_seconds`) | `world_d` + `tasks/area_reset.lua` | mine puzzle state and depleted ore nodes | state clears; the cooldown-gated node does **not** (the `task_list.md` bug, proven fixed) |
| Object state (4 efuns) | driver | door open/locked, lever position, node depleted | survives `reload`, cleared on reset |
| Builder pattern (`create`/`:set_*`/`:finish`) | `room_d` | the virtual provider uses `from_data`; a small runtime-generated crypt uses the chainable builder | both paths exercised |

### 2.2 Items, equipment, economy

| Feature | Demo vehicle | Proof |
|---|---|---|
| `Item` fields + `on_use`/`on_pickup`/`on_drop` | lantern, rope, lockpick, rations | hooks fire, messages differ |
| Stackables (`stackable`, `quantity`) | coins, arrows, herbs | `inventory` shows `(xN)` |
| **`weapon` component** — `roll_damage`, `speed`, `damage_type`, `two_handed`, hit/miss/crit lfun messages | rusted shortsword (starter), miner's pick (two-handed), silver dagger (`damage_type = "magic"` vs the wisp), a `required_strength = 16` greatsword | requirement refusal message; crit text; damage type interacting with resist |
| **`armour` component** — `defense`, `armor_type`, `resist`, `stat_bonus`, `encumbrance` | leather jerkin, guard's mail (heavy, `required_strength`), warded cloak (`resist = { magic = 6 }`) | `score` shows the `stat_bonus`; damage numbers drop; the cloak visibly blunts the wisp |
| **`requires` component**, shared by both | the greatsword and the guard's mail | one refusal path for level, strength and dexterity |
| Slots + full equipment map | head/chest/hands/feet/weapon/offhand/light/neck | `equipment` command |
| `drinkable` component | healing draught, antidote, ale | existing component gets a second and third user |
| **Containers** (G5) | backpack, bank vault, boss corpse | `put`/`get from`, capacity refusal |
| Shops, `value`, gold sink (G9) | smith (weapons/armor), apothecary (potions), general (tools) | buy/sell/restock; `spend_gold` returns false when broke |
| Loot tables | every mob | drops on kill |

### 2.3 Creatures & combat

| Feature | Demo vehicle | Proof |
|---|---|---|
| Templates, `count`, `populate()` idempotency | rats, marsh lurkers, mine crawlers | `populate` twice, count unchanged |
| `spawn`/`despawn`/`respawn_time` | all | mob returns after its timer |
| `in_room` / `find_in_room` prefix match | `attack lur` | hits the lurker |
| **`aggressive`** (G8) | marsh lurker, mine crawler | attacks on entry |
| **`stationary`** | town guards | never wanders |
| **`unique`** | the Wisp; the mine boss | only one at a time |
| **`patrol`** + `patrol_interval` | a night watchman circling the square | observed moving |
| **`echoes`** + `echo_interval`, weighted, lfun | tavern drunk, forge apprentice | varied ambient lines |
| **`dialogue`** + `on_interact` (G11) | quest givers, trainer, barkeep | `ask smith about ore` |
| **`faction`** | town guard vs bandit | guard assists against bandits, ignores rats |
| **`skills`** (`get_skill`/`set_skill`) | trainer NPC teaching player skills | first caller these methods have ever had |
| `on_death` / `on_spawn` / `on_combat` | boss triggers an area-wide event on death | see 2.6 |
| Combat round, to-hit ±dex, 5–95% clamp | any fight | `trace time` + observed hit rate |
| `damage_taken` pipeline w/ real mitigation (G4) | armored vs unarmored fight | same mob, visibly different numbers |
| `flee`, `disengage_all` | any fight | both sides stop |
| Death, respawn, XP award via `Player:award_xp` | dying to the boss | `death_d`; XP buff applies without combat knowing |
| Deterministic `_roll` override | new tests | pinned numbers |
| **Not covered by design:** initiative, groups, positioning, ranged, pursuit | — | leave alone; `combat.md` calls these out as out of scope |

### 2.4 Traits

| Feature | Demo vehicle | Proof |
|---|---|---|
| `attribute` | existing five + `perception`, `charisma` | `score` |
| `counter` | `level`; add `xp`, `gold` if they should be traits (currently plain Player fields) | decide and document |
| `derived`, incl. derived-of-derived | `carry_capacity` ← strength; `spell_power` ← intelligence + willpower (willpower is itself derived) | a two-level dependency chain |
| `min`/`max` as **another trait's id** | `hp.max = "max_hp"` (exists); add `stamina.max = "max_stamina"` | `seal()` folds the bound into the graph |
| `round` modes | one trait each of `floor`/`ceil`/`round`/`none` | `score` values |
| `hidden` | an internal `luck_seed` | absent from `score` |
| **Gauge regeneration** — remainder carry, re-anchor at target, `offline = false` | `stamina` drained by `flee` and by mining; `mp` by casting | log out an hour, log in, check |
| `seal()` reporting a **cycle as a path** and a missing dep | `game/traits/broken_example.lua`, loaded only by a test | error text names the path; server stays up |
| Memoization + `bump_all` on reload | `reload('daemons.trait_d')` mid-session | values recompute, no stale numbers |
| `Mobile:stat(id)` on a **mob** | mob-vs-mob or boss stats | works off-player |

### 2.5 Effects

| Feature | Demo vehicle | Proof |
|---|---|---|
| `modifiers` sugar (flat + `"+10%"`) | `hearty`, `well_fed` | desugars to `trait:` handlers |
| Explicit `hooks` with phases `pre/add/mult/reduce/clamp/post` | armour (`reduce`), stoneskin (`mult`), immunity ring (`pre`, cancels) | 30-damage hit yields the documented 20, not 21 |
| Multipliers **add, not compound** | two +20% XP buffs | +40% exactly |
| `tick` / heartbeat | marsh poison (DoT), regeneration | damage over time with no per-effect timer |
| `duration` + lazy expiry + `sweep()` messaging | any timed buff | `on_expire` message arrives while idle |
| Stack modes — all five | `stack`: drunk (3) · `refresh`: blessing · `independent`: well_fed · `ignore`: warded · `replace`: brand | `effects` command shows each behaving |
| `condition` via `checks.lua` predicates | a blessing that only lands on a character with a holy symbol | first user of `lib/checks.lua` |
| `survives_death` | the boss's curse | still on you after respawn |
| `persist = false` | `equip:<slot>` auras (G3), room auras | never written; reapplied on login |
| Source schemes `potion:` `spell:` `equip:` `room:` `quest:` `admin:` | one of each | `effects` shows sources |
| Conditional handler no table could express | berserker: `+5 str` under half health | the doc's own example, made real |
| Re-entrancy cap, per-handler `pcall` | a deliberately throwing effect in a test | one bad effect doesn't break the chain |

### 2.6 Events, timers, tasks

| Feature | Demo vehicle | Proof |
|---|---|---|
| `emit`/`on`/`off`/`off_by_prefix`, priority order | `mob.died` → loot, XP, quest counter, faction enrage, all at different priorities | ordering observable |
| `defer(event, data, delay)` | mine collapse warning 10s before it happens | delayed emit |
| Documented event names (`room.entered`, `player.levelup`, `item.picked_up`, `area.reset`…) | the aggro handler, quest hooks, the board | the naming convention gets used, not just documented |
| `ticker.after` / `every` / `remove` / `remove_by_prefix` | puzzle reset, weather cycle, patrol, echoes | `tasks`/`events` admin commands show them |
| **`task_d`** — `schedule`/`pause`/`resume`/`run_now`/`cancel` | shop restock, tide cycle in the reach | **currently undocumented and unused**; `tasks` command drives it |

### 2.7 State tiers & persistence

| Feature | Demo vehicle | Proof |
|---|---|---|
| `memory` tier | aggro tables, combat targets, sub-minute spell cooldowns | gone after restart, correctly |
| `write_behind` tier | quest kill-counters, statistics, effects | survives restart, may lose <`flush_seconds` on crash |
| `write_through` tier | daily reward claim, shop purchase ledger | survives a `kill -9` |
| `character` tier (`SAVE_FIELDS`) | traits, gauges, gold, quest flags, equipment | `score` after relog |
| `DAEMON.cooldown` durable vs memory (60s threshold) | daily herb node (24h, durable) vs a 6s spell (memory) | the daily gate **survives an area reset** — the original `task_list.md` bug |
| `edit` vs `get_scope` dirty-marking | quest counters | a mutation through `get_scope` that never persists is exactly the trap to demonstrate in a test |
| `flush_owner`/`evict_owner` on disconnect | any cached player state | written on logout |
| 48 KB warn / 64 KB refuse | a test that inflates a scope | named refusal, not a flush-time raise |
| `lua_to_json` refusing all six value kinds | a test per kind | field named in the error |
| **Document store — all 12 `db_*`** | notice board + player statistics + shop ledger | see below |
| Filter operators `== ~= > >= < <= in nin like exists contains`, dotted paths, `limit/offset/sort/order` | `board search`, `top` leaderboard command | each operator used at least once |
| `db_incr` | board view counts | atomic increment |
| `max_results` **erroring** rather than truncating | a test with >500 matches | hard error |

### 2.8 Interface & protocol

| Feature | Demo vehicle | Proof |
|---|---|---|
| Prompt templates (`%h %H %m %M %g %x %l %r %n`) | a per-class default prompt; `prompt` command | rendered after every command |
| Colour (`{red}`, `{fg:N}`, `{bg:N}`, `{/}`), `color` toggle | area titles, damage numbers, channel text | `color off` strips cleanly |
| Pager + `pagesize` | `help`, `board`, `who` on a busy server | paging intercepts input |
| Channels (`create/join/leave/send/list`), channel-name shortcut dispatch | `chat`, `newbie`, `trade`, and a `staff` channel gated by permission | `chat hello` works as a verb |
| **GMCP** — `Char.Vitals`, `Char.Status`, `Char.Effects`, `Room.Info` | pushed on move, damage, effect change | Mudlet inspector |
| **`on_gmcp` inbound** (currently only logs) | client sends `Core.Supports.Set`; add a `Char.Login` / custom `Game.Quest` package | first real inbound handling |
| ECHO masking | login | password hidden |
| NAWS / TTYPE | `get_session().window_width`/`terminal_type` are already exposed by the driver | wrap output to the client's real width in `Player:get_width` |
| MCCP2 | **negotiated but not applied to the write stream** | decide: implement zlib wrapping, or document as a known gap. Not blocking. |
| Multisession modes | test each of the four | second connection behaves per config |

### 2.9 Admin, building, observability

| Feature | Demo vehicle | Proof |
|---|---|---|
| RBAC: `create_role`, `grant_permission`, `assign_role`, `refresh_permissions` | `player` / `builder` / `staff` / `admin` roles, set up by a `game/setup_roles.lua` run once | role change takes effect without relog |
| `M.permission` command gating | all new admin/builder commands | denial is audited |
| `permissions.toml` `[efuns]` + `[directories]` | uncomment `/areas` (G13); builder can write areas, player cannot | denial message |
| Superuser bypass | account 1 | bypasses everything |
| **OLC** — `olc`, `dig`, `olc_d`, `codegen_d` | a `sandbox` area built live in-game and written to disk by `codegen_d`, then loaded on next boot | round-trip: build → generate → reload → walk it. **OLC has no doc page at all — write one.** |
| `journal` command + automatic Lua error capture | deliberately break a room action | file:line + traceback + session in `logs/journal.log` |
| `audit` + watch list persistence | watch `spawn` and `goto`; deny a player | `logs/audit.log`, `audit_watch.json` |
| `alert` / `announce` / `broadcast_to_perm` | staff-only alert vs global announce | different audiences |
| `verify` / `verify_file` | verify a syntactically broken area before reloading | compile without executing |
| `snoop` | staff snooping a test session | output mirrored; self/chain snoop refused |
| `trace calls`/`lines`/`time`/`timings` | trace a spell cast and a combat round | per-command timings |
| VS Code debug adapter | breakpoint in the quest daemon; conditional breakpoint on one `char_id` | stepping, locals, watch |
| `mudstatus` / `uptime` / `server_info()` | after a load test | uptime, output-drop counters, `compute.wedged` |
| Hot reload (`reload`, `on_load`/`on_unload`, DAEMON rebinding, trait bump) | reload `quest_d` while a quest is in progress | quest survives; values recompute |

### 2.10 Compute & sandbox

| Feature | Demo vehicle | Proof |
|---|---|---|
| `compute()` off-thread, `on_compute_result`, `meta.tag` | a `navigate <room>` command pathfinding across the world graph, including the virtual reach grid | the game does **not** freeze; the doc's own worked example, made real |
| Revalidation of a stale result | `navigate` re-checks `still_connected` before walking | the most important line in `compute.md` |
| `compute_cancel`, deadline, `kind` values | cancel a long route; force a timeout | each `meta.kind` observed |
| Worker VMs have no efuns | a compute module that tries `send()` | fails as designed |
| Marshalling refusals (functions, cycles, depth/node caps) | tests | refused at the call site |
| Sandbox — `io`, `os.execute`, `debug`, `jit`, bytecode, path traversal | extend `tests/sandbox_reality_check.rs` | all refused **through the real engine VM**, per CLAUDE.md's rule |
| `lua_instruction_limit` | an admin `eval`-style test command running a runaway loop | catchable error, game survives |
| `lua_memory_mb` | allocation past 64 MB | catchable error |
| **Known gap:** `while true do pcall(...) end` still wedges | — | leave; documented in `sandboxing.md` |

### 2.11 Lifecycle

| Feature | Demo vehicle | Proof |
|---|---|---|
| Login / registration / async Argon2 / lockout | 6 bad passwords from one address | 30s lockout, game never freezes |
| `authenticate_session` / `enter_game_session` / permission cache | login | `get_session().state == "playing"` |
| Autosave ticker | play 5 min, `kill -9` | progress since last autosave lost, earlier kept |
| **Clean shutdown** (`on_shutdown`, `shutdown_timeout_seconds`) | Ctrl+C mid-session | everything saved — the Phase-3 fix, re-proven with a bigger state surface |
| `on_disconnect` six-step protected cleanup | disconnect mid-combat, mid-page, while snooped, while in OLC | every step runs |

### 2.12 Cold surface — registered in Rust, never called from Lua

This is the most direct answer to "what would I need to fully test everything we
have available". Each row is a capability that has never once been invoked by
game code.

| Efun / family | Demo vehicle that lights it up | Phase |
|---|---|---|
| `create_role`, `delete_role`, `list_roles`, `assign_role`, `revoke_role`, `get_roles`, `grant_permission`, `revoke_permission`, `get_permissions` | `game/setup_roles.lua` + a `role` admin command that grants the builder role in-game | 6 |
| `refresh_permissions` | promoting a player to builder **while they are online** — the only thing that exercises D4 | 6 |
| `db_find`, `db_count`, `db_insert`, `db_exists`, `db_update`, `db_unset`, `db_incr`, `db_collections`, `db_clear` | notice board (`db_insert`/`db_find` with every operator, `db_incr` view counts), leaderboard (`sort`/`limit`), shop ledger (`db_update` merge), an admin `dbstat` (`db_collections`/`db_count`) | 1 |
| `compute`, `compute_cancel` | `navigate <room>` pathfinding; cancel a long route | 4 |
| `this_session` | any helper that needs the acting session without it being threaded through — the quest daemon is the natural first caller | 3 |
| `get_account` | a `finger`/`whois` staff command showing account creation date and admin flag | 6 |
| `file_exists`, `append_file`, `delete_file` | `codegen_d` round-trip: check before write, append to a build log, delete a scrapped room | 6 |
| `os_clock` | the `trace`-adjacent timing in `spell_d`'s cast resolution, or a `bench` admin command | 5 |
| `on_gmcp` inbound (defined, only logs) | client `Core.Supports.Set` handling + a custom `Game.Quest` package | 7 |
| `Mobile:get_dialogue`, `get_skill`, `set_skill` | quest givers and the trainer NPC | 1 / 5 |
| `Player:set_quest_flag`, `get_quest_flag`, `has_quest_flag` | the quest chain | 3 |
| `lib/checks.lua` predicates (`has_item`, `has_level`, `cooldown_ready`, `all`, `any`) | effect `condition` fields and quest prerequisites | 3 / 5 |
| `task_d` (`schedule`/`pause`/`resume`/`run_now`) | shop restock, tide cycle | 1 / 4 |
| `Item.on_use`/`on_pickup`/`on_drop`/`on_equip`/`on_remove` | G2/G3 verbs | 0 |
| `weapon` / `armour` / `requires` components entirely | the gear table in §2.2 | 0 / 1 |
| `Mobile.aggressive` / `stationary` / `unique` / `patrol` / `echoes` / `faction` | §2.3 | 0 / 1 / 2 |

*(`start_echo`/`stop_echo` are sometimes listed as cold — they are not; `mudlib/login.lua:74,79,103,107` uses both. They lack a Rust test, which is a smaller gap.)*

---

## Part 3 — The demo world

**Thornhollow** — a frontier town at the mouth of a collapsed mine, on the edge
of a drowned marsh. Chosen because each area has a natural reason to exercise a
different subsystem, not because the setting is novel.

| Area | Rooms | Exists to prove |
|---|---|---|
| `thornhollow` (multi-file: `square` / `market` / `undercroft`) | ~12 | shops, dialogue, quest givers, channels, board, bank vault (container), guards (stationary/patrol/faction), trainer (skills), the builder `sandbox` door |
| `greywater_marsh` | ~10 | weather daemon driving lfun descriptions and light, aggressive mobs, the unique Wisp, poison DoT, the durable-cooldown herb node, `damage_type` vs `resist` |
| `collapsed_mine` (3 levels) | ~14 | dark rooms + light source, locked door + key (object state), lever puzzle with a timed reset, unique boss with `survives_death` curse, corpse container, area reset made visible |
| `drowned_reach` (virtual, `reach.X.Y`) | ∞ | virtual provider, caching + eviction, `compute()` pathfinding home |
| `sandbox` | grows | OLC / `dig` / `codegen_d` round-trip under the `builder` role |
| `wizard_workshop` | 6 | **unchanged** — the existing regression fixture the real-mudlib tests lean on; add a side door from town |

New game-layer daemons: `weather_d` (the daemon recipe from the docs, made
real), `aggro_d` (G8), `quest_d` (G10), `spell_d`, `board_d`.
New mudlib daemons: `shop_d` (G9), plus `item_d` extended for instances (G1).

Five quests, one per persistence shape: fetch (items), kill-count
(write-behind counter), delivery (cross-area), daily (durable cooldown), chain
(quest flags gating the next).

Four spells: one damage (through the pipeline), one heal (gauge `adjust`), one
buff (effect with `condition`), one utility (memory-tier cooldown, mp cost).

---

## Part 4 — Build order

Each phase ends runnable and testable. Do not start a phase until the previous
one's tests pass.

| Phase | Work | Unblocks |
|---|---|---|
| **0. Verbs & driver fixes** | D1 (`list_dir` shadowing — do this first, it is a jail escape), D2, D3 (generalise `config()`), D4, D7 (seed the PRNG). M1 (heap/GC counters in `server_info`) **before** the leak work, so the fixes can be shown to move a number. Then L1, L2. Then G1–G8, G13: item instances, get/drop/put/give/use/examine, wear/wield/remove + `equip:` effect sources, containers, real `help` + `stat`, `death_d` config, aggro handler, `/areas` permission. **Item identity:** per-instance state belongs on the instance entry (already saved), not in object state (does not survive restarts); a `uuid()` efun — the `uuid` crate is already a dependency — covers the ground/container items that genuinely need addressing | everything |
| **1. Thornhollow** | Town multi-file area, `shop_d` (G9), NPC dialogue (G11), `emote` (G12), channels, `board_d` over `db_*`, **`tag_d`** — built here rather than earlier so it lands with its first real consumers (faction lookups for the aggro handler, `find("outdoor")` for `weather_d` in Phase 2) | economy, document store, dialogue, tags |
| **2. Greywater Marsh** | `weather_d`, lfun descriptions, aggressive/unique mobs, poison DoT, herb node on a durable cooldown, resist/damage-type gear | effects breadth, cooldown tiers |
| **3. Collapsed Mine** | Dark rooms + lantern, locked door, lever puzzle + timed reset, boss + `survives_death`, corpse container, `quest_d` (G10) + five quests | quests, object state, area reset |
| **4. Drowned Reach** | Virtual provider, eviction, `navigate` via `compute()` with revalidation. **Requires L2** — an infinite grid over a registry nothing evicts is an unbounded leak, so this phase is blocked on it rather than cleaning up after it | virtual rooms, compute |
| **5. Traits & spells** | Trait breadth (derived-of-derived, all `round` modes, stamina gauge, `hidden`, broken-trait fixture), `spell_d` + four spells | traits, effects, gauges |
| **6. Staff & building** | Roles setup script, `sandbox` area via OLC/`dig`/`codegen_d` round-trip, snoop/trace/journal/audit walkthrough, **write the missing OLC doc page** | RBAC, OLC, observability |
| **7. Protocol & ops** | GMCP packages incl. a custom one and real inbound `on_gmcp`, NAWS-aware wrapping, prompt/colour/pager polish, multisession matrix, shutdown/autosave/reset drills, MCCP2 decision | protocol, lifecycle |

---

## Part 5 — Verification

**Per phase:**

1. `cargo test` — all 548 existing tests still pass, plus the phase's new ones.
2. `cargo run`, connect on port 4000, walk the phase's content by hand.

**New test files**, each following the harness rule in `CLAUDE.md` — anything
touching a security or persistence boundary goes through
`tests/common/mod.rs`'s real `ScriptEngine`, in the shape of
`tests/sandbox_reality_check.rs`, never a helper called in isolation:

| File | Covers |
|---|---|
| `tests/items_ground.rs` | instances, get/drop/put/give, events fired |
| `tests/equipment.rs` | slots, requirements, `equip:` sources applied and removed |
| `tests/combat_mitigation.rs` | phase ordering yields 20 not 21; `_roll` pinned |
| `tests/quests.rs` | flags, write-behind counters, daily cooldown surviving an area reset |
| `tests/shop.rs` | buy/sell/restock, `spend_gold` refusal |
| `tests/virtual_rooms.rs` | generation, caching, eviction, reconnect regeneration |
| `tests/compute_navigate.rs` | result delivery, stale-result revalidation, cancel, timeout |
| `tests/traits_breadth.rs` | derived-of-derived, trait-as-bound, cycle reported as a path, regen remainder |
| `tests/effects_stacking.rs` | all five stack modes, `condition`, `survives_death`, `persist=false` |
| `tests/list_dir_jail.rs` | D1 — `list_dir("../..")` refused **through the engine's VM**, not through `efuns_io`'s helper. The existing suite passed while the jailed version was unreachable; a helper-level test would pass again. |
| `tests/permission_refresh.rs` | D4 — grant a role to an online session, confirm `has_permission` changes only after `refresh_permissions` |
| `tests/state_retention.rs` | L1 — spawn a mob, write object state, despawn, assert `get_all_object_state(id)` is nil and `_object_state_store` has no entry. Loop spawn/despawn 1000× and assert the store's key count is flat, not growing with `seq` |
| `tests/virtual_eviction.rs` | L2 — walk a virtual grid, leave, assert the room is gone from `world_d._rooms` and its object state with it; assert regeneration on return still works (the room ID is the persistence) |

**End-to-end drills** (manual, once per phase from 3 onward):

- Play 5 minutes, `kill -9` → autosave boundary is where it should be.
- Play 5 minutes, Ctrl+C → everything saved.
- Disconnect mid-combat, mid-page, while snooped, while in OLC → all six
  `on_disconnect` steps run, journal is clean.
- `reload` each new daemon mid-session → state survives, DAEMON rebinds.
- Wait out one `area_reset_seconds` → puzzle state clears, the daily gate does not.
- Trace a spell cast and a combat round; confirm no command exceeds ~2 ms.
- **Heap drill (from Phase 4 onward, using M1's counters):** record
  `collectgarbage("count")` at boot, then again after an hour of a mob respawn
  loop, a walk out into the virtual grid and back, and several `reload` cycles.
  The number should return close to its baseline each time. A monotonic climb
  across all three is the signature L1, L2 and closure retention on hot reload
  produce, and it is the only way to tell them apart from ordinary working set.
  Re-run once with `lua_instruction_limit = 0` to price the allocation-sinking
  cost noted above.

**Documentation** to write alongside (all are real gaps found in `docs/`):
an OLC page, and reference pages for `task_d`, `item_d`, `death_d`, `channel_d`,
`pager_d`, `snoop_d`, `prompt_d` template syntax, and `lib/color.lua` — each is
currently a single table row in `daemons.md`. Also refresh the stale coverage
table in `testing.md` and delete or fill the two 5-line redirect stubs
(`configuration/driver-toml.md`, `configuration/server-toml.md`) still listed in
`SUMMARY.md`.
