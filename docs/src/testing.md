# Testing

Oxigeon uses **Rust integration tests** to verify both the driver and the Lua mudlib.
All tests live in the `tests/` directory and run via `cargo test`.

## Quick Start

```bash
# Run the entire test suite
cargo test

# Run only the Lua unit tests
cargo test --test lua_unit

# Run a single test by name
cargo test --test lua_unit test_mobile_take_damage

# Run tests matching a pattern
cargo test --test lua_unit test_player
```

All 200+ tests should pass before committing. The Lua unit tests alone run in ~20ms.

---

## How Lua Tests Work

The Lua test file (`tests/lua_unit.rs`) boots a **lightweight LuaJIT VM** that:

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

| Module | Tests | What's Covered |
|--------|------:|----------------|
| Object | 7 | `resolve()` with all value types, `new()` defaults, state roundtrip |
| Item | 6 | Defaults, slot→equippable, stackable display, tags, examine |
| Mobile | 8 | Stats, damage/heal clamping, inventory, skills, tags, dialogue, examine, aggression |
| Player | 7 | Hydration, serialization roundtrip, deep copy, gold ops, quest flags, display name, inheritance |
| Room | 4 | Exits, character management, actions, items |
| ROOM_D | 4 | `from_data`, validation, builder API, `load_area` |
| Codegen | 2 | Room and meta generation + loadability |
| Inheritance | 4 | Object→Item, Mobile→Player, Item→Weapon, Item→Armor |
| Commands | 5 | Parse: basic, empty, verb-only, multi-arg, case-insensitive |

---

## Tips

- **Each test gets its own VM.** State doesn't leak between tests, and they run in parallel.
- **Use `r#"..."#` for Lua strings** in Rust to avoid escaping quotes.
- **Check `nil` with `== nil`** in Lua, not with Rust's `Option` — `eval_bool(lua, "return val == nil")` is the cleanest pattern.
- **Errors in `.exec().unwrap()`** will print the Lua stack trace, which makes debugging straightforward.
- **Add a stub if a new efun is needed.** If you add a new efun that modules call at load time, add a minimal stub in `make_test_lua()` so tests don't break.
- **Test the real modules.** Don't copy Lua code into your test — `require()` the actual file so your tests catch regressions in the real code.
