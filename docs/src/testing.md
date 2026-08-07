# Testing

Oxigeon uses **Rust integration tests** to verify both the driver and the Lua mudlib.
All tests live in the `tests/` directory and run via `cargo test`.

## The one rule about layout

**A test of the mudlib must not depend on this game.**

`game/` is content — "this game, and policy the driver has no view on" — so
anyone building their own world deletes it. A suite that then fails about rooms
they never wrote is a suite that has to be picked apart before it can be
trusted. So:

| | |
|---|---|
| `tests/driver/` | the engine: the stores, the sandbox, the debugger, the file jail, permissions. |
| `tests/mudlib/` | the Lua system layer: OLC, schema, components, abilities, GMCP, the cache. |
| `tests/demo_world/` | Thornhollow, the marsh, the mine, the workshop. **Deleted along with `game/`.** |

Three binaries rather than sixty, and the split between the first two answers
one question:

> **If you deleted `mudlib/` and wrote your own from scratch, would you keep this
> test or rewrite it?** Keep it -> `tests/driver/`. Rewrite it -> `tests/mudlib/`.

That resolves the genuinely ambiguous ones without arguing. `staff` is a driver
test although it drives the mudlib, because the RBAC efuns are Rust and the
mudlib is only the vehicle. `fs_shell` is a mudlib test although it exercises
the jail, because `ls` and `cd` are mudlib commands and a new mudlib would write
its own.

`tests/compute_wedge.rs` is deliberately **not** in either, and the reason is in
its own header: every test in it spins a core in `while true do end` for the
whole of its deadline, so as a neighbour it starves whatever it shares a binary
with. Folded into `tests/driver/` it made the pool-recovery test fail
intermittently -- a job that could not get scheduled inside a *forty-second*
deadline, with nothing wrong with the pool. Merge it into another binary and
that comes back.

`boot_with_fixture_world` writes a small self-contained game layer into a temp
directory — three rooms, one creature, one item, a trait set, a role, and one
game-layer command — and boots the real mudlib against it. Traits and roles are
in there because both are game-layer by design: a world with no trait
definitions has no `hp` for anything to lose.

`boot_fixture_with_probe` is the same world behind the probe dispatcher, for a
test that needs `eval` against a wired `DAEMON` table rather than a player's
view. Prefer it over `boot_real_mudlib_with_probe`, which puts the real `game/`
on `package.path`: anything that seeds a creature through *that* is quietly
asserting that this game defines the traits.

The check that keeps this honest, and the only one that proves it:

```bash
# `git stash push <path>` only reverts changes to tracked files — it leaves the
# directory in place, so the version of this check that used it never removed
# `game/` and never proved anything. Move them out of the tree.
mkdir ../away && mv game ../away/ && mv tests/demo_world ../away/
cargo test --test driver --test mudlib --no-fail-fast
mv ../away/game . && mv ../away/demo_world tests/ && rmdir ../away
```

That must be green. If a test you are writing fails it, ask whether it is really
asserting an authored value — a room's prose, a mob's hit points, a quest id. If
so it belongs in `tests/demo_world/`. If not, it wants the fixture.

`tests/mudlib/fixture_world.rs` is the proof the fixture is a real world you can play
in, and `boot_real_mudlib` now reads `start_room` out of `config/server.toml`
rather than hardcoding one, so re-pointing the config re-points the harness.

## Quick Start

```bash
# Run the entire test suite
cargo test

# Run one bucket
cargo test --test driver
cargo test --test mudlib
cargo test --test demo_world

# Run one file's worth, by module name
cargo test --test mudlib lua_unit

# Run a single test by name
cargo test --test mudlib test_mobile_take_damage
```

The `--test` argument is the *binary* now, not the file. What used to be
`tests/mudlib/schema.rs` is `tests/mudlib/schema.rs` and a module inside one binary, so
`cargo test --test schema` no longer resolves -- use
`cargo test --test mudlib schema`, which filters by module path.

All tests should pass before committing — 1242 at the time of writing, green on
the default Lua 5.5 build and on `--no-default-features --features luajit`. The
Lua unit tests alone run in ~20ms.

`cargo test` does not build `oxigeon-compute`. It is a separate workspace member
that links LuaJIT unconditionally, and cargo unifies features across a single
invocation, so making it a default member would break every `lua55` build. The
harness builds it on demand, into its own `target/compute-worker/` — a shared
target directory would contend for the build lock the outer `cargo test` holds,
which looks like a test run that hangs with no output.

---

## How Lua Tests Work

The Lua test file (`tests/mudlib/lua_unit.rs`) boots a **lightweight LuaJIT VM** that:

1. Points `package.path` at the **real** `mudlib/` and `game/` directories
2. Stubs only the efuns that touch Rust state (networking, DB, sessions)
3. `require()`s the actual module under test
4. Asserts against Lua return values from Rust

