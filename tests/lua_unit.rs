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

/// Create a Lua VM wired to require() from the real mudlib/ and game/ dirs,
/// with lightweight efun stubs so modules can load without crashing.
fn make_test_lua() -> Lua {
    let lua = Lua::new();
    let root = project_root();

    let mudlib = root.join("mudlib");
    let game   = root.join("game");

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

    // Empty DAEMON table
    lua.load("DAEMON = {}").exec().expect("init DAEMON");

    lua
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

/// Helper: run a Lua snippet and return a float result.
fn eval_num(lua: &Lua, code: &str) -> f64 {
    lua.load(code).eval::<f64>().unwrap()
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

#[test]
fn test_mobile_skills() {
    let lua = make_test_lua();
    lua.load("Mobile = require('lib.mobile')").exec().unwrap();
    lua.load(r#"mob = Mobile:new({ id = "t" })"#).exec().unwrap();

    assert_eq!(eval_int(&lua, r#"return mob:get_skill("swords")"#), 0);
    lua.load(r#"mob:set_skill("swords", 5)"#).exec().unwrap();
    assert_eq!(eval_int(&lua, r#"return mob:get_skill("swords")"#), 5);
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
    assert_eq!(eval_int(&lua, r#"return player:get_skill("magic")"#), 10);
    assert!(eval_bool(&lua, r#"return player:has_quest_flag("dragon_slain")"#));
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
    assert_eq!(eval_int(&lua, "return exported.skills.archery"), 7);
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

// ═══════════════════════════════════════════════════════════════════════════════
//  Codegen tests
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_codegen_generate_room() {
    let lua = make_test_lua();
    lua.load("CODEGEN = require('daemons.codegen_d')").exec().unwrap();
    lua.load(r#"
        source = CODEGEN.generate_room("test.lab", {
            short = "The Laboratory",
            description = "A room of science.",
            exits = { west = "test.entrance" },
            builder = "TestBuilder",
        })
    "#).exec().unwrap();

    let source = eval_str(&lua, "return source");
    assert!(source.contains("test.lab"), "should contain room id");
    assert!(source.contains("The Laboratory"), "should contain short");
    assert!(source.contains("A room of science"), "should contain description");
    assert!(source.contains("west"), "should contain exit direction");
    assert!(source.contains("TestBuilder"), "should contain builder name");
    assert!(source.contains("return {"), "should be a valid return table");

    // The generated source should be loadable
    lua.load(r#"
        local fn = load(source)
        loaded_data = fn()
    "#).exec().unwrap();
    assert_eq!(eval_str(&lua, "return loaded_data.id"), "test.lab");
    assert_eq!(eval_str(&lua, "return loaded_data.short"), "The Laboratory");
    assert_eq!(eval_str(&lua, r#"return loaded_data.exits.west"#), "test.entrance");
}

#[test]
fn test_codegen_generate_meta() {
    let lua = make_test_lua();
    lua.load("CODEGEN = require('daemons.codegen_d')").exec().unwrap();
    lua.load(r#"
        source = CODEGEN.generate_meta("test_area", {
            title = "Test Area",
            author = "Builder",
            status = "live",
        })
    "#).exec().unwrap();

    let source = eval_str(&lua, "return source");
    assert!(source.contains("test_area"));
    assert!(source.contains("Test Area"));
    assert!(source.contains("Builder"));

    // Should be loadable
    lua.load(r#"
        local fn = load(source)
        loaded_meta = fn()
    "#).exec().unwrap();
    assert_eq!(eval_str(&lua, "return loaded_meta.name"), "test_area");
}

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
    assert!(eval_bool(&lua, "return player.get_skill ~= nil"));
    assert!(eval_bool(&lua, "return player.has_item ~= nil"));

    // Player has its own methods
    assert!(eval_bool(&lua, "return player.to_save ~= nil"));
    assert!(eval_bool(&lua, "return player.award_xp ~= nil"));
    assert!(eval_bool(&lua, "return player.spend_gold ~= nil"));

    // Player inherits Object methods too
    assert!(eval_bool(&lua, "return player.get_state ~= nil"));
}

#[test]
fn test_weapon_inherits_item() {
    let lua = make_test_lua();
    lua.load(r#"
        Weapon = require('lib.weapon')
        w = Weapon:new({ id = "t", short = "Sword", damage = 10, damage_type = "slash" })
    "#).exec().unwrap();

    // Has Item methods
    assert!(eval_bool(&lua, "return w.is_equippable ~= nil"));
    assert!(eval_bool(&lua, "return w.has_tag ~= nil"));
    // damage is a {min,max} table when given a scalar
    assert_eq!(eval_int(&lua, "return w.damage.min"), 10);
    assert_eq!(eval_int(&lua, "return w.damage.max"), 10);
    assert_eq!(eval_str(&lua, "return w.damage_type"), "slash");
    // Weapon defaults to slot = "weapon"
    assert_eq!(eval_str(&lua, "return w.slot"), "weapon");
}

#[test]
fn test_armor_inherits_item() {
    let lua = make_test_lua();
    lua.load(r#"
        Armor = require('lib.armor')
        a = Armor:new({ id = "t", short = "Plate", defense = 15, slot = "chest" })
    "#).exec().unwrap();

    assert!(eval_bool(&lua, "return a.is_equippable ~= nil"));
    assert_eq!(eval_int(&lua, "return a.defense"), 15);
    assert_eq!(eval_str(&lua, "return a.slot"), "chest");
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
