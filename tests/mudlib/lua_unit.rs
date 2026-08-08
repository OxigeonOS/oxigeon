//! Lua unit tests for mudlib core modules.
//!
//! Tests pure Lua logic (Object, Item, Mobile, Player, Room, codegen_d, etc.)
//! using a lightweight Lua VM with stubbed efuns.  No ScriptEngine, no DB,
//! no sessions — just `require()` the real mudlib files and assert.

use mlua::prelude::*;
use std::path::PathBuf;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Return the workspace root (one level up from `tests/`).
fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Create a Lua VM wired to require() from the shipped mudlib.default/ and
/// game.example/ dirs, with lightweight efun stubs so modules can load without
/// crashing.
///
/// The shipped pair rather than the live `mudlib/` and `game/`: those are the
/// creator's own trees, untracked and absent on a clean clone, and since the
/// game root *shadows* the mudlib on `package.path` a module under test here
/// could otherwise be silently replaced by one nobody in this repo wrote.
fn make_test_lua() -> Lua {
    let lua = Lua::new();
    let root = project_root();

    let mudlib = root.join("mudlib.default");
    let game   = root.join("game.example");

    // Canonical, forward-slash paths for Lua's package.path
    let mudlib_s = mudlib.to_string_lossy().replace('\\', "/");
    let game_s   = game.to_string_lossy().replace('\\', "/");

    // Strip Windows \\?\ prefix if present
    let mudlib_s = mudlib_s.strip_prefix("//?/").unwrap_or(&mudlib_s).to_string();
    let game_s   = game_s.strip_prefix("//?/").unwrap_or(&game_s).to_string();

    // Set package.path: game first (shadows mudlib), then mudlib
    let path_setup = format!(
        "package.path = \"{game}/?.lua;{game}/?/init.lua;{mudlib}/?.lua;{mudlib}/?/init.lua;\" .. package.path",
        game = game_s,
        mudlib = mudlib_s,
    );
    lua.load(path_setup.as_str()).exec().expect("set package.path");

    // ── Stub efuns ───────────────────────────────────────────────────────
    // Only the minimal set needed so modules load without erroring.
    let globals = lua.globals();

    // log(level, msg)  — silent no-op
    let log_fn = lua.create_function(|_, (_level, _msg): (String, String)| {
        Ok(())
    }).unwrap();
    globals.set("log", log_fn).unwrap();

    // send(session_id, text) — no-op
    let send_fn = lua.create_function(|_, (_sid, _text): (String, String)| {
        Ok(())
    }).unwrap();
    globals.set("send", send_fn).unwrap();

    // send_prompt(session_id, text) — no-op
    let prompt_fn = lua.create_function(|_, (_sid, _text): (String, String)| {
        Ok(())
    }).unwrap();
    globals.set("send_prompt", prompt_fn).unwrap();

    // get_session(session_id) — returns nil (no sessions in tests)
    let gs_fn = lua.create_function(|_, _sid: String| {
        Ok(mlua::Value::Nil)
    }).unwrap();
    globals.set("get_session", gs_fn).unwrap();

    // get_character(char_id) — returns nil
    let gc_fn = lua.create_function(|_, _id: i64| {
        Ok(mlua::Value::Nil)
    }).unwrap();
    globals.set("get_character", gc_fn).unwrap();

    // set_object_state / get_object_state — backed by a Lua table
    lua.load(r#"
        _object_state_store = {}
        function set_object_state(id, key, value)
            _object_state_store[id] = _object_state_store[id] or {}
            _object_state_store[id][key] = value
        end
        function get_object_state(id, key)
            if _object_state_store[id] then
                return _object_state_store[id][key]
            end
            return nil
        end
    "#).exec().expect("stub object state");

    // has_permission — always true in tests
    let hp_fn = lua.create_function(|_, (_sid, _perm): (String, String)| {
        Ok(true)
    }).unwrap();
    globals.set("has_permission", hp_fn).unwrap();

    // config(key) — returns nil
    let cfg_fn = lua.create_function(|_, _key: String| {
        Ok(mlua::Value::Nil)
    }).unwrap();
    globals.set("config", cfg_fn).unwrap();

    // os_time() — returns 0
    let ot_fn = lua.create_function(|_, ()| Ok(0i64)).unwrap();
    globals.set("os_time", ot_fn).unwrap();

    // os_date(fmt) — returns fixed string
    let od_fn = lua.create_function(|_, _fmt: String| {
        Ok("2026-01-01 00:00:00".to_string())
    }).unwrap();
    globals.set("os_date", od_fn).unwrap();

    // write_file / read_file — no-ops for codegen tests
    // (codegen tests that need real I/O will set these up themselves)
    let wf_fn = lua.create_function(|_, (_path, _content): (String, String)| {
        Ok(true)
    }).unwrap();
    globals.set("write_file", wf_fn).unwrap();

    let rf_fn = lua.create_function(|_, _path: String| -> LuaResult<mlua::Value> {
        Ok(mlua::Value::Nil)
    }).unwrap();
    globals.set("read_file", rf_fn).unwrap();

    // set_persistent / get_persistent — the VM global that survives hot reload.
    // Backed by a real table here for the same reason object state is: the
    // daemons that use it store their whole world behind it, and a no-op stub
    // would make every one of them look stateless.
    lua.load(r#"
        _persistent_store = {}
        function set_persistent(key, value) _persistent_store[key] = value end
        function get_persistent(key) return _persistent_store[key] end
    "#).exec().expect("stub persistent store");

    // server_info() — only `uptime_secs` matters here; it is the monotonic
    // clock cache_d schedules on. Frozen, like os_time, so a test drives it.
    lua.load(r#"
        _uptime = 0
        function server_info() return { uptime_secs = _uptime, name = "TestMUD" } end
    "#).exec().expect("stub server_info");

    // Empty DAEMON table
    lua.load("DAEMON = {}").exec().expect("init DAEMON");

    lua
}

/// A VM with the daemons this plan added already loaded into `DAEMON`.
///
/// `cache_d` is the substrate for `cooldown_d` and `effect_d`, so almost every
/// test below needs the same four lines. Note there is deliberately no `db_*`
/// stub: everything reachable from here is the in-memory half, and the write
/// path is tested against the real store in `tests/state_cache.rs`. A stubbed
/// `db_put` here would prove only that a function was called.
fn make_daemon_lua() -> Lua {
    let lua = make_test_lua();
    lua.load(r#"
        -- `db_get` returns nil, which is not a lie: there is no database here,
        -- so every document really is absent. `db_put` and `db_delete` are
        -- deliberately left undefined — a stub that reported a successful write
        -- is exactly the kind of thing that keeps a suite green while the real
        -- path is broken. What reaches disk is tested in tests/state_cache.rs
        -- against the real store.
        function db_get() return nil end

        DAEMON.ticker = { every = function() end, after = function() end,
                          remove = function() end }
        DAEMON.cache    = require('daemons.cache_d')
        DAEMON.cooldown = require('daemons.cooldown_d')
        DAEMON.trait    = require('daemons.trait_d')
        DAEMON.effect   = require('daemons.effect_d')
    "#).exec().expect("load the state daemons");
    lua
}

/// Move both clocks. `os_time` drives expiry, `server_info().uptime_secs`
/// drives flush scheduling, and a test that only moved one would be testing a
/// world that cannot happen.
fn set_time(lua: &Lua, seconds: i64) {
    lua.load(format!(
        "_now = {seconds} _uptime = {seconds} function os_time() return _now end"
    ).as_str())
    .exec()
    .expect("set time");
}

/// Helper: run a Lua snippet and return a boolean result.
fn eval_bool(lua: &Lua, code: &str) -> bool {
    lua.load(code).eval::<bool>().unwrap()
}

/// Helper: run a Lua snippet and return a string result.
fn eval_str(lua: &Lua, code: &str) -> String {
    lua.load(code).eval::<String>().unwrap()
}

/// Helper: run a Lua snippet and return an integer result.
fn eval_int(lua: &Lua, code: &str) -> i64 {
    lua.load(code).eval::<i64>().unwrap()
}


// ═══════════════════════════════════════════════════════════════════════════════
//  Object tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_object_resolve_string() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    assert_eq!(eval_str(&lua, r#"return Object.resolve("hello", {})"#), "hello");
}

#[test]
fn test_object_resolve_nil() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    assert!(eval_bool(&lua, "return Object.resolve(nil, {}) == nil"));
}

#[test]
fn test_object_resolve_function() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    assert_eq!(
        eval_str(&lua, r#"return Object.resolve(function() return "dynamic" end, {})"#),
        "dynamic"
    );
}

#[test]
fn test_object_resolve_function_receives_obj() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    assert_eq!(
        eval_str(&lua, r#"
            local obj = { name = "test" }
            return Object.resolve(function(o) return o.name end, obj)
        "#),
        "test"
    );
}

#[test]
fn test_object_resolve_erroring_function() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    assert_eq!(
        eval_str(&lua, r#"return Object.resolve(function() error("boom") end, {})"#),
        "<invalid lfun return>"
    );
}

#[test]
fn test_object_resolve_number_returns_invalid() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    assert_eq!(
        eval_str(&lua, "return Object.resolve(42, {})"),
        "<invalid lfun return>"
    );
}

#[test]
fn test_object_new_defaults() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    lua.load(r#"obj = Object:new({ id = "test.obj" })"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return obj.id"), "test.obj");
    assert_eq!(eval_str(&lua, "return obj.short"), "Something");
    assert_eq!(eval_str(&lua, "return obj.description"), "You see nothing special.");
}

#[test]
fn test_object_new_custom_fields() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    lua.load(r#"obj = Object:new({ id = "x", short = "A sword", description = "Shiny" })"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return obj.short"), "A sword");
    assert_eq!(eval_str(&lua, "return obj.description"), "Shiny");
}

#[test]
fn test_object_state_roundtrip() {
    let lua = make_test_lua();
    lua.load("Object = require('lib.object')").exec().unwrap();
    lua.load(r#"obj = Object:new({ id = "test.state" })"#).exec().unwrap();
    lua.load(r#"obj:set_state("visited", true)"#).exec().unwrap();
    assert!(eval_bool(&lua, r#"return obj:get_state("visited") == true"#));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Item tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_item_defaults() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load(r#"item = Item:new({ id = "test.gem" })"#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return item.weight"), 1);
    assert_eq!(eval_int(&lua, "return item.value"), 0);
    assert!(!eval_bool(&lua, "return item.stackable"));
    assert_eq!(eval_int(&lua, "return item.quantity"), 1);
    assert!(!eval_bool(&lua, "return item:is_equippable()"));
    assert!(!eval_bool(&lua, "return item:is_stackable()"));
}

#[test]
fn test_item_equippable_from_slot() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load(r#"item = Item:new({ id = "test.helm", slot = "head" })"#).exec().unwrap();

    assert!(eval_bool(&lua, "return item:is_equippable()"));
    assert_eq!(eval_str(&lua, "return item.slot"), "head");
}

#[test]
fn test_item_stackable_display_name() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load(r#"item = Item:new({ id = "t", short = "Arrow", stackable = true, quantity = 20 })"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return item:display_name()"), "Arrow (x20)");
}

#[test]
fn test_item_non_stackable_display_name() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load(r#"item = Item:new({ id = "t", short = "Sword" })"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return item:display_name()"), "Sword");
}

#[test]
fn test_item_has_tag() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load(r#"item = Item:new({ id = "t", tags = {"quest", "fragile"} })"#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return item:has_tag("quest")"#));
    assert!(eval_bool(&lua, r#"return item:has_tag("fragile")"#));
    assert!(!eval_bool(&lua, r#"return item:has_tag("junk")"#));
}

#[test]
fn test_item_examine_includes_value_and_slot() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load(r#"item = Item:new({ id = "t", short = "Helm", value = 50, slot = "head" })"#).exec().unwrap();

    let text = eval_str(&lua, "return item:examine()");
    assert!(text.contains("Helm"), "examine should contain short name");
    assert!(text.contains("50 coins"), "examine should contain value");
    assert!(text.contains("head"), "examine should contain slot");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Mobile tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_mobile_default_stats() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "test.goblin" })"#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return mob.stats.hp"), 10);
    assert_eq!(eval_int(&lua, "return mob.stats.max_hp"), 10);
    assert_eq!(eval_int(&lua, "return mob.stats.level"), 1);
    assert!(eval_bool(&lua, "return mob:is_alive()"));
}

#[test]
fn test_mobile_custom_stats() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t", stats = { hp = 50, max_hp = 50, mp = 20, max_mp = 20 } })"#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return mob.stats.hp"), 50);
    assert_eq!(eval_int(&lua, "return mob.stats.mp"), 20);
}

#[test]
fn test_mobile_take_damage_clamps_to_zero() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t", stats = { hp = 10, max_hp = 10 } })"#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return mob:take_damage(5)"), 5);
    assert_eq!(eval_int(&lua, "return mob:take_damage(100)"), 0);
    assert!(!eval_bool(&lua, "return mob:is_alive()"));
}

#[test]
fn test_mobile_heal_clamps_to_max() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t", stats = { hp = 5, max_hp = 10 } })"#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return mob:heal(3)"), 8);
    assert_eq!(eval_int(&lua, "return mob:heal(999)"), 10);
}

#[test]
fn test_mobile_inventory_ops() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t" })"#).exec().unwrap();

    assert!(!eval_bool(&lua, r#"return mob:has_item("sword")"#));

    lua.load(r#"mob:add_item("sword")"#).exec().unwrap();
    assert!(eval_bool(&lua, r#"return mob:has_item("sword")"#));

    assert!(eval_bool(&lua, r#"return mob:remove_item("sword")"#));
    assert!(!eval_bool(&lua, r#"return mob:has_item("sword")"#));

    // Remove non-existent item returns false
    assert!(!eval_bool(&lua, r#"return mob:remove_item("shield")"#));
}

/// A skill is a trait now, not a parallel map. Without TRAIT_D loaded,
/// `:trait()` falls back to the stored number — which is what a bare Mobile
/// outside the engine sees, and is exactly how a skill is stored.
#[test]
fn test_mobile_skills_are_traits() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t" })"#).exec().unwrap();

    // Never learned reads as zero, and stores nothing.
    assert_eq!(eval_int(&lua, r#"return mob:trait("swords")"#), 0);
    assert!(eval_bool(&lua, r#"return mob.stats.swords == nil"#));

    lua.load(r#"mob.stats.swords = 5"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return mob:trait("swords")"#), 5);

    // The old parallel map is gone, not deprecated.
    assert!(eval_bool(&lua, "return mob.get_skill == nil"));
    assert!(eval_bool(&lua, "return mob.skills == nil"));
}

#[test]
fn test_mobile_tags() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t", tags = {"boss", "undead"} })"#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return mob:has_tag("boss")"#));
    assert!(!eval_bool(&lua, r#"return mob:has_tag("merchant")"#));
}

#[test]
fn test_mobile_dialogue() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({
        id = "t",
        dialogue = {
            greet = "Hello, adventurer!",
            quest = function(self) return "Level " .. self.stats.level .. " quest" end,
        }
    })"#).exec().unwrap();

    assert_eq!(eval_str(&lua, r#"return mob:get_dialogue("greet")"#), "Hello, adventurer!");
    assert_eq!(eval_str(&lua, r#"return mob:get_dialogue("quest")"#), "Level 1 quest");
    assert!(eval_bool(&lua, r#"return mob:get_dialogue("unknown") == nil"#));
}

#[test]
fn test_mobile_examine_format() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({
        id = "t", short = "Goblin", description = "A small green creature.",
        race = "goblin", faction = "horde", stats = { level = 5 }
    })"#).exec().unwrap();

    let text = eval_str(&lua, "return mob:examine()");
    assert!(text.contains("Goblin"));
    assert!(text.contains("A small green creature."));
    assert!(text.contains("Race: goblin"));
    assert!(text.contains("Faction: horde"));
    assert!(text.contains("Level: 5"));
}

#[test]
fn test_mobile_is_aggressive() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();

    lua.load(r#"mob_passive = Mobile:new({ id = "p" })"#).exec().unwrap();
    assert!(!eval_bool(&lua, "return mob_passive:is_aggressive()"));

    lua.load(r#"mob_angry = Mobile:new({ id = "a", aggressive = true })"#).exec().unwrap();
    assert!(eval_bool(&lua, "return mob_angry:is_aggressive()"));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Player tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_player_from_save_defaults() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        player = Player:from_save(1, { id = 1, name = "Hero", account_id = 1 }, {})
    "#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return player.name"), "Hero");
    assert_eq!(eval_int(&lua, "return player.stats.hp"), 100);
    assert_eq!(eval_int(&lua, "return player.stats.max_hp"), 100);
    assert_eq!(eval_int(&lua, "return player.stats.mp"), 50);
    assert_eq!(eval_int(&lua, "return player.gold"), 0);
    assert_eq!(eval_int(&lua, "return player.xp"), 0);
    assert_eq!(eval_int(&lua, "return player.stats.level"), 1);
}

#[test]
fn test_player_from_save_with_data() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        player = Player:from_save(42, { id = 42, name = "Merlin", account_id = 1 }, {
            stats = { hp = 80, max_hp = 100, mp = 30, max_mp = 50, level = 5 },
            gold = 250,
            xp = 1200,
            skills = { magic = 10, swords = 3 },
            quest_flags = { dragon_slain = true },
        })
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return player.stats.hp"), 80);
    assert_eq!(eval_int(&lua, "return player.stats.level"), 5);
    assert_eq!(eval_int(&lua, "return player.gold"), 250);
    assert_eq!(eval_int(&lua, "return player.xp"), 1200);
    // A blob written before skills were traits migrates into `stats`, where
    // storage decides presence. Nothing is lost; the home moved.
    assert_eq!(eval_int(&lua, "return player.stats.magic"), 10);
    assert_eq!(eval_int(&lua, "return player.stats.swords"), 3);
    assert!(eval_bool(&lua, "return player.skills == nil"));
    assert!(eval_bool(&lua, r#"return player:has_quest_flag("dragon_slain")"#));
}

/// The migration must not walk a character backwards. Once a skill has been
/// saved as a trait, a stale `skills` table left in the same blob loses.
#[test]
fn test_player_from_save_skill_migration_prefers_stats() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        player = Player:from_save(1, { id = 1, name = "X", account_id = 1 }, {
            stats  = { archery = 12 },
            skills = { archery = 7, tracking = 4 },
        })
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return player.stats.archery"), 12);
    assert_eq!(eval_int(&lua, "return player.stats.tracking"), 4);
}

#[test]
fn test_player_to_save_roundtrip() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        local saved = {
            stats = { hp = 75, max_hp = 100 },
            gold = 100,
            xp = 500,
            skills = { archery = 7 },
        }
        player = Player:from_save(1, { id = 1, name = "Archer", account_id = 1 }, saved)
        exported = player:to_save()
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return exported.stats.hp"), 75);
    assert_eq!(eval_int(&lua, "return exported.gold"), 100);
    assert_eq!(eval_int(&lua, "return exported.xp"), 500);
    // A skill round-trips through save/load after `Mobile.skills` was deleted:
    // in under the old key, out under `stats`, and `skills` no longer written.
    assert_eq!(eval_int(&lua, "return exported.stats.archery"), 7);
    assert!(eval_bool(&lua, "return exported.skills == nil"));
}

#[test]
fn test_player_to_save_deep_copy() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        player = Player:from_save(1, { id = 1, name = "X", account_id = 1 }, {})
        exported = player:to_save()
        -- Mutate the export; should NOT affect the player
        exported.stats.hp = 999
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return player.stats.hp"), 100);
    assert_eq!(eval_int(&lua, "return exported.stats.hp"), 999);
}

#[test]
fn test_player_gold_operations() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        player = Player:from_save(1, { id = 1, name = "X", account_id = 1 }, { gold = 100 })
    "#).exec().unwrap();

    lua.load("player:award_gold(50)").exec().unwrap();
    assert_eq!(eval_int(&lua, "return player.gold"), 150);

    assert!(eval_bool(&lua, "return player:spend_gold(100)"));
    assert_eq!(eval_int(&lua, "return player.gold"), 50);

    // Can't spend more than you have
    assert!(!eval_bool(&lua, "return player:spend_gold(200)"));
    assert_eq!(eval_int(&lua, "return player.gold"), 50);
}

#[test]
fn test_player_quest_flags() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        player = Player:from_save(1, { id = 1, name = "X", account_id = 1 }, {})
    "#).exec().unwrap();

    assert!(!eval_bool(&lua, r#"return player:has_quest_flag("intro_done")"#));

    lua.load(r#"player:set_quest_flag("intro_done")"#).exec().unwrap();
    assert!(eval_bool(&lua, r#"return player:has_quest_flag("intro_done")"#));
    assert!(eval_bool(&lua, r#"return player:get_quest_flag("intro_done") == true"#));
}

#[test]
fn test_player_display_name() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();

    // Without title
    lua.load(r#"p1 = Player:from_save(1, { id = 1, name = "Gandalf", account_id = 1 }, {})"#).exec().unwrap();
    // title defaults to name, so display_name returns just the name
    assert_eq!(eval_str(&lua, "return p1:display_name()"), "Gandalf");

    // With custom title
    lua.load(r#"p2 = Player:from_save(2, { id = 2, name = "Gandalf", account_id = 1 }, { title = "the Grey" })"#).exec().unwrap();
    assert_eq!(eval_str(&lua, "return p2:display_name()"), "Gandalf the Grey");
}

#[test]
fn test_player_inherits_mobile_methods() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        player = Player:from_save(1, { id = 1, name = "X", account_id = 1 }, {})
    "#).exec().unwrap();

    // Should have Mobile methods
    assert!(eval_bool(&lua, "return player:is_alive()"));
    assert_eq!(eval_int(&lua, "return player:take_damage(30)"), 70);
    assert_eq!(eval_int(&lua, "return player:heal(10)"), 80);

    // And inventory
    lua.load(r#"player:add_item("potion")"#).exec().unwrap();
    assert!(eval_bool(&lua, r#"return player:has_item("potion")"#));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Room tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_room_defaults() {
    let lua = make_test_lua();
    lua.load("Room = require('lib.room')").exec().unwrap();
    lua.load(r#"room = Room:new({ id = "test.room" })"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return room.id"), "test.room");
    assert_eq!(eval_int(&lua, "return room.light_level"), 2);
}

#[test]
fn test_room_exits() {
    let lua = make_test_lua();
    lua.load("Room = require('lib.room')").exec().unwrap();
    lua.load(r#"room = Room:new({
        id = "t", exits = { north = "other.room", south = "another.room" }
    })"#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return room:has_exit("north")"#));
    assert!(eval_bool(&lua, r#"return room:has_exit("south")"#));
    assert!(!eval_bool(&lua, r#"return room:has_exit("east")"#));
    assert_eq!(eval_str(&lua, r#"return room:get_exit("north")"#), "other.room");
}

#[test]
fn test_room_characters() {
    let lua = make_test_lua();
    lua.load("Room = require('lib.room')").exec().unwrap();
    lua.load(r#"room = Room:new({ id = "t" })"#).exec().unwrap();

    // Add characters
    lua.load("room:add_character(1)").exec().unwrap();
    lua.load("room:add_character(2)").exec().unwrap();
    assert_eq!(eval_int(&lua, "return #room:get_characters()"), 2);

    // No duplicates
    lua.load("room:add_character(1)").exec().unwrap();
    assert_eq!(eval_int(&lua, "return #room:get_characters()"), 2);

    // Remove
    lua.load("room:remove_character(1)").exec().unwrap();
    assert_eq!(eval_int(&lua, "return #room:get_characters()"), 1);
}

#[test]
fn test_room_actions() {
    let lua = make_test_lua();
    lua.load("Room = require('lib.room')").exec().unwrap();
    lua.load(r#"
        room = Room:new({ id = "t" })
        room:add_action("search", function() end, "search the area")
    "#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return room:get_action("search") ~= nil"#));
    assert!(eval_bool(&lua, r#"return room:get_action("nonexistent") == nil"#));
    assert_eq!(eval_int(&lua, "return #room:get_action_hints()"), 1);
}

#[test]
fn test_room_items() {
    let lua = make_test_lua();
    lua.load("Room = require('lib.room')").exec().unwrap();
    lua.load(r#"
        room = Room:new({ id = "t" })
        room:add_item("statue", "A weathered stone statue.")
    "#).exec().unwrap();

    assert_eq!(eval_str(&lua, r#"return room:get_item("statue")"#), "A weathered stone statue.");
    assert!(eval_bool(&lua, r#"return room:get_item("nothing") == nil"#));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ROOM_D tests (builder + from_data)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_room_d_from_data() {
    let lua = make_test_lua();
    lua.load("ROOM_D = require('daemons.room_d')").exec().unwrap();
    lua.load(r#"
        room = ROOM_D.from_data({
            id = "area.room1",
            short = "The Room",
            description = "A test room.",
            exits = { north = "area.room2" },
            items = { table = "A wooden table." },
        })
    "#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return room.id"), "area.room1");
    assert_eq!(eval_str(&lua, "return room.short"), "The Room");
    assert!(eval_bool(&lua, r#"return room:has_exit("north")"#));
    assert_eq!(eval_str(&lua, r#"return room:get_item("table")"#), "A wooden table.");
}

#[test]
fn test_room_d_from_data_missing_id() {
    let lua = make_test_lua();
    lua.load("ROOM_D = require('daemons.room_d')").exec().unwrap();

    assert!(eval_bool(&lua, r#"return ROOM_D.from_data({ short = "No ID" }) == nil"#));
}

#[test]
fn test_room_d_builder() {
    let lua = make_test_lua();
    lua.load("ROOM_D = require('daemons.room_d')").exec().unwrap();
    lua.load(r#"
        room = ROOM_D.create("test.builder_room")
            :set_short("Builder Room")
            :set_description("Built via the builder API.")
            :set_light(3)
            :set_smell("Fresh paint.")
            :add_exit("north", "test.other")
            :add_item("sign", "A freshly painted sign.")
            :finish()
    "#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return room.id"), "test.builder_room");
    assert_eq!(eval_str(&lua, "return room.short"), "Builder Room");
    assert_eq!(eval_int(&lua, "return room.light_level"), 3);
    assert!(eval_bool(&lua, r#"return room:has_exit("north")"#));
}

#[test]
fn test_room_d_load_area_and_merge() {
    let lua = make_test_lua();
    lua.load("ROOM_D = require('daemons.room_d')").exec().unwrap();
    // Stub DAEMON.world.set_area_meta
    lua.load(r#"
        DAEMON.world = { set_area_meta = function() end }
    "#).exec().unwrap();
    lua.load(r#"
        local area = {
            _meta = { name = "test_area", title = "Test Area" },
            { id = "test_area.r1", short = "Room 1" },
            { id = "test_area.r2", short = "Room 2" },
        }
        rooms = ROOM_D.load_area(area)
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return #rooms"), 2);
    assert_eq!(eval_str(&lua, "return rooms[1].id"), "test_area.r1");
}

// Codegen tests moved to `tests/codegen.rs`.
//
// They asked `make_test_lua` — a bare `mlua::Lua` with hand-stubbed globals —
// what a *rewritten* `codegen_d` produced. That module now depends on the file
// jail, the schema index and `list_dir`, none of which a stub reproduces, so the
// questions worth asking about it can only be asked of the real VM. This is the
// pattern `CLAUDE.md`'s testing section is about.

// ═══════════════════════════════════════════════════════════════════════════════
//  Inheritance chain verification
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_inheritance_chain_object_to_item() {
    let lua = make_test_lua();
    lua.load(r#"
        Object = require('lib.object')
        Item   = require('lib.item')
        item   = Item:new({ id = "t" })
    "#).exec().unwrap();

    // Item inherits Object methods
    assert!(eval_bool(&lua, "return item.get_state ~= nil"));
    assert!(eval_bool(&lua, "return item.set_state ~= nil"));
    // Item has its own methods
    assert!(eval_bool(&lua, "return item.is_equippable ~= nil"));
    assert!(eval_bool(&lua, "return item.display_name ~= nil"));
}

#[test]
fn test_inheritance_chain_mobile_to_player() {
    let lua = make_test_lua();
    lua.load(r#"
        Mobile = require('lib.mobile')
        Player = require('lib.player')
        player = Player:from_save(1, { id = 1, name = "X", account_id = 1 }, {})
    "#).exec().unwrap();

    // Player inherits Mobile methods
    assert!(eval_bool(&lua, "return player.take_damage ~= nil"));
    assert!(eval_bool(&lua, "return player.heal ~= nil"));
    assert!(eval_bool(&lua, "return player.is_alive ~= nil"));
    assert!(eval_bool(&lua, "return player.has_item ~= nil"));

    // Player has its own methods
    assert!(eval_bool(&lua, "return player.to_save ~= nil"));
    assert!(eval_bool(&lua, "return player.award_xp ~= nil"));
    assert!(eval_bool(&lua, "return player.spend_gold ~= nil"));

    // Player inherits Object methods too — including `trait`, which moved up
    // from Mobile when a trait stopped meaning "a character statistic".
    assert!(eval_bool(&lua, "return player.get_state ~= nil"));
    assert!(eval_bool(&lua, "return player.trait ~= nil"));
    assert!(eval_bool(&lua, "return player.has_trait ~= nil"));
}

/// `trait` is on Object, so a room and an item answer it too. That is the
/// point of the move: durability and corruption are traits in the same sense
/// strength is.
#[test]
fn test_trait_accessor_is_on_object_not_mobile() {
    let lua = make_test_lua();
    lua.load(r#"
        Object = require('lib.object')
        Item   = require('lib.item')
        Room   = require('lib.room')
        item   = Item:new({ id = "sword", stats = { durability = 40 } })
        room   = Room:new({ id = "a.b" })
    "#).exec().unwrap();

    assert!(eval_bool(&lua, "return Object.trait ~= nil"));
    assert_eq!(eval_int(&lua, r#"return item:trait("durability")"#), 40);
    // Absent reads as zero and stays absent — arithmetic is safe either way.
    assert_eq!(eval_int(&lua, r#"return item:trait("willpower")"#), 0);
    assert_eq!(eval_int(&lua, r#"return room:trait("corruption")"#), 0);

    // The old name is gone outright; a compatibility alias would be two
    // boundaries where the codebase deliberately keeps one.
    assert!(eval_bool(&lua, "return Object.stat == nil"));
    assert!(eval_bool(&lua, "return require('lib.mobile').stat == nil"));
}

#[test]
fn test_objdump_formats_an_inventory_of_instance_tables() {
    let lua = make_test_lua();
    lua.load("objdump = require('cmds.admin.objdump')").exec().unwrap();

    // The regression: entries are `{ template = "id" }` tables. Counting the
    // entry itself put a table in the output list, and table.concat raised —
    // so objdump threw for any player carrying anything.
    assert_eq!(
        eval_str(
            &lua,
            r#"return objdump._format_inventory({
                { template = "potion_red" },
                { template = "potion_red" },
                { template = "empty_vial" },
            })"#,
        ),
        "potion_red x2, empty_vial",
    );

    // Bare strings still arrive from older saves.
    assert_eq!(
        eval_str(&lua, r#"return objdump._format_inventory({ "rope", "rope", "rope" })"#),
        "rope x3",
    );

    // An instance with overrides still counts under its template.
    assert_eq!(
        eval_str(
            &lua,
            r#"return objdump._format_inventory({
                { template = "sword", enchant = "fire" },
                { template = "sword" },
            })"#,
        ),
        "sword x2",
    );

    assert_eq!(eval_str(&lua, "return objdump._format_inventory({})"), "(empty)");
    assert_eq!(eval_str(&lua, "return objdump._format_inventory(nil)"), "(empty)");
    // Junk in the list is skipped rather than crashing the dump.
    assert_eq!(
        eval_str(&lua, "return objdump._format_inventory({ {}, 42, { template = 'ok' } })"),
        "ok",
    );
}

#[test]
fn test_weapon_archetype_produces_an_item_with_a_component() {
    let lua = make_test_lua();
    lua.load(r#"
        Weapon = require('components.weapon')
        w = Weapon{ id = "t", short = "Sword", damage = 10, damage_type = "slash" }
    "#).exec().unwrap();

    // It is an Item — the archetype composes, it does not subclass.
    assert!(eval_bool(&lua, "return w.is_equippable ~= nil"));
    assert!(eval_bool(&lua, "return w.has_tag ~= nil"));
    assert!(eval_bool(&lua, "return w.get_state ~= nil"));

    // The weapon data lives in its own namespace, and its presence IS the
    // has-component test.
    assert!(eval_bool(&lua, "return Weapon.is(w)"));
    assert!(eval_bool(&lua, "return w.weapon ~= nil"));
    // A scalar damage becomes a min == max spread.
    assert_eq!(eval_int(&lua, "return w.weapon.min"), 10);
    assert_eq!(eval_int(&lua, "return w.weapon.max"), 10);
    assert_eq!(eval_str(&lua, "return w.weapon.damage_type"), "slash");
    assert_eq!(eval_str(&lua, "return w.slot"), "weapon");

    // Behaviour is not on the instance; it is in the system module.
    assert!(eval_bool(&lua, "return w.roll_damage == nil"));
    assert_eq!(eval_int(&lua, "return Weapon.roll_damage(w)"), 10);

    // The roll is injectable, which is what keeps combat deterministic.
    assert_eq!(
        eval_int(
            &lua,
            r#"
            local hi = Weapon{ id = "u", damage = {2, 8} }
            return Weapon.roll_damage(hi, function(n) return n end)
            "#,
        ),
        8,
    );

    // A plain Item is not a weapon, and the system says so rather than erroring.
    assert!(eval_bool(
        &lua,
        r#"
        local Item = require('lib.item')
        return Weapon.is(Item:new({ id = "rock" })) == false
           and Weapon.roll_damage(Item:new({ id = "rock" })) == nil
        "#
    ));
}

#[test]
fn test_armor_archetype_produces_an_item_with_a_component() {
    let lua = make_test_lua();
    lua.load(r#"
        Armor = require('components.armor')
        a = Armor{ id = "t", short = "Plate", defense = 15, slot = "chest",
                   resist = { fire = 5, ice = -3 } }
    "#).exec().unwrap();

    assert!(eval_bool(&lua, "return a.is_equippable ~= nil"));
    assert!(eval_bool(&lua, "return Armor.is(a)"));
    assert_eq!(eval_int(&lua, "return a.armour.defense"), 15);
    assert_eq!(eval_str(&lua, "return a.slot"), "chest");
    assert_eq!(eval_int(&lua, "return Armor.defense(a)"), 15);
    assert_eq!(eval_int(&lua, "return Armor.resist(a, 'fire')"), 5);
    assert_eq!(eval_int(&lua, "return Armor.resist(a, 'ice')"), -3);
    // Unmentioned damage types are zero, so a caller can add unconditionally.
    assert_eq!(eval_int(&lua, "return Armor.resist(a, 'acid')"), 0);
    assert_eq!(eval_int(&lua, "return Armor.encumbrance(a)"), 2);

    // `resist` reaches a player, so its order must not depend on `pairs`.
    assert_eq!(
        eval_str(
            &lua,
            r#"
            local lines = Armor.describe(a)
            for _, l in ipairs(lines) do
                if l:sub(1, 7) == "Resist:" then return l end
            end
            return "missing"
            "#,
        ),
        "Resist: fire +5, ice -3",
    );
}

#[test]
fn test_requires_is_shared_by_weapon_and_armor() {
    let lua = make_test_lua();
    lua.load(r#"
        Weapon   = require('components.weapon')
        Armor    = require('components.armor')
        Requires = require('components.requires')

        greatsword = Weapon{ id = "gs", damage = {8, 14}, required_strength = 16 }
        mail       = Armor{ id = "mail", slot = "chest", defense = 8,
                            required_level = 5, required_dexterity = 12 }
        dagger     = Weapon{ id = "dg", damage = {1, 4} }
    "#).exec().unwrap();

    // No requirements at all means the component is absent, not empty.
    assert!(eval_bool(&lua, "return dagger.requires == nil"));
    assert!(eval_bool(&lua, "return Requires.met(dagger, {})"));

    // Armor now gets the same three checks a Weapon does, and vice versa.
    assert!(eval_bool(&lua, "return greatsword.requires.strength == 16"));
    assert!(eval_bool(&lua, "return mail.requires.dexterity == 12"));

    assert!(eval_bool(&lua, "return Requires.met(greatsword, { strength = 16 })"));
    assert!(eval_bool(&lua, "return Requires.met(greatsword, { strength = 20 })"));
    assert!(eval_bool(&lua, "return Requires.met(greatsword, { strength = 15 }) == false"));
    assert_eq!(
        eval_str(&lua, "local _, why = Requires.met(greatsword, { strength = 15 }); return why"),
        "Requires 16 strength",
    );

    // Short on two counts reports the same one every time.
    assert_eq!(
        eval_str(&lua, "local _, why = Requires.met(mail, { level = 1, dexterity = 1 }); return why"),
        "Requires level 5",
    );

    // An entity answering :trait() is preferred over a stored stats table,
    // because the stored value is wrong for a buffed or derived trait.
    assert!(eval_bool(
        &lua,
        r#"
        local buffed = { stats = { strength = 10 },
                         trait = function(_, id) return id == "strength" and 18 or 0 end }
        return Requires.met(greatsword, buffed)
        "#
    ));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Command parser tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_commands_parse_basic() {
    let lua = make_test_lua();
    lua.load("commands = require('lib.commands')").exec().unwrap();
    lua.load(r#"verb, args_str, args = commands.parse("look north")"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return verb"), "look");
    assert_eq!(eval_str(&lua, "return args_str"), "north");
    assert_eq!(eval_int(&lua, "return #args"), 1);
}

#[test]
fn test_commands_parse_empty() {
    let lua = make_test_lua();
    lua.load("commands = require('lib.commands')").exec().unwrap();
    assert!(eval_bool(&lua, r#"
        local verb, args_str, args = commands.parse("")
        return verb == nil and args_str == "" and #args == 0
    "#));
}

#[test]
fn test_commands_parse_verb_only() {
    let lua = make_test_lua();
    lua.load("commands = require('lib.commands')").exec().unwrap();
    lua.load(r#"verb, args_str, args = commands.parse("quit")"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return verb"), "quit");
    assert_eq!(eval_str(&lua, "return args_str"), "");
    assert_eq!(eval_int(&lua, "return #args"), 0);
}

#[test]
fn test_commands_parse_multiple_args() {
    let lua = make_test_lua();
    lua.load("commands = require('lib.commands')").exec().unwrap();
    lua.load(r#"verb, args_str, args = commands.parse("say hello world friend")"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return verb"), "say");
    assert_eq!(eval_int(&lua, "return #args"), 3);
    assert_eq!(eval_str(&lua, "return args[1]"), "hello");
}

#[test]
fn test_commands_parse_case_insensitive_verb() {
    let lua = make_test_lua();
    lua.load("commands = require('lib.commands')").exec().unwrap();
    lua.load(r#"verb, _, _ = commands.parse("LOOK north")"#).exec().unwrap();

    assert_eq!(eval_str(&lua, "return verb"), "look");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Item Instance tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_mobile_item_instance_add_creates_table() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t" })"#).exec().unwrap();

    lua.load(r#"mob:add_item("sword")"#).exec().unwrap();

    // add_item should create { template = "sword" }
    assert!(eval_bool(&lua, r#"return type(mob.inventory[1]) == "table""#));
    assert_eq!(eval_str(&lua, r#"return mob.inventory[1].template"#), "sword");
}

#[test]
fn test_mobile_item_instance_has_item() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t" })"#).exec().unwrap();

    lua.load(r#"mob:add_item("potion")"#).exec().unwrap();
    assert!(eval_bool(&lua, r#"return mob:has_item("potion")"#));
    assert!(!eval_bool(&lua, r#"return mob:has_item("elixir")"#));
}

#[test]
fn test_mobile_item_instance_find_item() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t" })"#).exec().unwrap();

    lua.load(r#"mob:add_item("key")"#).exec().unwrap();

    // find_item returns entry table and index
    assert!(eval_bool(&lua, r#"
        local entry, idx = mob:find_item("key")
        return entry ~= nil and idx == 1 and entry.template == "key"
    "#));

    // find non-existent returns nil
    assert!(eval_bool(&lua, r#"
        local entry, idx = mob:find_item("nope")
        return entry == nil and idx == nil
    "#));
}

#[test]
fn test_mobile_item_instance_remove() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t" })"#).exec().unwrap();

    lua.load(r#"mob:add_item("gem")"#).exec().unwrap();
    lua.load(r#"mob:add_item("gem")"#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return #mob.inventory"), 2);

    // Remove first occurrence
    assert!(eval_bool(&lua, r#"return mob:remove_item("gem")"#));
    assert_eq!(eval_int(&lua, "return #mob.inventory"), 1);

    // Still has one
    assert!(eval_bool(&lua, r#"return mob:has_item("gem")"#));
}

#[test]
fn test_mobile_item_instance_add_instance() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t" })"#).exec().unwrap();

    lua.load(r#"mob:add_item_instance({ template = "sword", enchant = "fire" })"#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return mob:has_item("sword")"#));
    assert_eq!(eval_str(&lua, r#"return mob.inventory[1].enchant"#), "fire");
}

#[test]
fn test_mobile_item_legacy_string_compat() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    // Legacy: inventory with plain string entries
    lua.load(r#"mob = Mobile:new({ id = "t", inventory = {"old_item"} })"#).exec().unwrap();

    // has_item should still work with legacy strings
    assert!(eval_bool(&lua, r#"return mob:has_item("old_item")"#));
    assert!(eval_bool(&lua, r#"return mob:remove_item("old_item")"#));
    assert!(!eval_bool(&lua, r#"return mob:has_item("old_item")"#));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Color tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_color_colorize_basic_tags() {
    let lua = make_test_lua();
    lua.load("color = require('lib.color')").exec().unwrap();

    // {red} should become \27[31m
    let result = eval_str(&lua, r#"return color.colorize("{red}hello{/}")"#);
    assert!(result.contains("\x1b[31m"));
    assert!(result.contains("hello"));
    assert!(result.contains("\x1b[0m"));
}

#[test]
fn test_color_colorize_multiple_tags() {
    let lua = make_test_lua();
    lua.load("color = require('lib.color')").exec().unwrap();

    let result = eval_str(&lua, r#"return color.colorize("{bold}{cyan}test{/}")"#);
    assert!(result.contains("\x1b[1m"));
    assert!(result.contains("\x1b[36m"));
}

#[test]
fn test_color_colorize_unknown_tag_preserved() {
    let lua = make_test_lua();
    lua.load("color = require('lib.color')").exec().unwrap();

    let result = eval_str(&lua, r#"return color.colorize("{unknown}text")"#);
    assert!(result.contains("{unknown}"));
}

#[test]
fn test_color_strip_removes_all_tags() {
    let lua = make_test_lua();
    lua.load("color = require('lib.color')").exec().unwrap();

    let result = eval_str(&lua, r#"return color.strip("{red}hello {bold}world{/}")"#);
    assert_eq!(result, "hello world");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Checks tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_checks_has_item() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load("checks = require('lib.checks')").exec().unwrap();

    lua.load(r#"
        player = Mobile:new({ id = "p1" })
        player:add_item("key")
        check_key = checks.has_item("key")
        check_gem = checks.has_item("gem")
    "#).exec().unwrap();

    assert!(eval_bool(&lua, "return check_key(player)"));
    assert!(!eval_bool(&lua, "local ok = check_gem(player); return ok"));
}

#[test]
fn test_checks_has_level() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load("checks = require('lib.checks')").exec().unwrap();

    lua.load(r#"
        player = Mobile:new({ id = "p1", stats = { level = 5 } })
        check_low = checks.has_level(3)
        check_exact = checks.has_level(5)
        check_high = checks.has_level(10)
    "#).exec().unwrap();

    assert!(eval_bool(&lua, "return check_low(player)"));
    assert!(eval_bool(&lua, "return check_exact(player)"));
    assert!(!eval_bool(&lua, "local ok = check_high(player); return ok"));
}

#[test]
fn test_checks_all_composite() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load("checks = require('lib.checks')").exec().unwrap();

    lua.load(r#"
        player = Mobile:new({ id = "p1", stats = { level = 5 } })
        player:add_item("key")
        both = checks.all(checks.has_item("key"), checks.has_level(3))
        fails = checks.all(checks.has_item("key"), checks.has_level(10))
    "#).exec().unwrap();

    assert!(eval_bool(&lua, "return both(player)"));
    assert!(!eval_bool(&lua, "local ok = fails(player); return ok"));
}

#[test]
fn test_checks_any_composite() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load("checks = require('lib.checks')").exec().unwrap();

    lua.load(r#"
        player = Mobile:new({ id = "p1", stats = { level = 5 } })
        either = checks.any(checks.has_item("key"), checks.has_level(3))
        neither = checks.any(checks.has_item("gem"), checks.has_level(10))
    "#).exec().unwrap();

    assert!(eval_bool(&lua, "return either(player)"));
    assert!(!eval_bool(&lua, "local ok = neither(player); return ok"));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Exit system tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_room_exit_string_target() {
    let lua = make_test_lua();
    lua.load("Room = require('lib.room')").exec().unwrap();
    lua.load(r#"room = Room:new({ id = "test", exits = { north = "area.room2" } })"#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return room:has_exit("north")"#));
    assert_eq!(eval_str(&lua, r#"return room:get_exit("north")"#), "area.room2");
}

#[test]
fn test_room_exit_table_target() {
    let lua = make_test_lua();
    lua.load("Room = require('lib.room')").exec().unwrap();
    lua.load(r#"room = Room:new({ id = "test", exits = {
        north = { target = "area.room2", check_fail = "The door is locked." }
    }})"#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return room:has_exit("north")"#));
    assert_eq!(eval_str(&lua, r#"return room:get_exit("north")"#), "area.room2");
}

#[test]
fn test_room_exit_hidden_excluded_from_appearance() {
    let lua = make_test_lua();
    lua.load("Room = require('lib.room')").exec().unwrap();
    lua.load(r#"room = Room:new({
        id = "test", short = "Test Room", long = "A room.",
        exits = {
            north = "area.room2",
            secret = { target = "area.hidden", hidden = true }
        }
    })"#).exec().unwrap();

    // get_exit_info should return both
    assert!(eval_bool(&lua, r#"return room:get_exit_info("secret") ~= nil"#));

    // has_exit should still find hidden
    assert!(eval_bool(&lua, r#"return room:has_exit("secret")"#));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Death hook tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_mobile_take_damage_fires_death_hook() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"
        _death_fired = false
        mob = Mobile:new({ id = "m1", stats = { hp = 10, max_hp = 10 } })
        mob.on_death = function(self) _death_fired = true end
    "#).exec().unwrap();

    // Damage that doesn't kill shouldn't fire hook
    lua.load(r#"mob:take_damage(5)"#).exec().unwrap();
    assert!(!eval_bool(&lua, "return _death_fired"));

    // Damage that kills should fire hook
    lua.load(r#"mob:take_damage(5)"#).exec().unwrap();
    assert!(eval_bool(&lua, "return _death_fired"));
}

#[test]
fn test_mobile_take_damage_death_hook_only_fires_once() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"
        _death_count = 0
        mob = Mobile:new({ id = "m1", stats = { hp = 5, max_hp = 10 } })
        mob.on_death = function(self) _death_count = _death_count + 1 end
    "#).exec().unwrap();

    // Kill
    lua.load(r#"mob:take_damage(10)"#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return _death_count"), 1);

    // Already dead — shouldn't fire again
    lua.load(r#"mob:take_damage(10)"#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return _death_count"), 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Player inventory migration tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_player_from_save_migrates_string_inventory() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        p = Player:from_save(1, { id = 1, name = "Test", account_id = 1 },
            { inventory = { "old_sword", "old_shield" } })
    "#).exec().unwrap();

    // Both should be migrated to instance tables
    assert!(eval_bool(&lua, r#"return type(p.inventory[1]) == "table""#));
    assert_eq!(eval_str(&lua, r#"return p.inventory[1].template"#), "old_sword");
    assert_eq!(eval_str(&lua, r#"return p.inventory[2].template"#), "old_shield");
}

#[test]
fn test_player_from_save_preserves_instance_tables() {
    let lua = make_test_lua();
    lua.load("Player = require('lib.player')").exec().unwrap();
    lua.load(r#"
        p = Player:from_save(1, { id = 1, name = "Test", account_id = 1 },
            { inventory = { { template = "magic_sword", enchant = "fire" } } })
    "#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return type(p.inventory[1]) == "table""#));
    assert_eq!(eval_str(&lua, r#"return p.inventory[1].template"#), "magic_sword");
    assert_eq!(eval_str(&lua, r#"return p.inventory[1].enchant"#), "fire");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Item_d resolve tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_item_d_resolve_pristine() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load("item_d = require('daemons.item_d')").exec().unwrap();

    lua.load(r#"
        local sword = Item:new({ id = "iron_sword", short = "an iron sword", value = 10 })
        item_d.register(sword)
    "#).exec().unwrap();

    // Resolve a pristine instance (no overrides)
    assert!(eval_bool(&lua, r#"
        local entry = { template = "iron_sword" }
        local resolved = item_d.resolve(entry)
        return resolved ~= nil and resolved.short == "an iron sword" and resolved.value == 10
    "#));
}

#[test]
fn test_item_d_resolve_with_overrides() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load("item_d = require('daemons.item_d')").exec().unwrap();

    lua.load(r#"
        local sword = Item:new({ id = "iron_sword", short = "an iron sword", value = 10 })
        item_d.register(sword)
    "#).exec().unwrap();

    // Resolve with overrides — instance fields win
    assert!(eval_bool(&lua, r#"
        local entry = { template = "iron_sword", short = "a flaming sword", enchant = "fire" }
        local resolved = item_d.resolve(entry)
        return resolved.short == "a flaming sword" and resolved.value == 10 and resolved.enchant == "fire"
    "#));
}

#[test]
fn test_item_d_find_by_name_with_instances() {
    let lua = make_test_lua();
    lua.load("Item = require('lib.item')").exec().unwrap();
    lua.load("item_d = require('daemons.item_d')").exec().unwrap();

    lua.load(r#"
        item_d.register(Item:new({ id = "red_potion", short = "a red potion" }))
        item_d.register(Item:new({ id = "blue_gem", short = "a sparkling blue gem" }))
    "#).exec().unwrap();

    // Search with instance tables
    assert!(eval_bool(&lua, r#"
        local inv = { { template = "red_potion" }, { template = "blue_gem" } }
        local tmpl_id, item = item_d.find_by_name("red", inv)
        return tmpl_id == "red_potion" and item.short == "a red potion"
    "#));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Snoop_d tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_snoop_d_basic_operations() {
    let lua = make_test_lua();
    lua.load("snoop = require('daemons.snoop_d')").exec().unwrap();

    // Start snooping
    assert!(eval_bool(&lua, r#"
        local ok, _ = snoop.start("admin_sid", "player_sid")
        return ok
    "#));

    // Check state
    assert_eq!(eval_str(&lua, r#"return snoop.get_target("admin_sid")"#), "player_sid");
    assert!(eval_bool(&lua, r#"return snoop.is_snooped("player_sid")"#));
    assert!(!eval_bool(&lua, r#"return snoop.is_snooped("admin_sid")"#));

    // Stop
    assert!(eval_bool(&lua, r#"return snoop.stop("admin_sid")"#));
    assert!(!eval_bool(&lua, r#"return snoop.is_snooped("player_sid")"#));
}

#[test]
fn test_snoop_d_prevents_self_snoop() {
    let lua = make_test_lua();
    lua.load("snoop = require('daemons.snoop_d')").exec().unwrap();

    assert!(!eval_bool(&lua, r#"
        local ok, _ = snoop.start("sid1", "sid1")
        return ok
    "#));
}

#[test]
fn test_snoop_d_cleanup() {
    let lua = make_test_lua();
    lua.load("snoop = require('daemons.snoop_d')").exec().unwrap();

    lua.load(r#"snoop.start("admin1", "target1")"#).exec().unwrap();
    lua.load(r#"snoop.start("admin2", "target1")"#).exec().unwrap();

    // Cleanup target1 — should remove all snoops involving it
    lua.load(r#"snoop.cleanup("target1")"#).exec().unwrap();

    assert!(!eval_bool(&lua, r#"return snoop.is_snooped("target1")"#));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Pager_d tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_pager_d_basic_paging() {
    let lua = make_test_lua();
    lua.load("pager = require('daemons.pager_d')").exec().unwrap();

    // Create a long text
    lua.load(r#"
        local lines = {}
        for i = 1, 20 do lines[i] = "Line " .. i end
        long_text = table.concat(lines, "\r\n")
    "#).exec().unwrap();

    // Not paging yet
    assert!(!eval_bool(&lua, r#"return pager.is_paging("test_sid")"#));

    // Start paging with page length of 5
    lua.load(r#"pager.page("test_sid", long_text, 5)"#).exec().unwrap();

    // Should be paging now
    assert!(eval_bool(&lua, r#"return pager.is_paging("test_sid")"#));

    // Stop paging
    lua.load(r#"pager.stop("test_sid")"#).exec().unwrap();
    assert!(!eval_bool(&lua, r#"return pager.is_paging("test_sid")"#));
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Channel_d tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_channel_d_default_ooc() {
    let lua = make_test_lua();
    // Need stub for all_sessions
    lua.load(r#"function all_sessions() return {} end"#).exec().unwrap();
    lua.load("channel = require('daemons.channel_d')").exec().unwrap();

    // OOC channel should exist by default
    let channels = eval_int(&lua, r#"return #channel.list()"#);
    assert!(channels >= 1, "Expected at least 1 default channel (ooc)");
}

#[test]
fn test_channel_d_join_leave() {
    let lua = make_test_lua();
    lua.load(r#"function all_sessions() return {} end"#).exec().unwrap();
    lua.load("channel = require('daemons.channel_d')").exec().unwrap();

    // Join OOC
    assert!(eval_bool(&lua, r#"
        local ok, _ = channel.join("ooc", 42)
        return ok
    "#));

    assert!(eval_bool(&lua, r#"return channel.is_subscribed("ooc", 42)"#));

    // Leave
    assert!(eval_bool(&lua, r#"
        local ok, _ = channel.leave("ooc", 42)
        return ok
    "#));

    assert!(!eval_bool(&lua, r#"return channel.is_subscribed("ooc", 42)"#));
}

// ─── cmds/trace.lua ──────────────────────────────────────────────────────────

/// The trace command is a thin front-end over the Rust efuns, so the only Lua
/// logic worth testing is its argument parsing — which is why `parse_args` is
/// factored out of `execute`.
#[test]
fn test_trace_command_parse_args() {
    let lua = make_test_lua();
    lua.load("trace = require('cmds.admin.trace')").exec().unwrap();

    let check = |args: &str, want_sub: &str, want_count: &str, want_scope: &str| {
        let src = format!(
            "local s, c, sc = trace.parse_args({args})
             return s .. '|' .. tostring(c) .. '|' .. tostring(sc)"
        );
        let got: String = lua.load(src.as_str()).eval().unwrap();
        assert_eq!(got, format!("{want_sub}|{want_count}|{want_scope}"), "for args {args}");
    };

    check("{}", "status", "nil", "nil");
    check("{'ON'}", "on", "nil", "nil");
    check("{'lines', 'all'}", "lines", "nil", "all");
    check("{'show', '25'}", "show", "25", "nil");
    check("{'calls', 'all', '10'}", "calls", "10", "all");
    // Non-numeric junk should not be mistaken for a count.
    check("{'timings', 'wat'}", "timings", "nil", "nil");
}

#[test]
fn test_trace_command_exposes_the_standard_command_shape() {
    let lua = make_test_lua();
    lua.load("trace = require('cmds.admin.trace')").exec().unwrap();

    assert!(eval_bool(&lua, "return trace.name == 'trace'"));
    assert!(eval_bool(&lua, "return trace.category == 'admin'"));
    assert!(eval_bool(&lua, "return trace.permission == 'cmd.trace'"));
    assert!(eval_bool(&lua, "return type(trace.execute) == 'function'"));
}


// ═══════════════════════════════════════════════════════════════════════════════
//  jsonsafe — can this value survive the trip to the database?
//
//  These mirror rules that live in Rust (`lua_to_json`), so they are a
//  reimplementation and reimplementations drift. `tests/state_cache.rs` runs
//  the same values past the real encoder and demands the two agree; these are
//  the fast half of that pair.
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn jsonsafe_accepts_ordinary_data() {
    let lua = make_test_lua();
    lua.load("js = require('lib.jsonsafe')").exec().unwrap();
    assert!(eval_bool(&lua, r#"return js.check({ a = 1, b = "two", c = true })"#));
    assert!(eval_bool(&lua, r#"return js.check({ 1, 2, 3 })"#));
    assert!(eval_bool(&lua, r#"return js.check({ nested = { deep = { 1, 2 } } })"#));
    assert!(eval_bool(&lua, r#"return js.check(42)"#));
    assert!(eval_bool(&lua, r#"return js.check("hello")"#));
    assert!(eval_bool(&lua, r#"return js.check({})"#));
}

#[test]
fn jsonsafe_refuses_a_function_and_names_the_field() {
    let lua = make_test_lua();
    lua.load("js = require('lib.jsonsafe')").exec().unwrap();
    let why = eval_str(&lua, r#"
        local ok, why = js.check({ effect = { tick = function() end } })
        return (not ok) and why or "ACCEPTED"
    "#);
    assert!(why.contains("effect.tick"), "should name the field, got {why:?}");
    assert!(why.contains("function"), "should say what was wrong, got {why:?}");
}

#[test]
fn jsonsafe_refuses_a_table_that_is_both_a_list_and_a_map() {
    let lua = make_test_lua();
    lua.load("js = require('lib.jsonsafe')").exec().unwrap();
    let why = eval_str(&lua, r#"
        local ok, why = js.check({ 1, 2, name = "mixed" })
        return (not ok) and why or "ACCEPTED"
    "#);
    assert!(why.contains("'name'"), "should name the offending key, got {why:?}");
    assert!(why.contains("list"), "should explain the clash, got {why:?}");
}

#[test]
fn jsonsafe_refuses_infinities_and_nan() {
    let lua = make_test_lua();
    lua.load("js = require('lib.jsonsafe')").exec().unwrap();
    assert!(!eval_bool(&lua, "return (js.check({ x = 1/0 }))"));
    assert!(!eval_bool(&lua, "return (js.check({ x = -1/0 }))"));
    assert!(!eval_bool(&lua, "return (js.check({ x = 0/0 }))"));
}

#[test]
fn jsonsafe_refuses_a_boolean_key() {
    let lua = make_test_lua();
    lua.load("js = require('lib.jsonsafe')").exec().unwrap();
    assert!(!eval_bool(&lua, "return (js.check({ [true] = 1 }))"));
}

/// A table that refers to itself has no JSON form, and the driver catches it
/// with the depth limit rather than by tracking identity. So does this.
#[test]
fn jsonsafe_refuses_a_cycle_by_running_out_of_depth() {
    let lua = make_test_lua();
    lua.load("js = require('lib.jsonsafe')").exec().unwrap();
    let why = eval_str(&lua, r#"
        local t = {}
        t.self = t
        local ok, why = js.check(t)
        return (not ok) and why or "ACCEPTED"
    "#);
    assert!(why.contains("nesting is deeper"), "got {why:?}");
    assert!(why.contains("refers to itself"), "should hint at the cause, got {why:?}");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  lib/traits — dependency ordering and the regeneration settle
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn topo_sort_puts_dependencies_first() {
    let lua = make_test_lua();
    lua.load("tl = require('lib.traits')").exec().unwrap();
    let order = eval_str(&lua, r#"
        local order = tl.topo_sort({
            willpower = { depends = { "wisdom", "level" } },
            wisdom    = { depends = {} },
            level     = { depends = {} },
        })
        return table.concat(order, ",")
    "#);
    let pos = |id: &str| order.split(',').position(|s| s == id).unwrap();
    assert!(pos("wisdom") < pos("willpower"), "got {order}");
    assert!(pos("level") < pos("willpower"), "got {order}");
}

/// "There is a cycle in your 30 traits" is not something anyone can act on.
#[test]
fn topo_sort_reports_the_cycle_as_a_path() {
    let lua = make_test_lua();
    lua.load("tl = require('lib.traits')").exec().unwrap();
    let path = eval_str(&lua, r#"
        local order, cycle = tl.topo_sort({
            willpower = { depends = { "wisdom" } },
            wisdom    = { depends = { "insight" } },
            insight   = { depends = { "willpower" } },
        })
        if order then return "NO CYCLE FOUND" end
        return table.concat(cycle, " -> ")
    "#);
    assert!(path.contains("willpower"), "got {path}");
    assert!(path.contains("wisdom"), "got {path}");
    assert!(path.contains("insight"), "got {path}");
    // The path closes on itself, so the first name appears again at the end.
    let names: Vec<&str> = path.split(" -> ").collect();
    assert_eq!(names[0], names[names.len() - 1], "the path should close: {path}");
}

/// The table that pins the regeneration arithmetic. The two "nothing changed"
/// rows are the most important assertions in this file: a settle that always
/// reported a change would dirty every online player's state on every prompt
/// and undo the whole write-behind design.
#[test]
fn settle_earns_whole_units_and_carries_the_remainder() {
    let lua = make_test_lua();
    lua.load("tl = require('lib.traits')").exec().unwrap();

    // name, rate, per, cur, anchor, now, expected cur ("" = no change), expected anchor
    let cases: &[(&str, f64, f64, f64, f64, f64, &str, &str)] = &[
        ("nothing has elapsed",      1.0, 3.0, 40.0, 1000.0, 1000.0, "",    ""),
        ("less than one unit",       1.0, 3.0, 40.0, 1000.0, 1002.0, "",    ""),
        ("three units, one carried", 1.0, 3.0, 40.0, 1000.0, 1010.0, "43",  "1009"),
        ("a faster rate",            0.5, 1.0, 40.0, 1000.0, 1003.0, "41",  "1002"),
        ("clamped, re-anchored",     1.0, 3.0, 98.0, 1000.0, 1100.0, "100", "1100"),
        ("the clock stepped back",   1.0, 3.0, 40.0, 1000.0,  900.0, "40",  "900"),
    ];

    for (name, rate, per, cur, anchor, now, want_cur, want_anchor) in cases {
        let got = eval_str(&lua, &format!(r#"
            local c, a = tl.settle({cur}, {anchor}, {now}, {rate}, {per}, 100, 0, 100)
            if c == nil then return "" end
            return tostring(c) .. "/" .. tostring(a)
        "#));
        let want = if want_cur.is_empty() {
            String::new()
        } else {
            format!("{want_cur}/{want_anchor}")
        };
        assert_eq!(got, want, "settle case: {name}");
    }
}

/// A gauge sitting at its target must not accumulate credit while it waits.
/// Otherwise a player at full health banks an hour of regeneration and dumps
/// it the instant something hits them.
#[test]
fn settle_at_target_reports_no_change_however_long_it_waits() {
    let lua = make_test_lua();
    lua.load("tl = require('lib.traits')").exec().unwrap();
    assert!(eval_bool(&lua, r#"
        local c, a = tl.settle(100, 1000, 99999, 1, 3, 100, 0, 100)
        return c == nil and a == nil
    "#));
}

#[test]
fn round_modes_do_what_they_say() {
    let lua = make_test_lua();
    lua.load("tl = require('lib.traits')").exec().unwrap();
    assert_eq!(eval_int(&lua, "return tl.round(2.7, 'floor')"), 2);
    assert_eq!(eval_int(&lua, "return tl.round(2.2, 'ceil')"), 3);
    assert_eq!(eval_int(&lua, "return tl.round(2.5, 'round')"), 3);
    assert_eq!(eval_str(&lua, "return tostring(tl.round(2.7, 'none'))"), "2.7");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  lib/effects — phase ordering and the single fold
// ═══════════════════════════════════════════════════════════════════════════════

/// The worked example from the design: 30 damage, -15% and -5 flat.
/// Percentage first gives 20.5; flat first gives 21.25. Phases are what make
/// the answer independent of which effect happened to land first.
#[test]
fn a_percentage_applies_before_a_flat_reduction() {
    let lua = make_test_lua();
    lua.load("el = require('lib.effects')").exec().unwrap();
    let amount = eval_str(&lua, r#"
        local ev = { amount = 30, scale = 0, min = 0 }
        el.dispatch(ev, {
            -- deliberately registered flat-first, to prove registration order
            -- is not what decides the answer
            { phase = "reduce", def = "b", fn = function(e) e.amount = e.amount - 5 end },
            { phase = "mult",   def = "a", fn = function(e) e.scale = e.scale - 0.15 end },
        })
        return tostring(ev.amount)
    "#);
    assert_eq!(amount, "20.5", "expected 30*0.85-5, got {amount}");
    assert_ne!(amount, "21.25", "that is what phase-reversal would give");
}

#[test]
fn multipliers_in_the_same_phase_add_rather_than_compound() {
    let lua = make_test_lua();
    lua.load("el = require('lib.effects')").exec().unwrap();
    // Two +20% buffs: 140, not 144. Additive composition is what makes the
    // mult phase genuinely order-independent.
    assert_eq!(eval_str(&lua, r#"
        local ev = { amount = 100, scale = 0 }
        el.dispatch(ev, {
            { phase = "mult", def = "a", fn = function(e) e.scale = e.scale + 0.2 end },
            { phase = "mult", def = "b", fn = function(e) e.scale = e.scale + 0.2 end },
        })
        -- Compared as a number, not as its spelling: `100 * 1.4` is a float,
        -- and from Lua 5.3 on `tostring` renders that `140.0`. The pipeline
        -- floors at the call site (`mobile.lua`), so what matters here is the
        -- value.
        return tostring(ev.amount == 140)
    "#), "true");
}

#[test]
fn cancelling_stops_the_rest_of_the_chain() {
    let lua = make_test_lua();
    lua.load("el = require('lib.effects')").exec().unwrap();
    assert!(eval_bool(&lua, r#"
        local ran = false
        local ev = { amount = 30, scale = 0 }
        el.dispatch(ev, {
            { phase = "pre",    def = "a", fn = function(e)
                e.cancelled = true e.reason = "The flames wash over you harmlessly." end },
            { phase = "reduce", def = "b", fn = function() ran = true end },
        })
        return ev.cancelled and not ran and ev.reason ~= nil
    "#));
}

#[test]
fn ties_break_deterministically_by_definition_id() {
    let lua = make_test_lua();
    lua.load("el = require('lib.effects')").exec().unwrap();
    assert_eq!(eval_str(&lua, r#"
        local seen = {}
        local ev = { amount = 0, scale = 0 }
        el.dispatch(ev, {
            { phase = "add", def = "zeta",  fn = function() seen[#seen+1] = "zeta" end },
            { phase = "add", def = "alpha", fn = function() seen[#seen+1] = "alpha" end },
            { phase = "add", def = "mid",   fn = function() seen[#seen+1] = "mid" end },
        })
        return table.concat(seen, ",")
    "#), "alpha,mid,zeta");
}

/// The overwhelmingly common case: an entity with no effects at all. It must
/// come out exactly as it went in, and it must not allocate a new table.
#[test]
fn an_event_with_no_handlers_is_returned_untouched() {
    let lua = make_test_lua();
    lua.load("el = require('lib.effects')").exec().unwrap();
    assert!(eval_bool(&lua, r#"
        local ev = { amount = 30 }
        local out = el.dispatch(ev, {})
        return out == ev and out.amount == 30 and out.scale == nil
    "#));
}

#[test]
fn a_handler_that_raises_does_not_stop_the_others() {
    let lua = make_test_lua();
    lua.load("el = require('lib.effects')").exec().unwrap();
    assert_eq!(eval_str(&lua, r#"
        local errors = 0
        local ev = { amount = 10, scale = 0 }
        el.dispatch(ev, {
            { phase = "add", def = "bad",  fn = function() error("boom") end },
            { phase = "add", def = "good", fn = function(e) e.amount = e.amount + 5 end },
        }, function() errors = errors + 1 end)
        return tostring(ev.amount) .. "/" .. tostring(errors)
    "#), "15/1");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  The daemons load and register
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn the_state_daemons_load() {
    let lua = make_daemon_lua();
    assert!(eval_bool(&lua, "return type(DAEMON.cache.set) == 'function'"));
    assert!(eval_bool(&lua, "return type(DAEMON.cooldown.mark) == 'function'"));
    assert!(eval_bool(&lua, "return type(DAEMON.trait.value) == 'function'"));
    assert!(eval_bool(&lua, "return type(DAEMON.effect.run) == 'function'"));
}

#[test]
fn cooldown_d_registers_both_of_its_namespaces() {
    let lua = make_daemon_lua();
    assert_eq!(eval_str(&lua, r#"return DAEMON.cache.spec("cooldowns").tier"#), "write_through");
    assert_eq!(eval_str(&lua, r#"return DAEMON.cache.spec("cooldowns_fast").tier"#), "memory");
}


// ═══════════════════════════════════════════════════════════════════════════════
//  cache_d — the tiered store
//
//  Everything here goes through `_plan_flush`, which reports what *would* be
//  written without writing anything. That split is what lets scheduling,
//  ephemerality, pruning and budgeting be tested honestly with no database
//  stubs at all. The write itself is tested in tests/state_cache.rs.
// ═══════════════════════════════════════════════════════════════════════════════

/// A namespace fixture: `nsdef(kind)` defines "t" with the given tier.
fn cache_lua(spec: &str) -> Lua {
    let lua = make_daemon_lua();
    set_time(&lua, 1000);
    lua.load(&format!(r#"DAEMON.cache.define("t", {spec})"#)).exec().unwrap();
    lua
}

#[test]
fn a_value_comes_back_out_of_the_cache() {
    let lua = cache_lua(r#"{ tier = "write_behind" }"#);
    assert!(eval_bool(&lua, r#"return DAEMON.cache.set("t", 42, "gold", 1200)"#));
    assert_eq!(eval_int(&lua, r#"return DAEMON.cache.get("t", 42, "gold")"#), 1200);
    // The scope accepts a number or a string and lands in the same place.
    assert_eq!(eval_int(&lua, r#"return DAEMON.cache.get("t", "42", "gold")"#), 1200);
}

#[test]
fn an_unknown_namespace_is_refused_rather_than_invented() {
    let lua = cache_lua(r#"{ tier = "memory" }"#);
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("nope", 1, "x") == nil"#));
    assert!(!eval_bool(&lua, r#"return DAEMON.cache.set("nope", 1, "x", 1)"#));
}

/// A number key would come back from JSON as a string, silently changing the
/// shape of the document. Refusing costs one line and removes the whole class.
#[test]
fn a_number_key_is_refused() {
    let lua = cache_lua(r#"{ tier = "write_behind" }"#);
    assert!(!eval_bool(&lua, r#"return DAEMON.cache.set("t", 1, 7, "value")"#));
}

#[test]
fn a_value_that_could_not_be_written_is_refused_at_the_call_site() {
    let lua = cache_lua(r#"{ tier = "write_behind" }"#);
    assert!(!eval_bool(&lua, r#"return DAEMON.cache.set("t", 1, "bad", { f = function() end })"#));
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("t", 1, "bad") == nil"#),
        "a value that cannot be flushed must not be stored either — memory and \
         the database would disagree forever");
    assert_eq!(eval_int(&lua, "return DAEMON.cache.stats().rejected_writes"), 1);
}

/// The memory tier never serializes, so it may hold things JSON cannot: live
/// object references, functions, an aggro table pointing at real mobs.
#[test]
fn the_memory_tier_accepts_what_json_cannot() {
    let lua = cache_lua(r#"{ tier = "memory" }"#);
    assert!(eval_bool(&lua, r#"return DAEMON.cache.set("t", 1, "aggro", { fn = function() end })"#));
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("t", 1, "aggro") ~= nil"#));
}

#[test]
fn writing_dirties_a_scope_and_reading_does_not() {
    let lua = cache_lua(r#"{ tier = "write_behind" }"#);
    lua.load(r#"DAEMON.cache.set("t", 1, "a", 1)"#).exec().unwrap();
    assert!(eval_bool(&lua, r#"return DAEMON.cache.inspect("t", 1).dirty"#));

    lua.load(r#"DAEMON.cache._apply(DAEMON.cache._plan_flush(nil, { all = true }))"#).exec().unwrap();
    // No db_put here, so the write fails and the scope stays dirty — which is
    // itself the right behaviour, and is asserted in a_failed_write_is_retried.
    assert_eq!(eval_int(&lua, r#"return #DAEMON.cache.keys("t", 1)"#), 1);
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("t", 1, "a") == 1"#));
    assert!(!eval_bool(&lua, r#"return DAEMON.cache.inspect("t", 2) ~= nil"#));
}

#[test]
fn a_flush_plan_carries_the_whole_scope_as_one_document() {
    let lua = cache_lua(r#"{ tier = "write_behind", flush_seconds = 0 }"#);
    lua.load(r#"
        for i = 1, 5 do DAEMON.cache.set("t", 42, "k" .. i, i * 10) end
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, r#"
        local plan = DAEMON.cache._plan_flush(nil, { all = true })
        return #plan
    "#), 1, "five keys in one scope are one document write, not five");

    assert_eq!(eval_str(&lua, r#"
        local plan = DAEMON.cache._plan_flush(nil, { all = true })
        local n = 0
        for _ in pairs(plan[1].doc) do n = n + 1 end
        return plan[1].action .. "/" .. plan[1].id .. "/" .. n
    "#), "put/42/5");
}

#[test]
fn the_scope_prefix_becomes_the_document_id() {
    let lua = cache_lua(r#"{ tier = "write_behind", scope_prefix = "char:" }"#);
    lua.load(r#"DAEMON.cache.set("t", 42, "a", 1)"#).exec().unwrap();
    assert_eq!(eval_str(&lua, r#"
        return DAEMON.cache._plan_flush(nil, { all = true })[1].id
    "#), "char:42");
}

/// The requirement, generalised: an entry that would have expired before the
/// server came back is not worth writing. Not because writing is expensive —
/// because writing it would be wrong.
#[test]
fn an_entry_shorter_than_min_lifetime_is_never_written() {
    let lua = cache_lua(r#"{ tier = "write_behind", min_lifetime = 30 }"#);
    lua.load(r#"
        DAEMON.cache.set("t", 1, "brief", "gone soon", { expires_at = 1020 })
        DAEMON.cache.set("t", 1, "lasting", "still here", { expires_at = 5000 })
    "#).exec().unwrap();

    // Both are fully live in memory.
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("t", 1, "brief") ~= nil"#));

    assert_eq!(eval_str(&lua, r#"
        local plan = DAEMON.cache._plan_flush(nil, { all = true })
        local keys = {}
        for k in pairs(plan[1].doc) do keys[#keys+1] = k end
        table.sort(keys)
        return table.concat(keys, ",")
    "#), "lasting", "the short-lived entry must not reach the document");
}

/// And a scope holding nothing but ephemeral entries is not dirty at all —
/// otherwise a stream of short buffs would schedule a write every interval
/// that had nothing to say.
#[test]
fn ephemeral_entries_do_not_make_a_scope_dirty() {
    let lua = cache_lua(r#"{ tier = "write_behind", min_lifetime = 30 }"#);
    lua.load(r#"DAEMON.cache.set("t", 1, "brief", "x", { expires_at = 1010 })"#).exec().unwrap();
    assert!(!eval_bool(&lua, r#"return DAEMON.cache.inspect("t", 1).dirty"#));
    assert_eq!(eval_int(&lua, r#"return #DAEMON.cache._plan_flush(nil, { all = true })"#), 0);
}

#[test]
fn an_expired_key_disappears_on_read_and_from_the_plan() {
    let lua = cache_lua(r#"{ tier = "write_behind" }"#);
    lua.load(r#"
        DAEMON.cache.set("t", 1, "temp", "x", { expires_at = 1005 })
        DAEMON.cache.set("t", 1, "keep", "y")
    "#).exec().unwrap();
    set_time(&lua, 1010);

    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("t", 1, "temp") == nil"#));
    assert_eq!(eval_str(&lua, r#"
        local plan = DAEMON.cache._plan_flush(nil, { all = true })
        local keys = {}
        for k in pairs(plan[1].doc) do keys[#keys+1] = k end
        return table.concat(keys, ",")
    "#), "keep");
}

#[test]
fn a_scope_that_empties_plans_a_delete_rather_than_an_empty_document() {
    let lua = cache_lua(r#"{ tier = "write_behind", delete_when_empty = true }"#);
    lua.load(r#"
        DAEMON.cache.set("t", 1, "only", "x")
        DAEMON.cache.delete("t", 1, "only")
    "#).exec().unwrap();
    assert_eq!(eval_str(&lua, r#"
        return DAEMON.cache._plan_flush(nil, { all = true })[1].action
    "#), "delete");
}

#[test]
fn the_memory_tier_never_appears_in_a_flush_plan() {
    let lua = cache_lua(r#"{ tier = "memory" }"#);
    lua.load(r#"for i = 1, 10 do DAEMON.cache.set("t", i, "k", i) end"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return #DAEMON.cache._plan_flush(nil, { all = true })"#), 0);
}

/// Two hundred dirty scopes on one tick is 20ms of game thread — a visible
/// hitch. The budget spreads it, oldest first so staleness stays bounded.
#[test]
fn a_budget_limits_one_tick_and_takes_the_oldest_first() {
    let lua = cache_lua(r#"{ tier = "write_behind", flush_seconds = 0 }"#);
    lua.load(r#"for i = 1, 200 do DAEMON.cache.set("t", i, "k", i) end"#).exec().unwrap();

    assert_eq!(eval_int(&lua, r#"return #DAEMON.cache._plan_flush(32, { all = true })"#), 32);
    assert_eq!(eval_str(&lua, r#"
        local plan = DAEMON.cache._plan_flush(3, { all = true })
        return plan[1].id .. "," .. plan[2].id .. "," .. plan[3].id
    "#), "1,2,3", "scope 1 was dirtied first, so it is written first");
}

#[test]
fn edit_marks_the_scope_dirty_however_many_keys_it_touched() {
    let lua = cache_lua(r#"{ tier = "write_behind" }"#);
    assert!(eval_bool(&lua, r#"
        DAEMON.cache.edit("t", 1, function(scope)
            for i = 1, 12 do scope["k" .. i] = i end
        end)
        return DAEMON.cache.inspect("t", 1).dirty
    "#));
    assert_eq!(eval_int(&lua, r#"return #DAEMON.cache.keys("t", 1)"#), 12);
}

#[test]
fn a_failed_write_is_retried_rather_than_dropped() {
    let lua = cache_lua(r#"{ tier = "write_behind", flush_seconds = 0 }"#);
    lua.load(r#"
        DAEMON.cache.set("t", 1, "a", 1)
        -- No db_put in this harness, so the write fails.
        DAEMON.cache._apply(DAEMON.cache._plan_flush(nil, { all = true }))
    "#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return DAEMON.cache.inspect("t", 1).dirty"#),
        "a scope whose write failed must stay dirty — the data is still only in memory");
    assert_eq!(eval_int(&lua, r#"return DAEMON.cache.inspect("t", 1).fails"#), 1);
    assert_eq!(eval_int(&lua, "return DAEMON.cache.stats().flush_failures"), 1);
}

#[test]
fn three_failures_quarantine_a_scope_without_losing_it() {
    let lua = cache_lua(r#"{ tier = "write_behind", flush_seconds = 0 }"#);
    lua.load(r#"
        DAEMON.cache.set("t", 1, "a", 1)
        for _ = 1, 3 do
            local plan = DAEMON.cache._plan_flush(nil, { all = true })
            -- Backoff would normally skip these; drive _apply directly.
            if #plan == 0 then
                plan = { { ns = "t", scope = "1", collection = "t", id = "1",
                           action = "put", doc = { a = 1 } } }
            end
            DAEMON.cache._apply(plan)
        end
    "#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return DAEMON.cache.inspect("t", 1).poisoned"#));
    assert_eq!(eval_int(&lua, r#"return DAEMON.cache.get("t", 1, "a")"#), 1,
        "quarantine must keep the data in memory so the game carries on");
    assert_eq!(eval_int(&lua, r#"return #DAEMON.cache._plan_flush(nil, { all = true })"#), 0,
        "and stop scheduling it");
}

#[test]
fn state_survives_a_hot_reload_of_the_daemon() {
    let lua = cache_lua(r#"{ tier = "write_behind" }"#);
    lua.load(r#"DAEMON.cache.set("t", 7, "kept", "yes")"#).exec().unwrap();
    // What a hot reload does: drop the module and require it again.
    lua.load(r#"
        package.loaded['daemons.cache_d'] = nil
        DAEMON.cache = require('daemons.cache_d')
    "#).exec().unwrap();
    assert_eq!(eval_str(&lua, r#"return DAEMON.cache.get("t", 7, "kept")"#), "yes");
    assert!(eval_bool(&lua, r#"return DAEMON.cache.spec("t") ~= nil"#),
        "namespaces registered by other daemons must survive too, or their \
         define calls would never re-run");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  cooldown_d
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn a_cooldown_counts_down_and_then_reports_ready() {
    let lua = make_daemon_lua();
    set_time(&lua, 1000);
    assert!(eval_bool(&lua, r#"return DAEMON.cooldown.mark(42, "manasteel", 3600) ~= false"#));
    assert_eq!(eval_int(&lua, r#"return DAEMON.cooldown.remaining(42, "manasteel")"#), 3600);
    assert!(!eval_bool(&lua, r#"return DAEMON.cooldown.ready(42, "manasteel")"#));

    set_time(&lua, 1000 + 3599);
    assert_eq!(eval_int(&lua, r#"return DAEMON.cooldown.remaining(42, "manasteel")"#), 1);

    set_time(&lua, 1000 + 3600);
    assert!(eval_bool(&lua, r#"return DAEMON.cooldown.ready(42, "manasteel")"#));
    assert_eq!(eval_int(&lua, r#"return DAEMON.cooldown.remaining(42, "manasteel")"#), 0);
}

/// The threshold rule: under a minute it is a game mechanic and lives in
/// memory, over a minute it is a promise to the player and goes to disk.
#[test]
fn duration_chooses_the_tier() {
    let lua = make_daemon_lua();
    set_time(&lua, 1000);
    lua.load(r#"
        DAEMON.cooldown.mark(1, "daily", 86400)
        DAEMON.cooldown.mark(1, "fireball", 6)
    "#).exec().unwrap();

    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("cooldowns", 1, "daily") ~= nil"#));
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("cooldowns_fast", 1, "daily") == nil"#));
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("cooldowns_fast", 1, "fireball") ~= nil"#));
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("cooldowns", 1, "fireball") == nil"#));
}

#[test]
fn the_durable_flag_overrides_the_threshold_both_ways() {
    let lua = make_daemon_lua();
    set_time(&lua, 1000);
    lua.load(r#"
        DAEMON.cooldown.mark(1, "rare_but_short", 10, { durable = true })
        DAEMON.cooldown.mark(1, "long_but_cheap", 86400, { durable = false })
    "#).exec().unwrap();
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("cooldowns", 1, "rare_but_short") ~= nil"#));
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("cooldowns_fast", 1, "long_but_cheap") ~= nil"#));
}

/// Re-marking with a different tier must not leave the old copy behind, or
/// `remaining` would keep finding it.
#[test]
fn moving_a_cooldown_between_tiers_leaves_nothing_behind() {
    let lua = make_daemon_lua();
    set_time(&lua, 1000);
    lua.load(r#"
        DAEMON.cooldown.mark(1, "thing", 86400)
        DAEMON.cooldown.mark(1, "thing", 5)
    "#).exec().unwrap();
    assert!(eval_bool(&lua, r#"return DAEMON.cache.get("cooldowns", 1, "thing") == nil"#));
    assert_eq!(eval_int(&lua, r#"return DAEMON.cooldown.remaining(1, "thing")"#), 5);
}

#[test]
fn listing_shows_only_what_is_still_gating() {
    let lua = make_daemon_lua();
    set_time(&lua, 1000);
    lua.load(r#"
        DAEMON.cooldown.mark(1, "short", 10, { durable = false })
        DAEMON.cooldown.mark(1, "long", 100, { durable = false })
    "#).exec().unwrap();
    set_time(&lua, 1020);
    assert_eq!(eval_str(&lua, r#"
        local list = DAEMON.cooldown.list(1)
        local out = {}
        for _, c in ipairs(list) do out[#out+1] = c.what end
        return table.concat(out, ",")
    "#), "long");
}

#[test]
fn a_cooldown_needs_a_positive_duration() {
    let lua = make_daemon_lua();
    assert!(!eval_bool(&lua, r#"return DAEMON.cooldown.mark(1, "x", 0) ~= false"#));
    assert!(!eval_bool(&lua, r#"return DAEMON.cooldown.mark(1, "x", -5) ~= false"#));
    assert!(!eval_bool(&lua, r#"return DAEMON.cooldown.mark(1, "", 10) ~= false"#));
}


// ═══════════════════════════════════════════════════════════════════════════════
//  trait_d — attributes, derivation, regeneration
// ═══════════════════════════════════════════════════════════════════════════════

/// A VM with a small but representative trait set: two attributes, a counter,
/// two derived traits (one of which is a gauge's maximum), and a regenerating
/// gauge. `e` is an attached entity.
fn trait_lua() -> Lua {
    let lua = make_daemon_lua();
    set_time(&lua, 1000);
    lua.load(r#"
        _formula_calls = 0
        DAEMON.trait.define_all({
            { id = "constitution", kind = "attribute", default = 10 },
            { id = "wisdom",       kind = "attribute", default = 10 },
            { id = "level",        kind = "counter",   default = 1 },
            { id = "max_hp", kind = "derived", depends = { "constitution", "level" },
              formula = function(t)
                  _formula_calls = _formula_calls + 1
                  return 50 + t.constitution * 5 + t.level * 10
              end },
            { id = "hp", kind = "gauge", max = "max_hp", min = 0,
              regen = { rate = 1, per = 3, target = "max" } },
            { id = "willpower", kind = "derived", depends = { "wisdom", "level" },
              formula = function(t) return math.floor((t.wisdom - 10) / 2) + t.level end },
        })
        DAEMON.trait.seal()
        e = { char_id = 1, stats = {} }
        DAEMON.trait.seed(e, "character")
    "#).exec().expect("trait fixture");
    lua
}

#[test]
fn an_attribute_starts_at_its_default_and_can_be_set() {
    let lua = trait_lua();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "constitution")"#), 10);
    lua.load(r#"DAEMON.trait.set_base(e, "constitution", 16)"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "constitution")"#), 16);
    assert_eq!(eval_int(&lua, "return e.stats.constitution"), 16,
        "the base is what is stored, and CHARACTER_D is what saves it");
}

/// The requirement: a trait derived from another trait, D&D style.
#[test]
fn a_derived_trait_reads_another_trait() {
    let lua = trait_lua();
    // wisdom 10, level 1 -> (10-10)/2 + 1
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "willpower")"#), 1);
    lua.load(r#"DAEMON.trait.set_base(e, "wisdom", 18)"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "willpower")"#), 5,
        "changing wisdom must change willpower with nothing else touched");
}

#[test]
fn a_derived_trait_is_never_stored() {
    let lua = trait_lua();
    lua.load(r#"local _ = DAEMON.trait.value(e, "max_hp")"#).exec().unwrap();
    assert!(eval_bool(&lua, "return e.stats.max_hp == nil"),
        "storing a derived value would shadow the formula forever");
}

/// `max_hp` used to be a plain saved number. Attaching an old character has to
/// drop it, or the formula would never be consulted again.
#[test]
fn attaching_drops_a_stored_value_for_a_trait_that_is_now_derived() {
    let lua = trait_lua();
    lua.load(r#"
        old = { char_id = 2, stats = { max_hp = 999, hp = 900, constitution = 10, level = 1 } }
        DAEMON.trait.attach(old)
    "#).exec().unwrap();
    assert!(eval_bool(&lua, "return old.stats.max_hp == nil"));
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(old, "max_hp")"#), 110);
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(old, "hp")"#), 110,
        "and the gauge is clamped into the range the formula now gives");
}

/// `depends` is what the cycle detector reasons about, so a formula that reads
/// something it did not declare has to be an error rather than a quiet
/// success — otherwise the declaration rots and the detector's answer is a lie.
#[test]
fn reading_an_undeclared_dependency_is_an_error() {
    let lua = trait_lua();
    lua.load(r#"
        _logged = {}
        function log(level, msg) _logged[#_logged + 1] = tostring(msg) end
        DAEMON.trait.define({ id = "sneaky", kind = "derived", depends = { "wisdom" },
            formula = function(t) return t.constitution end })
        DAEMON.trait.seal()
        DAEMON.trait.bump(e)
        _v = DAEMON.trait.value(e, "sneaky")
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return _v"), 0, "it falls back to the default");
    assert!(eval_bool(&lua, r#"
        for _, m in ipairs(_logged) do
            if m:find("undeclared dependency") and m:find("constitution") then return true end
        end
        return false
    "#), "the error must name the dependency that was not declared");
}

#[test]
fn a_dependency_cycle_is_reported_as_a_path_and_does_not_crash() {
    let lua = trait_lua();
    lua.load(r#"
        _logged = {}
        function log(level, msg) _logged[#_logged + 1] = tostring(msg) end
        DAEMON.trait.define({ id = "a", kind = "derived", depends = { "b" },
            formula = function(t) return t.b + 1 end })
        DAEMON.trait.define({ id = "b", kind = "derived", depends = { "a" },
            formula = function(t) return t.a + 1 end })
        _ok = DAEMON.trait.seal()
    "#).exec().unwrap();

    assert!(!eval_bool(&lua, "return _ok"));
    assert!(eval_bool(&lua, r#"
        for _, m in ipairs(_logged) do
            if m:find("dependency cycle") and m:find("->") then return true end
        end
        return false
    "#), "the message has to show the path, not just that a cycle exists");
    // Everything else still works.
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "willpower")"#), 1,
        "one broken trait must not disable the other thirty");
    assert!(eval_bool(&lua, r#"return DAEMON.trait.errors()["a"] ~= nil"#));
}

#[test]
fn a_gauge_is_clamped_to_the_trait_that_is_its_maximum() {
    let lua = trait_lua();
    // constitution 10, level 1 -> max_hp 110
    lua.load(r#"DAEMON.trait.set_cur(e, "hp", 500)"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "hp")"#), 110);
    lua.load(r#"DAEMON.trait.set_cur(e, "hp", -20)"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "hp")"#), 0);
}

/// The memo is what keeps trait resolution off the per-command hot path.
#[test]
fn values_are_computed_once_until_something_changes() {
    let lua = trait_lua();
    lua.load(r#"
        _formula_calls = 0
        for _ = 1, 20 do local _ = DAEMON.trait.value(e, "max_hp") end
    "#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return _formula_calls"), 1,
        "twenty reads, one evaluation");

    lua.load(r#"DAEMON.trait.set_base(e, "constitution", 12)"#).exec().unwrap();
    lua.load(r#"local _ = DAEMON.trait.value(e, "max_hp")"#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return _formula_calls"), 2, "and a change invalidates it");
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "max_hp")"#), 120);
}

#[test]
fn regeneration_advances_a_gauge_without_a_timer() {
    let lua = trait_lua();
    lua.load(r#"DAEMON.trait.set_cur(e, "hp", 40)"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "hp")"#), 40);

    set_time(&lua, 1010);
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "hp")"#), 43,
        "ten seconds at one per three is three points");
    assert_eq!(eval_int(&lua, "return e.stats._at.hp"), 1009,
        "and the tenth second is carried, not lost");
}

/// The line that keeps write-behind working: a settle that earned nothing must
/// not report a change, because the prompt calls this on every command.
#[test]
fn a_gauge_at_full_health_changes_nothing_however_long_it_idles() {
    let lua = trait_lua();
    lua.load(r#"DAEMON.trait.set_cur(e, "hp", 110)"#).exec().unwrap();
    let anchor_before = eval_str(&lua, "return tostring(e.stats._at.hp)");
    set_time(&lua, 99999);
    assert!(!eval_bool(&lua, "return DAEMON.trait.touch(e)"));
    assert_eq!(eval_str(&lua, "return tostring(e.stats._at.hp)"), anchor_before);
}

#[test]
fn adjusting_a_gauge_settles_it_first() {
    let lua = trait_lua();
    lua.load(r#"DAEMON.trait.set_cur(e, "hp", 40)"#).exec().unwrap();
    set_time(&lua, 1010);
    // 40 -> 43 by regeneration, then -3 from the adjustment.
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.adjust(e, "hp", -3)"#), 40);
}

#[test]
fn all_reports_base_and_effective_side_by_side() {
    let lua = trait_lua();
    assert_eq!(eval_str(&lua, r#"
        for _, t in ipairs(DAEMON.trait.all(e)) do
            if t.id == "willpower" then
                return t.label .. "/" .. t.kind .. "/" .. tostring(t.value)
            end
        end
        return "missing"
    "#), "willpower/derived/1");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  effect_d — instances, stacking, and the pipeline in anger
// ═══════════════════════════════════════════════════════════════════════════════

/// The trait fixture plus the three effects from the design: the four examples
/// the whole system was specified against.
fn effect_lua() -> Lua {
    let lua = trait_lua();
    lua.load(r#"
        _events = {}
        DAEMON.effect.define_all({
            {
                id = "insight", label = "Insight", duration = 3600,
                stack = "stack", max_stacks = 3,
                hooks = { xp_gained = { phase = "mult", fn = function(ev, ctx)
                    ev.scale = ev.scale + 0.20 * (ctx.stacks or 1)
                end } },
            },
            {
                id = "stoneskin", label = "Stoneskin", duration = 60, potency = 5,
                modifiers = { constitution = 2 },
                hooks = {
                    damage_taken = { phase = "mult", fn = function(ev)
                        ev.scale = ev.scale - 0.15
                    end },
                    ["damage_taken#flat"] = { hook = "damage_taken", phase = "reduce",
                        fn = function(ev, ctx)
                            ev.amount = math.max(0, ev.amount - (ctx.potency or 5))
                        end },
                },
                on_apply  = function() _events[#_events+1] = "stoneskin:apply" end,
                on_expire = function(ctx) _events[#_events+1] = "stoneskin:expire:" .. tostring(ctx.reason) end,
            },
            {
                id = "regeneration", label = "Regeneration", duration = 30, tick = 3,
                hooks = { heartbeat = { phase = "post", fn = function(ev, ctx)
                    _events[#_events+1] = "regen:" .. tostring(ev.ticks)
                end } },
            },
        })
    "#).exec().expect("effect fixture");
    lua
}

#[test]
fn an_effect_can_be_applied_and_read_back() {
    let lua = effect_lua();
    assert!(eval_bool(&lua, r#"return DAEMON.effect.apply(e, "stoneskin") ~= false"#));
    assert!(eval_bool(&lua, r#"return DAEMON.effect.has(e, "stoneskin")"#));
    assert_eq!(eval_int(&lua, "return #DAEMON.effect.active(e)"), 1);
    assert_eq!(eval_str(&lua, "return _events[1]"), "stoneskin:apply");
}

/// The four worked examples, end to end through the real pipeline.
#[test]
fn the_pipeline_applies_a_percentage_before_a_flat_reduction() {
    let lua = effect_lua();
    lua.load(r#"DAEMON.effect.apply(e, "stoneskin")"#).exec().unwrap();
    assert_eq!(eval_str(&lua, r#"
        local ev = DAEMON.effect.run(e, "damage_taken", { amount = 30, scale = 0, min = 0 })
        return tostring(ev.amount)
    "#), "20.5", "30 * 0.85 - 5");
}

#[test]
fn stacks_scale_a_multiplier() {
    let lua = effect_lua();
    lua.load(r#"
        DAEMON.effect.apply(e, "insight")
        DAEMON.effect.apply(e, "insight")
    "#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"
        local inst = DAEMON.effect.active(e)[1].inst
        return inst.stacks
    "#), 2);
    assert_eq!(eval_str(&lua, r#"
        local ev = DAEMON.effect.run(e, "xp_gained", { amount = 100, scale = 0 })
        return tostring(ev.amount == 140)
    "#), "true", "two stacks of +20% is +40%, not +44%");
}

#[test]
fn a_passive_modifier_changes_a_trait_and_everything_derived_from_it() {
    let lua = effect_lua();
    let before = eval_int(&lua, r#"return DAEMON.trait.value(e, "max_hp")"#);
    lua.load(r#"DAEMON.effect.apply(e, "stoneskin")"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "constitution")"#), 12,
        "+2 constitution from the effect, with nothing stored on the trait");
    assert_eq!(eval_int(&lua, "return e.stats.constitution"), 10,
        "and the base is untouched — this is what a stored `mod` gets wrong");
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "max_hp")"#), before + 10,
        "the modifier flows through the dependency graph");

    lua.load(r#"DAEMON.effect.remove(e, "stoneskin")"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "max_hp")"#), before,
        "and removing it needs no unapply step, because nothing was applied");
}

/// A gauge is changed by events, never by modifiers. Catching this at
/// definition time beats a buff that silently does nothing.
#[test]
fn an_effect_cannot_declare_a_modifier_on_a_gauge() {
    let lua = effect_lua();
    assert!(!eval_bool(&lua, r#"
        return DAEMON.effect.define({ id = "bogus", modifiers = { hp = 10 } })
    "#));
    assert!(!eval_bool(&lua, r#"
        return DAEMON.effect.define({ id = "bogus2", modifiers = { level = 1 } })
    "#), "a counter is the same story");
}

#[test]
fn refreshing_extends_rather_than_duplicating() {
    let lua = effect_lua();
    lua.load(r#"DAEMON.effect.apply(e, "stoneskin", { source = "spell:x" })"#).exec().unwrap();
    set_time(&lua, 1030);
    lua.load(r#"DAEMON.effect.apply(e, "stoneskin", { source = "spell:x" })"#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return #DAEMON.effect.active(e)"), 1);
    assert_eq!(eval_int(&lua, "return DAEMON.effect.active(e)[1].inst.expires"), 1090);
}

#[test]
fn a_weaker_reapplication_never_shortens_an_effect() {
    let lua = effect_lua();
    lua.load(r#"DAEMON.effect.apply(e, "stoneskin", { duration = 600 })"#).exec().unwrap();
    lua.load(r#"DAEMON.effect.apply(e, "stoneskin", { duration = 10 })"#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return DAEMON.effect.active(e)[1].inst.expires"), 1600);
}

#[test]
fn independent_effects_each_keep_their_own_clock() {
    let lua = effect_lua();
    lua.load(r#"
        DAEMON.effect.define({ id = "bleed", stack = "independent", duration = 20 })
        DAEMON.effect.apply(e, "bleed")
        DAEMON.effect.apply(e, "bleed")
    "#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return #DAEMON.effect.active(e)"), 2);
}

#[test]
fn ignore_and_replace_do_what_they_say() {
    let lua = effect_lua();
    lua.load(r#"
        DAEMON.effect.define({ id = "shield", stack = "ignore", duration = 60 })
        DAEMON.effect.define({ id = "stance", stack = "replace", duration = 60 })
        _first  = DAEMON.effect.apply(e, "shield")
        _second = DAEMON.effect.apply(e, "shield")
        DAEMON.effect.apply(e, "stance")
        DAEMON.effect.apply(e, "stance")
    "#).exec().unwrap();
    assert!(eval_bool(&lua, "return _first ~= false and _second == false"));
    assert_eq!(eval_int(&lua, r#"
        local n = 0
        for _, x in ipairs(DAEMON.effect.active(e)) do
            if x.inst.def == "stance" then n = n + 1 end
        end
        return n
    "#), 1);
}

#[test]
fn an_expired_effect_stops_applying_and_says_so_once() {
    let lua = effect_lua();
    lua.load(r#"
        _events = {}
        DAEMON.effect.apply(e, "stoneskin")
    "#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "constitution")"#), 12);

    set_time(&lua, 1000 + 61);
    assert_eq!(eval_int(&lua, "return #DAEMON.effect.active(e)"), 0);
    assert_eq!(eval_int(&lua, r#"return DAEMON.trait.value(e, "constitution")"#), 10,
        "an expired effect must stop modifying, with no sweep having run");

    // Reading again must not fire on_expire a second time.
    lua.load("local _ = DAEMON.effect.active(e)").exec().unwrap();
    assert_eq!(eval_int(&lua, r#"
        local n = 0
        for _, ev in ipairs(_events) do
            if ev:find("stoneskin:expire") then n = n + 1 end
        end
        return n
    "#), 1);
}

/// Time is the only thing that changed, so the memo has to notice it — via the
/// one comparison TRAIT_D caches, not by giving up on memoization.
#[test]
fn expiry_invalidates_memoized_trait_values() {
    let lua = effect_lua();
    lua.load(r#"
        DAEMON.effect.apply(e, "stoneskin")
        _v1 = DAEMON.trait.value(e, "max_hp")
    "#).exec().unwrap();
    set_time(&lua, 1000 + 61);
    assert!(eval_bool(&lua, r#"return DAEMON.trait.value(e, "max_hp") < _v1"#));
}

#[test]
fn the_heartbeat_earns_whole_ticks_and_carries_the_rest() {
    let lua = effect_lua();
    lua.load(r#"
        _events = {}
        DAEMON.effect.apply(e, "regeneration")
    "#).exec().unwrap();

    set_time(&lua, 1010);
    lua.load("DAEMON.effect.heartbeat()").exec().unwrap();
    assert_eq!(eval_str(&lua, "return _events[1]"), "regen:3", "ten seconds is three ticks of three");
    assert_eq!(eval_int(&lua, "return DAEMON.effect.active(e)[1].inst.last_tick"), 1009);

    // Two more seconds is not another tick.
    lua.load("_events = {}").exec().unwrap();
    set_time(&lua, 1011);
    lua.load("DAEMON.effect.heartbeat()").exec().unwrap();
    assert_eq!(eval_int(&lua, "return #_events"), 0);
}

#[test]
fn modify_returns_the_number_untouched_when_nothing_listens() {
    let lua = effect_lua();
    assert_eq!(eval_int(&lua, r#"return DAEMON.effect.modify(e, "nothing_listens", 30)"#), 30);
}

/// Equipment, room auras and anything else rebuilt from its source calls this
/// on every login and every change. It has to be safe to call repeatedly.
#[test]
fn set_source_effects_is_idempotent() {
    let lua = effect_lua();
    lua.load(r#"
        DAEMON.effect.set_source_effects(e, "equip:head", { { def = "stoneskin" } })
        DAEMON.effect.set_source_effects(e, "equip:head", { { def = "stoneskin" } })
    "#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return #DAEMON.effect.active(e)"), 1);

    lua.load(r#"DAEMON.effect.set_source_effects(e, "equip:head", {})"#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return #DAEMON.effect.active(e)"), 0,
        "taking the hat off takes the effect with it");
}

#[test]
fn clearing_honours_effects_that_survive_death() {
    let lua = effect_lua();
    lua.load(r#"
        DAEMON.effect.define({ id = "curse", duration = 3600, survives_death = true })
        DAEMON.effect.apply(e, "stoneskin")
        DAEMON.effect.apply(e, "curse")
        DAEMON.effect.clear(e, { keep_survivors = true, reason = "death" })
    "#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return #DAEMON.effect.active(e)"), 1);
    assert_eq!(eval_str(&lua, "return DAEMON.effect.active(e)[1].inst.def"), "curse");
}

/// Instances are written to the database, so they must be plain data — and
/// they must survive `Player._deep_copy`, which drops functions.
#[test]
fn an_effect_instance_is_plain_saveable_data() {
    let lua = effect_lua();
    lua.load(r#"
        js = require('lib.jsonsafe')
        Player = require('lib.player')
        DAEMON.effect.apply(e, "stoneskin", { source = "potion:x", potency = 7, caster = 3 })
        inst = DAEMON.effect.active(e)[1].inst
    "#).exec().unwrap();

    assert!(eval_bool(&lua, "return js.check(inst)"));
    assert!(eval_bool(&lua, r#"
        local copy = Player._deep_copy(inst)
        for k, v in pairs(inst) do if copy[k] ~= v then return false end end
        for k, v in pairs(copy) do if inst[k] ~= v then return false end end
        return true
    "#), "a deep copy must be identical — anything it dropped was not saveable");
}

/// Regeneration heals, healing is an event, and an effect could listen to it
/// and heal again. Refusing beats a silent infinite loop on the game thread.
#[test]
fn the_pipeline_refuses_to_re_enter_itself() {
    let lua = effect_lua();
    lua.load(r#"
        _depth = 0
        DAEMON.effect.define({ id = "loop", duration = 60, hooks = {
            heal_received = { phase = "post", fn = function(ev, ctx)
                _depth = _depth + 1
                DAEMON.effect.run(ctx.entity, "heal_received", { amount = 1, scale = 0 })
            end },
        } })
        DAEMON.effect.apply(e, "loop")
        DAEMON.effect.run(e, "heal_received", { amount = 10, scale = 0 })
    "#).exec().unwrap();
    assert_eq!(eval_int(&lua, "return _depth"), 1);
}


/// Overwriting the same key must not make the scope look as though it is
/// growing. The byte estimate once added the key on every write rather than
/// only the first, so a scope holding a single counter crept toward the
/// document ceiling and eventually refused every write with a size complaint
/// that was not true.
#[test]
fn repeatedly_overwriting_one_key_does_not_inflate_the_scope() {
    let lua = cache_lua(r#"{ tier = "write_behind" }"#);
    lua.load(r#"
        for i = 1, 20000 do DAEMON.cache.set("t", 1, "counter", i) end
    "#).exec().unwrap();

    assert_eq!(eval_int(&lua, "return DAEMON.cache.stats().rejected_writes"), 0,
        "twenty thousand updates to one small key must all be accepted");
    assert_eq!(eval_int(&lua, r#"return DAEMON.cache.get("t", 1, "counter")"#), 20000);
    assert!(eval_bool(&lua, r#"return DAEMON.cache.inspect("t", 1).bytes < 200"#),
        "one small key should not be estimated at hundreds of bytes");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Grammar — pronoun sets and verb agreement
// ═══════════════════════════════════════════════════════════════════════════════

fn grammar_lua() -> Lua {
    let lua = make_test_lua();
    lua.load("G = require('lib.grammar')").exec().unwrap();
    lua
}

/// A regular verb takes -s for a third-person singular subject.
#[test]
fn a_regular_verb_takes_s_for_a_singular_subject() {
    let lua = grammar_lua();
    for (verb, want) in [("swing", "swings"), ("hit", "hits"), ("draw", "draws")] {
        assert_eq!(eval_str(&lua, &format!("return G.conjugate('{verb}', false)")), want);
        assert_eq!(eval_str(&lua, &format!("return G.conjugate('{verb}', true)")), verb);
    }
}

/// A sibilant takes -es, and so does a bare -o.
#[test]
fn a_sibilant_verb_takes_es() {
    let lua = grammar_lua();
    for (verb, want) in [
        ("kiss", "kisses"), ("push", "pushes"), ("catch", "catches"),
        ("mix", "mixes"), ("buzz", "buzzes"), ("echo", "echoes"),
    ] {
        assert_eq!(eval_str(&lua, &format!("return G.conjugate('{verb}', false)")), want);
    }
}

/// Consonant + y becomes -ies; vowel + y does not.
#[test]
fn a_consonant_y_becomes_ies_and_a_vowel_y_does_not() {
    let lua = grammar_lua();
    for (verb, want) in [
        ("parry", "parries"), ("fly", "flies"), ("try", "tries"),
        ("say", "says"), ("buy", "buys"), ("play", "plays"),
    ] {
        assert_eq!(eval_str(&lua, &format!("return G.conjugate('{verb}', false)")), want);
    }
}

/// The irregulars are a table, not a rule — and a modal never inflects at all.
#[test]
fn the_irregulars_are_a_table_and_modals_do_not_inflect() {
    let lua = grammar_lua();
    for (verb, want) in [("be", "is"), ("have", "has"), ("do", "does"), ("go", "goes")] {
        assert_eq!(eval_str(&lua, &format!("return G.conjugate('{verb}', false)")), want);
    }
    for verb in ["can", "will", "must", "may", "shall"] {
        assert_eq!(eval_str(&lua, &format!("return G.conjugate('{verb}', false)")), verb);
        assert_eq!(eval_str(&lua, &format!("return G.conjugate('{verb}', true)")), verb);
    }
}

/// **The load-bearing one.** Singular they takes the same verb form as "you",
/// because `plural` is not a count — it is which form the subject takes. If this
/// ever fails, the game says "they swings".
#[test]
fn singular_they_takes_the_same_verb_form_as_you() {
    let lua = grammar_lua();

    assert!(eval_bool(&lua, "return G.SETS.neutral.plural == G.VIEWER.plural"));

    for verb in ["swing", "be", "have", "parry", "go"] {
        let they = eval_str(&lua, &format!("return G.conjugate('{verb}', G.SETS.neutral.plural)"));
        let you = eval_str(&lua, &format!("return G.conjugate('{verb}', G.VIEWER.plural)"));
        assert_eq!(they, you, "'{verb}' must agree the same way for they and for you");
    }

    // Spelled out, because these are the two that read as a bug instantly.
    assert_eq!(eval_str(&lua, "return G.conjugate('swing', G.SETS.neutral.plural)"), "swing");
    assert_eq!(eval_str(&lua, "return G.conjugate('be', G.SETS.neutral.plural)"), "are");
    // …and the singular sets still inflect.
    assert_eq!(eval_str(&lua, "return G.conjugate('be', G.SETS.female.plural)"), "is");
    assert_eq!(eval_str(&lua, "return G.conjugate('be', G.SETS.thing.plural)"), "is");
}

/// One person is `themself`; a swarm is `themselves`. One row apart, and it is
/// the difference people notice.
#[test]
fn neutral_is_themself_and_plural_is_themselves() {
    let lua = grammar_lua();
    assert_eq!(eval_str(&lua, "return G.SETS.neutral.themself"), "themself");
    assert_eq!(eval_str(&lua, "return G.SETS.plural.themself"), "themselves");

    // …and they are otherwise identical, which is the claim being made.
    assert!(eval_bool(
        &lua,
        "for _, f in ipairs(G.FORMS) do if f ~= 'themself' and G.SETS.neutral[f] ~= G.SETS.plural[f] then return false end end return true"
    ));
}

/// Every set is complete, including one the game layer adds later.
#[test]
fn every_pronoun_set_is_complete() {
    let lua = grammar_lua();
    lua.load("G.define_set('partial', { they = 'ze', them = 'hir' })").exec().unwrap();

    let missing = eval_str(
        &lua,
        "local bad = {} for name, set in pairs(G.SETS) do for _, f in ipairs(G.FORMS) do if type(set[f]) ~= 'string' or set[f] == '' then bad[#bad+1] = name..'.'..f end end if type(set.plural) ~= 'boolean' then bad[#bad+1] = name..'.plural' end end table.sort(bad) return table.concat(bad, ',')",
    );
    assert_eq!(missing, "");

    // A partial definition is filled from neutral rather than leaving nils.
    assert_eq!(eval_str(&lua, "return G.SETS.partial.they"), "ze");
    assert_eq!(eval_str(&lua, "return G.SETS.partial.themself"), "themself");
}

/// An entity with no gender is `it` — unless it is a player, and every player
/// alive is in that bucket because nothing sets `gender` at character creation.
#[test]
fn an_entity_with_no_gender_is_it_unless_it_is_a_player() {
    let lua = grammar_lua();

    assert_eq!(eval_str(&lua, "return G.set_for({ id = 'wisp' }).they"), "it");
    assert_eq!(eval_str(&lua, "return G.set_for({ id = 'wisp' }).themself"), "itself");

    assert_eq!(eval_str(&lua, "return G.set_for({ char_id = 1 }).they"), "they");
    assert_eq!(eval_str(&lua, "return G.set_for({ char_id = 1 }).their"), "their");

    // A declared gender wins over both.
    assert_eq!(eval_str(&lua, "return G.set_for({ char_id = 1, gender = 'female' }).they"), "she");
    assert_eq!(eval_str(&lua, "return G.set_for({ id = 'x', gender = 'male' }).them"), "him");
}

/// An entity may carry its own pronoun table, so neopronouns need nothing from
/// the mudlib. Missing forms fill from neutral rather than becoming nil.
#[test]
fn an_entity_may_carry_its_own_pronoun_table() {
    let lua = grammar_lua();
    let out = eval_str(
        &lua,
        "local s = G.set_for({ char_id = 2, pronouns = { they = 'ze', them = 'hir', their = 'hir', theirs = 'hirs', themself = 'hirself' } }) return s.they .. '|' .. s.them .. '|' .. s.theirs .. '|' .. s.themself",
    );
    assert_eq!(out, "ze|hir|hirs|hirself");

    let partial = eval_str(
        &lua,
        "local s = G.set_for({ char_id = 2, pronouns = { they = 'ze' } }) return s.they .. '|' .. s.their",
    );
    assert_eq!(partial, "ze|their", "an unspecified form falls back rather than vanishing");
}

/// Capitalising steps over a colour tag rather than putting a capital on a brace.
#[test]
fn capitalising_skips_a_leading_colour_tag() {
    let lua = grammar_lua();
    assert_eq!(eval_str(&lua, "return G.capitalise('a wisp')"), "A wisp");
    assert_eq!(eval_str(&lua, "return G.capitalise('{cyan}a wisp')"), "{cyan}A wisp");
    assert_eq!(eval_str(&lua, "return G.capitalise('{red}{bold}a wisp')"), "{red}{bold}A wisp");
    assert_eq!(eval_str(&lua, "return G.capitalise('')"), "");
    assert_eq!(eval_str(&lua, "return G.capitalise('{red}')"), "{red}");
}

/// An article is dropped so a possessive can take its place.
#[test]
fn an_article_is_stripped_for_a_possessive() {
    let lua = grammar_lua();
    assert_eq!(eval_str(&lua, "return G.strip_article('a longsword')"), "longsword");
    assert_eq!(eval_str(&lua, "return G.strip_article('an iron key')"), "iron key");
    assert_eq!(eval_str(&lua, "return G.strip_article('the Delver')"), "Delver");
    assert_eq!(eval_str(&lua, "return G.strip_article('anvil')"), "anvil");
    assert_eq!(eval_str(&lua, "return G.strip_article('right fist')"), "right fist");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Render — one authored line, read per viewer
// ═══════════════════════════════════════════════════════════════════════════════

fn render_lua() -> Lua {
    let lua = make_test_lua();
    lua.load(
        "R = require('lib.render') \
         wren = { char_id = 1, name = 'Wren', gender = 'female' } \
         wisp = { id = 'mob:1', short = 'a pale wisp' } \
         sword = { id = 'item:1', short = 'a longsword' } \
         onlooker = { char_id = 2, name = 'Bram', gender = 'male' } \
         ctx = { actor = wren, target = wisp, weapon = sword, dealt = 7 }",
    )
    .exec()
    .unwrap();
    lua
}

/// **The headline.** One template, three readers, three correct sentences.
#[test]
fn three_viewers_of_one_line_see_three_sentences() {
    let lua = render_lua();
    let t = "$Actor $actor.v(swing) $weapon.of(actor) at $target.";

    assert_eq!(
        eval_str(&lua, &format!("return R.render([[{t}]], ctx, wren)")),
        "You swing your longsword at a pale wisp."
    );
    assert_eq!(
        eval_str(&lua, &format!("return R.render([[{t}]], ctx, wisp)")),
        "Wren swings her longsword at you."
    );
    assert_eq!(
        eval_str(&lua, &format!("return R.render([[{t}]], ctx, onlooker)")),
        "Wren swings her longsword at a pale wisp."
    );
}

/// A viewer of nil makes nobody "you" — the mode a log line and the old
/// `Abilities.render` both want.
#[test]
fn a_viewer_of_nil_makes_nobody_you() {
    let lua = render_lua();
    let out = eval_str(&lua, "return R.render('$Actor $actor.v(hit) $target.', ctx, nil)");
    assert_eq!(out, "Wren hits a pale wisp.");
    assert!(!out.contains("you"), "{out}");
}

/// An unknown token survives verbatim — role, form and argument alike.
#[test]
fn an_unknown_token_survives_verbatim() {
    let lua = render_lua();
    for t in [
        "You strike $victim.",
        "You strike $actor.blah.",
        "You strike $dealt.v(swing).",
        "You strike $target.nosuchform.",
    ] {
        assert_eq!(eval_str(&lua, &format!("return R.render([[{t}]], ctx, wren)")), t);
    }
}

/// A table nothing can name is not renderable, so `$ability` survives rather
/// than printing an address. This is the general form of the `$target` bug.
#[test]
fn an_unnameable_table_survives_rather_than_printing_an_address() {
    let lua = render_lua();
    let out = eval_str(
        &lua,
        "return R.render('$spec and $dealt', { spec = { cost = {}, cooldown = {} }, dealt = 3 }, nil)",
    );
    assert_eq!(out, "$spec and 3");
    assert!(!out.contains("table: 0x"), "{out}");
}

/// A token boundary is the whole identifier, not a prefix.
#[test]
fn a_token_boundary_is_the_whole_identifier() {
    let lua = render_lua();

    // `$target_name` is one token and resolves on its own.
    assert_eq!(
        eval_str(&lua, "return R.render('$target_name', { target_name = 'the wisp' }, nil)"),
        "the wisp"
    );
    // `$names` is not `$name` followed by an s.
    assert_eq!(
        eval_str(&lua, "return R.render('$names', { name = 'Wren' }, nil)"),
        "$names"
    );
}

/// A capital initial capitalises the rendered value, and only that.
#[test]
fn a_capital_initial_capitalises_the_rendered_value() {
    let lua = render_lua();
    assert_eq!(eval_str(&lua, "return R.render('$Target', ctx, nil)"), "A pale wisp");
    assert_eq!(eval_str(&lua, "return R.render('$target', ctx, nil)"), "a pale wisp");
    assert_eq!(eval_str(&lua, "return R.render('$Target', ctx, wisp)"), "You");
    assert_eq!(eval_str(&lua, "return R.render('$Actor.their', ctx, nil)"), "Her");
}

/// A colour tag is not a token, and capitalising steps over one.
#[test]
fn a_colour_tag_is_left_alone() {
    let lua = render_lua();
    assert_eq!(
        eval_str(&lua, "return R.render('{red}$Actor $actor.v(hit) $target.{/}', ctx, nil)"),
        "{red}Wren hits a pale wisp.{/}"
    );
    assert_eq!(
        eval_str(&lua, "return R.render('{cyan}$Target', ctx, nil)"),
        "{cyan}A pale wisp"
    );
}

/// `$$` is a literal dollar.
#[test]
fn a_double_dollar_is_a_literal_dollar() {
    let lua = render_lua();
    assert_eq!(eval_str(&lua, "return R.render('$$5 for $target', ctx, nil)"), "$5 for a pale wisp");
}

/// An item takes its owner's possessive, with the article stripped; without an
/// owner it keeps its own article.
#[test]
fn an_item_renders_with_its_owners_possessive() {
    let lua = render_lua();
    assert_eq!(eval_str(&lua, "return R.render('$weapon.of(actor)', ctx, wren)"), "your longsword");
    assert_eq!(eval_str(&lua, "return R.render('$weapon.of(actor)', ctx, nil)"), "her longsword");
    assert_eq!(eval_str(&lua, "return R.render('$weapon', ctx, nil)"), "a longsword");
    // An owner role that is not in ctx degrades to the plain name.
    assert_eq!(eval_str(&lua, "return R.render('$weapon.of(nobody)', ctx, nil)"), "a longsword");
}

/// Verb agreement follows the subject, which is why the verb names it.
#[test]
fn a_verb_agrees_with_the_subject_it_names() {
    let lua = render_lua();
    let t = "$Actor $actor.v(be) here and $target $target.v(be) too.";
    assert_eq!(
        eval_str(&lua, &format!("return R.render([[{t}]], ctx, wren)")),
        "You are here and a pale wisp is too."
    );
    assert_eq!(
        eval_str(&lua, &format!("return R.render([[{t}]], ctx, wisp)")),
        "Wren is here and you are too."
    );
}

/// The same template parses once.
#[test]
fn the_same_template_parses_once() {
    let lua = render_lua();
    assert!(eval_bool(
        &lua,
        "local a = R.parse('$actor hits $target') local b = R.parse('$actor hits $target') return a == b"
    ));
    lua.load("R.flush_cache()").exec().unwrap();
    assert!(eval_bool(
        &lua,
        "local a = R.parse('$actor hits $target') R.flush_cache() return a ~= R.parse('$actor hits $target')"
    ));
}

/// `render_for` renders once per distinct role set, not once per viewer.
#[test]
fn a_group_render_costs_one_pass_per_role_set() {
    let lua = render_lua();
    let out = eval_str(
        &lua,
        "local viewers = { wren, wisp } \
         for i = 1, 8 do viewers[#viewers+1] = { char_id = 100 + i, name = 'Extra' } end \
         local calls = 0 local real = R.render \
         R.render = function(...) calls = calls + 1 return real(...) end \
         local map = R.render_for('$Actor $actor.v(hit) $target.', ctx, viewers) \
         R.render = real \
         return calls .. '|' .. map[wren] .. '|' .. map[wisp] .. '|' .. map[viewers[3]]",
    );
    assert_eq!(
        out,
        "3|You hit a pale wisp.|Wren hits you.|Wren hits a pale wisp.",
        "ten viewers, two participants, three distinct sentences and three renders"
    );
}

/// `display_name` defaults to `name`, which is what `combat_d` did before this
/// existed. The `config` stub returns nil, so this is the fallback path.
#[test]
fn display_name_prefers_name_by_default() {
    let lua = render_lua();
    assert_eq!(eval_str(&lua, "return R.display_name({ name = 'Wren' })"), "Wren");
    assert_eq!(
        eval_str(&lua, "return R.display_name({ name = 'wisp', short = 'a pale wisp' })"),
        "wisp"
    );
    assert_eq!(eval_str(&lua, "return R.display_name({ short = 'a rat' })"), "a rat");
    assert_eq!(eval_str(&lua, "return R.display_name({ id = 'x' })"), "something");
    // An lfun short resolves, as it does everywhere else.
    assert_eq!(
        eval_str(&lua, "return R.display_name({ short = function() return 'a shifting light' end })"),
        "a shifting light"
    );
}

/// `game.display_name_prefers = "short"` makes messages read as prose.
///
/// Which field names a creature in a sentence is a decision about what kind of
/// game this is — "You hit a pale wisp" for roleplay, "You hit wisp" for
/// hack-and-slash — so it is configuration rather than a rule. Either way the
/// *other* field is the fallback, so a thing carrying only one still works.
#[test]
fn display_name_can_prefer_short_instead() {
    let lua = render_lua();
    lua.load(
        "config = function(k) if k == 'game.display_name_prefers' then return 'short' end end \
         R.flush_cache()",
    )
    .exec()
    .unwrap();

    assert_eq!(
        eval_str(&lua, "return R.display_name({ name = 'wisp', short = 'a pale wisp' })"),
        "a pale wisp"
    );
    // Fallbacks, both directions.
    assert_eq!(eval_str(&lua, "return R.display_name({ name = 'Wren' })"), "Wren");
    assert_eq!(eval_str(&lua, "return R.display_name({ short = 'a rat' })"), "a rat");
    assert_eq!(eval_str(&lua, "return R.display_name({ id = 'x' })"), "something");

    // And it reaches a rendered line, which is the point of the setting.
    assert_eq!(
        eval_str(
            &lua,
            "return R.render('$Actor $actor.v(hit) $target.', \
             { actor = { name = 'Wren' }, target = { name = 'wisp', short = 'a pale wisp' } }, nil)"
        ),
        "Wren hits a pale wisp."
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Queues — roundtime arithmetic and the shape of a track
// ═══════════════════════════════════════════════════════════════════════════════

fn queues_lua() -> Lua {
    let lua = make_test_lua();
    lua.load("Q = require('lib.queues') AB = require('lib.abilities')").exec().unwrap();
    lua
}

/// Rounds ceil to whole seconds, because the clock is whole seconds.
///
/// `os_time()` is integer, so a stored expiry of `now + 2.25` is observed at
/// one-second granularity — the gate would open somewhere in [2s, 3s] depending
/// on where inside the second it was marked. A stated 3 beats an unstated
/// 2-or-3, and the variance would be invisible and unreproducible.
#[test]
fn rounds_ceil_to_whole_seconds_because_the_clock_does() {
    let lua = queues_lua();
    assert_eq!(eval_int(&lua, "return Q.rounds_to_seconds(0.75, 4)"), 3);
    assert_eq!(eval_int(&lua, "return Q.rounds_to_seconds(0.75, 8)"), 6);
    assert_eq!(eval_int(&lua, "return Q.rounds_to_seconds(1, 3)"), 3);
    assert_eq!(eval_int(&lua, "return Q.rounds_to_seconds(1.25, 4)"), 5);
    // 2.25 rounds up rather than truncating to something the clock cannot show.
    assert_eq!(eval_int(&lua, "return Q.rounds_to_seconds(0.5625, 4)"), 3);
    // Floored at one: a roundtime of zero is not a roundtime.
    assert_eq!(eval_int(&lua, "return Q.rounds_to_seconds(0.01, 4)"), 1);
    // And nothing at all when either half is absent.
    assert_eq!(eval_int(&lua, "return Q.rounds_to_seconds(0, 4)"), 0);
    assert_eq!(eval_int(&lua, "return Q.rounds_to_seconds(1, 0)"), 0);
}

/// `{ rounds = n }` is multiplicative, which is why it is a branch and not a
/// desugar into the additive `scale`.
#[test]
fn a_rounds_amount_multiplies_the_characters_round() {
    let lua = queues_lua();

    assert_eq!(eval_int(&lua, "return AB.roll({ rounds = 0.75 }, { round_length = 4 })"), 3);
    assert_eq!(eval_int(&lua, "return AB.roll({ rounds = 1 }, { round_length = 4 })"), 4);
    assert_eq!(eval_int(&lua, "return AB.roll({ rounds = 1.25 }, { round_length = 4 })"), 5);

    // A different character, a different round — the same authored spec.
    assert_eq!(eval_int(&lua, "return AB.roll({ rounds = 0.75 }, { round_length = 8 })"), 6);

    // The proof it cannot be `scale`: scaling reads a trait off the *user* and
    // adds, so with a base of 0 it is 0 whatever the round length is.
    assert_eq!(
        eval_int(
            &lua,
            "return AB.roll({ min = 0, max = 0, scale = { trait = 'round_length', per = 0.75 } }, { round_length = 4 })"
        ),
        0,
        "the desugar could not have worked; scale is additive and off the user"
    );
}

/// A rounds component may itself be rolled, and a scale adds flat seconds.
#[test]
fn a_rounds_amount_composes_with_the_shapes_around_it() {
    let lua = queues_lua();

    // A range of rounds, low end pinned.
    assert_eq!(
        eval_int(
            &lua,
            "return AB.roll({ rounds = { min = 1, max = 3 } }, { round_length = 4 }, function(n) return 1 end)"
        ),
        4
    );
    // Flat seconds on top of a round.
    assert_eq!(
        eval_int(
            &lua,
            "return AB.roll({ rounds = 1, scale = { trait = 'haste', per = 1 } }, { round_length = 4, user = { trait = function() return -2 end } })"
        ),
        2,
        "one round of four, less two seconds of haste"
    );
}

/// The roundtime key is per track, so two tracks cannot gate each other.
#[test]
fn the_roundtime_key_is_per_track() {
    let lua = queues_lua();
    assert_eq!(eval_str(&lua, "return Q.rt_key('combat')"), "rt.combat");
    assert_eq!(eval_str(&lua, "return Q.rt_key('crafting')"), "rt.crafting");
    assert_eq!(eval_str(&lua, "return Q.rt_key(nil)"), "rt.combat");
    assert!(eval_bool(&lua, "return Q.rt_key('combat') ~= Q.rt_key('crafting')"));
}

/// A queue refuses past its bound rather than dropping what was promised.
#[test]
fn a_queue_refuses_the_newest_past_its_bound() {
    let lua = queues_lua();
    let out = eval_str(
        &lua,
        "local q = {} local opts = { max = 2 } local a = Q.push(q, { id = 'one' }, opts) local b = Q.push(q, { id = 'two' }, opts) local c = Q.push(q, { id = 'three' }, opts) return tostring(a) .. '|' .. tostring(b) .. '|' .. tostring(c) .. '|' .. #q .. '|' .. q[1].id",
    );
    assert_eq!(out, "true|true|false|2|one", "the newest is refused; the promised ones stay");
}

/// A front insert jumps the queue without displacing anything.
#[test]
fn a_front_insert_does_not_displace() {
    let lua = queues_lua();
    let out = eval_str(
        &lua,
        "local q = {} local opts = { max = 3 } Q.push(q, { id = 'one' }, opts) Q.push(q, { id = 'two' }, opts) Q.push(q, { id = 'jump' }, { max = 3, front = true }) local ids = {} for _, e in ipairs(q) do ids[#ids+1] = e.id end return table.concat(ids, ',') .. '|' .. Q.pop(q).id .. '|' .. #q",
    );
    assert_eq!(out, "jump,one,two|jump|2");
}

/// A stale entry is one the player would be surprised to see act.
#[test]
fn an_entry_goes_stale_rather_than_replaying_a_minute_later() {
    let lua = queues_lua();
    assert!(eval_bool(&lua, "return Q.is_stale({ at = 0 }, 45, 30)"));
    assert!(!eval_bool(&lua, "return Q.is_stale({ at = 0 }, 20, 30)"));
    // Zero disables it, and an entry with no timestamp never goes stale.
    assert!(!eval_bool(&lua, "return Q.is_stale({ at = 0 }, 9999, 0)"));
    assert!(!eval_bool(&lua, "return Q.is_stale({ id = 'x' }, 9999, 30)"));
}

/// History is newest-first, bounded, and carries no entity references.
#[test]
fn history_is_bounded_and_holds_no_entities() {
    let lua = queues_lua();
    let out = eval_str(
        &lua,
        "local h = {} for i = 1, 5 do Q.remember(h, { kind = 'ability', id = 'a' .. i, at = i, target = { id = 'mob:1' } }, 3) end local ids = {} local refs = 0 for _, e in ipairs(h) do ids[#ids+1] = e.id if e.target ~= nil then refs = refs + 1 end end return table.concat(ids, ',') .. '|' .. refs",
    );
    assert_eq!(out, "a5,a4,a3|0", "newest first, three kept, and the target did not come along");
}

/// A track fills itself out, and refuses a policy that is not one.
#[test]
fn a_track_normalises_and_refuses_a_bad_policy() {
    let lua = queues_lua();
    let out = eval_str(
        &lua,
        "local t = Q.normalise_track({ id = 'combat' }) return t.round_trait .. '|' .. t.round_seconds .. '|' .. t.max .. '|' .. t.empty",
    );
    assert_eq!(out, "round_length|3|3|idle");

    // A crafting track names its own round, and nothing is hardcoded.
    assert_eq!(
        eval_str(&lua, "return Q.normalise_track({ id = 'crafting', round_trait = 'craft_round_length' }).round_trait"),
        "craft_round_length"
    );

    assert_eq!(
        eval_str(&lua, "local _, e = Q.normalise_track({ id = 'x', empty = 'wander' }) return e"),
        "'wander' is not an empty-queue policy. One of: auto idle repeat"
    );
    assert_eq!(
        eval_str(&lua, "local _, e = Q.normalise_track({}) return e"),
        "a track needs a string id"
    );
}

/// `track` defaults to combat, and `none` opts out for ever.
#[test]
fn an_ability_declares_which_track_it_belongs_to() {
    let lua = queues_lua();
    assert_eq!(eval_str(&lua, "return tostring(AB.normalise({ id = 'a' }).track)"), "combat");
    assert_eq!(
        eval_str(&lua, "return tostring(AB.normalise({ id = 'b', track = 'crafting' }).track)"),
        "crafting"
    );
    assert_eq!(eval_str(&lua, "return tostring(AB.normalise({ id = 'c', track = 'none' }).track)"), "nil");
    assert_eq!(eval_str(&lua, "return tostring(AB.normalise({ id = 'd', track = false }).track)"), "nil");
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Combat — the resolution arithmetic
// ═══════════════════════════════════════════════════════════════════════════════

fn combat_lua() -> Lua {
    let lua = make_test_lua();
    lua.load("C = require('lib.combat')").exec().unwrap();
    lua
}

/// **The most important test in the change.** The default configuration
/// reproduces the one-line formula it replaced, exactly.
///
/// Combat was `clamp(60 + (a_dex - d_dex) * 3, 5, 95)`. The pipeline computes
/// `clamp(BASE + (A - D) * STEP, FLOOR, CEIL)` with those four as defaults, so a
/// game that configures nothing gets what it had — and there is no second code
/// path to drift from the first.
#[test]
fn the_default_threshold_reproduces_the_formula_it_replaced() {
    let lua = combat_lua();

    for (a, d) in [
        (10, 10), (12, 8), (8, 12), (20, 5), (5, 20),
        (0, 0), (50, 1), (1, 50), (14, 13), (7, 19),
    ] {
        let want = (60 + (a - d) * 3).clamp(5, 95);
        let got = eval_int(
            &lua,
            &format!("return C.threshold({a}, {{ {{ id = 'dodge', value = {d} }} }})"),
        );
        assert_eq!(got, want, "a={a} d={d}");
    }
}

/// The clamp holds at both ends, so nothing is ever certain either way.
#[test]
fn the_threshold_clamps_at_both_ends() {
    let lua = combat_lua();
    assert_eq!(
        eval_int(&lua, "return C.threshold(0, { { id = 'dodge', value = 999 } })"),
        5
    );
    assert_eq!(
        eval_int(&lua, "return C.threshold(999, { { id = 'dodge', value = 0 } })"),
        95
    );
}

/// Shares normalise over the **available** set, so an unusable channel costs
/// nothing rather than leaking effort.
#[test]
fn effort_redistributes_over_what_can_actually_be_used() {
    let lua = combat_lua();

    // All three available: 2/2/1 of five.
    let out = eval_str(
        &lua,
        "local s = C.shares({ dodge = 2, parry = 2, block = 1 }, { dodge = true, parry = true, block = true }) return string.format('%.2f|%.2f|%.2f', s.dodge, s.parry, s.block)",
    );
    assert_eq!(out, "0.40|0.40|0.20");

    // No shield: block's share goes to the two that remain, not to waste.
    let out = eval_str(
        &lua,
        "local s = C.shares({ dodge = 2, parry = 2, block = 1 }, { dodge = true, parry = true }) return string.format('%.2f|%.2f|%s', s.dodge, s.parry, tostring(s.block))",
    );
    assert_eq!(out, "0.50|0.50|nil", "not 0.40/0.40 with a fifth thrown away");

    // Nothing available is an empty table, not a division by zero.
    assert_eq!(
        eval_str(&lua, "local s = C.shares({ dodge = 1 }, {}) local n = 0 for _ in pairs(s) do n = n + 1 end return tostring(n)"),
        "0"
    );
}

/// The strongest channel is the one that answers, and ties break by id.
#[test]
fn the_strongest_channel_is_the_one_that_defends() {
    let lua = combat_lua();

    let out = eval_str(
        &lua,
        "local cs = C.channels({ dodge = 1, parry = 3 }, 20, { dodge = true, parry = true }) local _, ch, d = C.threshold(10, cs) return ch .. '|' .. string.format('%.0f', d)",
    );
    assert_eq!(out, "parry|15", "three quarters of twenty");

    // A tie is decided by name, never by `pairs` order.
    let out = eval_str(
        &lua,
        "local cs = C.channels({ dodge = 1, parry = 1 }, 20, { dodge = true, parry = true }) local _, ch = C.threshold(10, cs) return ch",
    );
    assert_eq!(out, "dodge");
}

/// An ability's multipliers move the threshold in the direction their names claim.
#[test]
fn an_attacks_multipliers_move_the_contest() {
    let lua = combat_lua();

    let plain = eval_int(&lua, "return C.threshold(10, { { id = 'dodge', value = 10 } })");
    assert_eq!(plain, 60);

    // Harder to dodge: the defender's rating is cut, so the attack lands more.
    let cut = eval_int(
        &lua,
        "local cs = C.channels({ dodge = 1 }, 10, { dodge = true }, { multipliers = { dodge = 0.5 } }) return C.threshold(10, cs)",
    );
    assert!(cut > plain, "a defence multiplier below one should help the attacker: {cut}");

    // And accuracy does the same from the other side.
    let out = eval_str(
        &lua,
        "local r = C.resolve({ accuracy = 10, accuracy_multiplier = 2, channels = { { id = 'dodge', value = 10 } } }, function(n) return 1 end) return r.threshold .. '|' .. tostring(r.hit)",
    );
    assert_eq!(out, "90|true");
}

/// The margin is out of 100, not out of the threshold.
///
/// A 95%-to-hit attack rolling 3 is a decisive blow — a skill differential
/// expressing itself. A 10%-to-hit attack rolling 3 is a graze — that is luck,
/// and luck should scrape. Dividing by the threshold inverts both.
#[test]
fn the_margin_is_out_of_a_hundred_so_luck_only_ever_grazes() {
    let lua = combat_lua();

    let skilled = eval_int(
        &lua,
        "return C.resolve({ accuracy = 99, channels = { { id = 'dodge', value = 0 } } }, function(n) return 3 end).margin",
    );
    let lucky = eval_int(
        &lua,
        "return C.resolve({ accuracy = 0, channels = { { id = 'dodge', value = 99 } } }, function(n) return 3 end).margin",
    );
    assert_eq!(skilled, 92, "95 threshold less a roll of 3");
    assert_eq!(lucky, 2, "a 5 threshold less a roll of 3 — a scrape");
    assert!(skilled > lucky * 10, "the two must not be close");
}

/// The shipped band table is one band worth exactly what a hit was worth before
/// degrees existed. Asserted explicitly so nobody "improves" it into a
/// compatibility break.
#[test]
fn the_shipped_degree_table_changes_no_damage() {
    let lua = combat_lua();
    assert_eq!(eval_int(&lua, "return #C.DEFAULT_DEGREES"), 1);
    // `string.format`, not `tostring`: 5.5 renders a float `1.0` and LuaJIT
    // renders `1`, and the assertion is about the value, not the printing.
    assert_eq!(eval_str(&lua, "return string.format('%.1f', C.DEFAULT_DEGREES[1].power)"), "1.0");
    assert_eq!(eval_int(&lua, "return C.DEFAULT_DEGREES[1].at"), 0);

    // And a resolve with no band table reports power 1.
    assert_eq!(
        eval_str(&lua, "return string.format('%.1f', C.resolve({ accuracy = 50, channels = {} }, function(n) return 1 end).power)"),
        "1.0"
    );
}

/// A band table is looked up from the top down.
#[test]
fn a_degree_band_is_the_highest_one_the_margin_reaches() {
    let lua = combat_lua();
    lua.load(
        "BANDS = C.normalise_degrees({ { id = 'graze', at = 0, power = 0.5 }, { id = 'solid', at = 30, power = 1.0 }, { id = 'decisive', at = 70, power = 1.75 } })",
    )
    .exec()
    .unwrap();

    // Sorted descending on normalise, whatever order they were written in.
    assert_eq!(eval_str(&lua, "return BANDS[1].id"), "decisive");

    for (margin, want) in [(0, "graze"), (29, "graze"), (30, "solid"), (69, "solid"), (70, "decisive"), (99, "decisive")] {
        assert_eq!(
            eval_str(&lua, &format!("return C.degree({margin}, BANDS).id")),
            want,
            "margin {margin}"
        );
    }
}

/// Damage composes the degree with where it landed.
#[test]
fn damage_composes_the_degree_and_the_location() {
    let lua = combat_lua();

    assert_eq!(eval_int(&lua, "return C.damage(10, { power = 1.0 })"), 10);
    assert_eq!(eval_int(&lua, "return C.damage(10, { power = 1.5 })"), 15);

    // A part multiplier and a proportional vulnerability compound.
    assert_eq!(
        eval_int(&lua, "return C.damage(10, { power = 1.0 }, { multiplier = 2 })"),
        20
    );
    assert_eq!(
        eval_int(
            &lua,
            "return C.damage(10, { power = 1.0 }, { vulnerable = { piercing = 0.25 } }, 'piercing')"
        ),
        12
    );
    // A vulnerability for another damage type does nothing.
    assert_eq!(
        eval_int(
            &lua,
            "return C.damage(10, { power = 1.0 }, { vulnerable = { piercing = 0.25 } }, 'blunt')"
        ),
        10
    );
}

/// A channel spec fills itself out and refuses what cannot be one.
#[test]
fn a_channel_spec_normalises() {
    let lua = combat_lua();
    assert_eq!(eval_str(&lua, "return C.normalise_channel('parry', {}).trait"), "defense_parry");
    assert_eq!(
        eval_str(&lua, "return C.normalise_channel('parry', { trait = 'blade_work' }).trait"),
        "blade_work"
    );
    assert_eq!(
        eval_str(&lua, "local _, e = C.normalise_channel('x', { available = 3 }) return e"),
        "a channel's `available` must be a function"
    );
    assert_eq!(
        eval_str(&lua, "local _, e = C.normalise_channel(nil, {}) return e"),
        "a channel needs a string id"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Pager — every path ends its output with a newline
// ═══════════════════════════════════════════════════════════════════════════════

/// A VM where `send` records what it was given instead of discarding it.
fn pager_lua() -> Lua {
    let lua = make_test_lua();
    lua.load(
        "SENT = {} \
         send = function(sid, text) SENT[#SENT+1] = tostring(text) end \
         send_prompt = function(sid, text) SENT[#SENT+1] = '<PROMPT>' .. tostring(text) end \
         P = require('daemons.pager_d')",
    )
    .exec()
    .unwrap();
    lua
}

/// **The bug this exists for.** Text shorter than a page went out unterminated,
/// so `send_prompt` — which adds no leading newline — landed on its last line.
///
/// It read worst in `olc`, which pages more than anything else and whose output
/// is almost always under a screen, so the paged path (which did terminate) was
/// almost never the one taken.
#[test]
fn short_output_ends_with_a_newline_so_the_prompt_clears_it() {
    let lua = pager_lua();
    lua.load("P.page('s1', 'one line')").exec().unwrap();

    assert_eq!(eval_int(&lua, "return #SENT"), 1);
    assert_eq!(eval_str(&lua, "return SENT[1]"), "one line\r\n");
}

/// The paged path already did this; the two must agree.
#[test]
fn a_full_page_ends_with_a_newline_too() {
    let lua = pager_lua();
    lua.load(
        "local lines = {} for i = 1, 30 do lines[i] = 'line ' .. i end \
         P.page('s1', table.concat(lines, '\\r\\n'), 10)",
    )
    .exec()
    .unwrap();

    assert!(
        eval_bool(&lua, "return SENT[1]:sub(-2) == '\\r\\n'"),
        "the first page must terminate: {}",
        eval_str(&lua, "return string.format('%q', SENT[1]:sub(-6))")
    );
}

/// The `--More--` prompt is the one thing that deliberately sits at the cursor.
#[test]
fn the_more_prompt_still_sits_at_the_cursor() {
    let lua = pager_lua();
    lua.load(
        "local lines = {} for i = 1, 30 do lines[i] = 'line ' .. i end \
         P.page('s1', table.concat(lines, '\\r\\n'), 10)",
    )
    .exec()
    .unwrap();

    // Body, then a prompt with no newline of its own.
    assert_eq!(eval_int(&lua, "return #SENT"), 2);
    assert!(eval_bool(&lua, "return SENT[2]:find('<PROMPT>', 1, true) == 1"));
    assert!(
        eval_bool(&lua, "return SENT[2]:sub(-1) ~= '\\n'"),
        "the More prompt is meant to sit at the cursor"
    );
}

/// Paging to the end terminates the last page and sends no further prompt.
#[test]
fn the_last_page_terminates_and_stops_prompting() {
    let lua = pager_lua();
    lua.load(
        "local lines = {} for i = 1, 15 do lines[i] = 'line ' .. i end \
         P.page('s1', table.concat(lines, '\\r\\n'), 10) \
         SENT = {} \
         P.handle_input('s1', '')",
    )
    .exec()
    .unwrap();

    assert_eq!(eval_int(&lua, "return #SENT"), 1, "the last page, and no More prompt");
    assert!(eval_bool(&lua, "return SENT[1]:sub(-2) == '\\r\\n'"));
    assert!(!eval_bool(&lua, "return P.is_paging('s1')"));
}