This means tests exercise real production code — no copies, no mocks of Lua logic.

### The Test Harness

The `make_test_lua()` function sets up a ready-to-use VM:

```rust
fn make_test_lua() -> Lua {
    let lua = Lua::new();

    // Point require() at the real mudlib/ and game/ directories
    // (package.path is set relative to CARGO_MANIFEST_DIR)

    // Stub efuns that would normally be provided by the Rust driver:
    // log(), send(), send_prompt(), get_session(), get_character(),
    // set_object_state(), get_object_state(), has_permission(),
    // config(), write_file(), read_file()

    // Initialize empty DAEMON table
    lua.load("DAEMON = {}").exec().unwrap();

    lua
}
```

The stubs are intentionally minimal:

| Stub | Behavior | Why |
|------|----------|-----|
| `log(level, msg)` | No-op | Prevents errors when modules log at load time |
| `send(sid, text)` | No-op | Modules can call send without crashing |
| `send_prompt(sid, text)` | No-op | Same |
| `get_session(sid)` | Returns `nil` | No real sessions in unit tests |
| `get_character(id)` | Returns `nil` | No real DB |
| `set_object_state` / `get_object_state` | Backed by a Lua table | Object state works correctly without Rust |
| `has_permission(sid, perm)` | Always `true` | Permissions aren't the thing being tested |
| `config(key)` | Returns `nil` | No server config in tests |
| `write_file` / `read_file` | No-op / `nil` | Filesystem not needed for most tests |

### Rust Helper Functions

Four helper functions eliminate boilerplate:

```rust
fn eval_bool(lua: &Lua, code: &str) -> bool   // Lua → bool
fn eval_str(lua: &Lua, code: &str) -> String   // Lua → String
fn eval_int(lua: &Lua, code: &str) -> i64      // Lua → integer
fn eval_num(lua: &Lua, code: &str) -> f64      // Lua → float
```

---

## Writing a New Test

### 1. Simple Value Test

Test that a module returns the expected value:

```rust
#[test]
fn test_item_default_weight() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load(r#"item = Item:new({ id = "test.gem" })"#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return item.weight"), 1);
}
```

### 2. Method Behavior Test

Test that a method produces the correct side effect:

```rust
#[test]
fn test_mobile_take_damage_clamps_to_zero() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t", stats = { hp = 10, max_hp = 10 } })"#).exec().unwrap();

    // Partial damage
    assert_eq!(eval_int(&lua, "return mob:take_damage(5)"), 5);
    // Overkill clamps to 0
    assert_eq!(eval_int(&lua, "return mob:take_damage(100)"), 0);
    // Dead
    assert!(!eval_bool(&lua, "return mob:is_alive()"));
}
```

### 3. Roundtrip / Serialization Test

Test that data survives a save/load cycle:

```rust
#[test]
fn test_player_to_save_roundtrip() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        local saved = {
            stats = { hp = 75, max_hp = 100 },
            gold = 100,
            skills = { archery = 7 },
        }
        player = Player:from_save(1, { id = 1, name = "Archer", account_id = 1 }, saved)
        exported = player:to_save()
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return exported.stats.hp"), 75);
    assert_eq!(eval_int(&lua, "return exported.gold"), 100);
    assert_eq!(eval_int(&lua, "return exported.skills.archery"), 7);
}
```

### 4. Lfun / Dynamic Property Test

Test that lfun resolution works (functions-as-properties):

```rust
#[test]
fn test_mobile_dialogue_with_lfun() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({
        id = "t",
        dialogue = {
            greet = "Hello!",
            quest = function(self) return "Level " .. self.stats.level .. " quest" end,
        }
    })"#).exec().unwrap();

    // Static string
    assert_eq!(eval_str(&lua, r#"return mob:get_dialogue("greet")"#), "Hello!");
    // Dynamic lfun
    assert_eq!(eval_str(&lua, r#"return mob:get_dialogue("quest")"#), "Level 1 quest");
    // Missing key
    assert!(eval_bool(&lua, r#"return mob:get_dialogue("unknown") == nil"#));
}
```

### 5. Inheritance Chain Test

Verify that a subclass has access to its parent's methods:

```rust
#[test]
fn test_player_inherits_mobile_methods() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        player = Player:from_save(1, { id = 1, name = "X", account_id = 1 }, {})
    "#).exec().unwrap();

    // Mobile methods are available on Player
    assert!(eval_bool(&lua, "return player:is_alive()"));
    assert_eq!(eval_int(&lua, "return player:take_damage(30)"), 70);

    // Object methods are also available
    assert!(eval_bool(&lua, "return player.get_state ~= nil"));
}
```

### 6. Testing with Custom Stubs

If your test needs a specific daemon to be available, set it up in Lua before running assertions:

```rust
#[test]
fn test_room_d_load_area_with_world_daemon() {
    let lua = make_test_lua();
    lua.load("ROOM_D = require('daemons.room_d')").exec().unwrap();

    // Provide a stub DAEMON.world so load_area can call set_area_meta
    lua.load(r#"
        DAEMON.world = { set_area_meta = function() end }
    "#).exec().unwrap();

    lua.load(r#"
        local area = {
            _meta = { name = "test", title = "Test" },
            { id = "test.r1", short = "Room 1" },
        }
        rooms = ROOM_D.load_area(area)
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return #rooms"), 1);
}
```

---

## What's Currently Tested

**687 tests.** The table below is the *integration* suite — the files that boot
a real `ScriptEngine` and ask what game code can actually do. `tests/mudlib/lua_unit.rs`
is a further 160-odd unit tests over the Lua libraries in isolation.

### The security and boundary suites

Anything touching a security or persistence boundary goes through
`tests/common/mod.rs`'s real engine, per the rule at the top of this page.

| File | Covers |
|---|---|
| `sandbox_reality_check.rs` | `io`, `os.execute`, `debug`, `jit`, bytecode and path traversal, refused **through the engine's own VM** |
| `list_dir_jail.rs` | D1 — the second, unjailed `list_dir` that overwrote the jailed one |
| `instruction_limit.rs` | the budget is armed and enforced, not merely parsed |
| `permission_config.rs`, `permissions.rs` | RBAC storage and the session cache |
| `permission_refresh.rs` | D4 — a role change reaching a player who is already online |
| `state_retention.rs` | L1/L2 — object state on despawn, virtual rooms on eviction, and the heap counters |

### The game systems

| File | Covers |
|---|---|
| `traits_effects.rs` | traits and effects as a player meets them; presence, seeding, learning |
| `trait_sparsity.rs` | `all()` filtering, `category` as a lens, and the O(entity) recompute counted rather than timed |
| `traits_breadth.rs` | derived-of-derived, every `round` mode, `hidden`, offline regeneration, the broken-trait fixture, and spells |
| `items_ground.rs` | instances, `get`/`drop`/`put`/`give`, containment cycles, hooks and events |
| `equipment.rs` | slots, requirements, two-handed displacement, `equip:` sources applied and removed |
| `combat_mitigation.rs` | phase ordering with real armour; damage type meeting a resist table |
| `shop.rs` | prices, the gold sink, restocking, the ledger over `db_*` |
| `board.rs` | every document-store filter operator, `db_incr`, `db_update` as a merge, `db_unset` |
| `quests.rs` | all three persistence tiers, and a daily gate surviving an area reset |
| `thornhollow.rs` | the multi-file area, dialogue, factions, echoes, tags, room-action precedence |
| `marsh.rs` | weather-driven lfun descriptions, poison on the heartbeat, conditions, `survives_death` |
| `mine.rs` | dark rooms and light sources, a locked door, the lever puzzle, the boss's corpse |
| `virtual_rooms.rs` | generation, the exit graph, `still_connected`, and `compute()` pathfinding end to end |
| `staff.rs` | roles declared in a file, granted in-game, and `/areas` actually gated |
| `gmcp_inbound.rs` | `Core.Supports.Set` read and gating what is pushed; a custom package |
| `lifecycle.rs` | container contents through save and load, and the disconnect ordering |

### The driver

| File | Covers |
|---|---|
| `account_store.rs`, `character_store.rs` | persistence |
| `auth_off_thread.rs` | Argon2 off the game thread, and the lockout |
| `clean_shutdown.rs` | `on_shutdown` runs and is waited for |
| `compute_bridge.rs`, `compute_wedge.rs` | job delivery, marshalling refusals, a wedged worker |
| `document_store.rs`, `document_efuns.rs` | the store and its twelve efuns |
| `hot_reload.rs` | `reload`, `on_load`/`on_unload`, DAEMON rebinding |
| `state_cache.rs` | tiers, dirty marking, flush planning, quarantine |
| `observability.rs`, `game_logger.rs` | the journal and the audit trail |
| `output_backpressure.rs` | what happens when a client stops reading |
| `dap_attach.rs`, `debug_*.rs` | the debug adapter |
| `real_mudlib_harness.rs` | the harness itself |

---

## Tips

- **Each test gets its own VM.** State doesn't leak between tests, and they run in parallel.
- **Use `r#"..."#` for Lua strings** in Rust to avoid escaping quotes.
- **Check `nil` with `== nil`** in Lua, not with Rust's `Option` — `eval_bool(lua, "return val == nil")` is the cleanest pattern.
- **Errors in `.exec().unwrap()`** will print the Lua stack trace, which makes debugging straightforward.
- **Add a stub if a new efun is needed.** If you add a new efun that modules call at load time, add a minimal stub in `make_test_lua()` so tests don't break.
- **Test the real modules.** Don't copy Lua code into your test — `require()` the actual file so your tests catch regressions in the real code.
