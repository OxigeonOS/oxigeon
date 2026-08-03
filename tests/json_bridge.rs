//! `lua_to_json` is the only path a Lua table takes to the database
//! (`save_character_data`) and to the wire (`send_gmcp`). It used to fail
//! silently in four different ways on that path:
//!
//! - a table with both list entries and string keys lost the string keys
//! - NaN and infinity became `0`
//! - functions became `null`, and keys that were neither string nor integer
//!   simply vanished
//! - a self-referential table recursed until the Rust stack was gone, taking
//!   the process with it — no Lua error, nothing in the log
//!
//! Every one of those is a player's data quietly being wrong, which is exactly
//! what `CLAUDE.md` forbids. These pin the loud behaviour that replaced them.
//!
//! This tests the function directly rather than through `RealVm`, which is
//! legitimate here: it is a pure function with no engine state behind it, and
//! `tests/document_efuns.rs` covers the reachable-from-Lua half.

use mlua::prelude::*;
use oxigeon::core::scripting::efuns::{json_to_lua, lua_to_json};

/// Evaluate `src` and convert whatever it returns.
fn convert(src: &str) -> LuaResult<serde_json::Value> {
    let lua = Lua::new();
    let value: LuaValue = lua.load(src).eval()?;
    lua_to_json(&lua, &value)
}

fn err_of(src: &str) -> String {
    match convert(src) {
        Ok(v) => panic!("expected a refusal, got {v}"),
        Err(e) => e.to_string(),
    }
}

// ─── the four silent failures ────────────────────────────────────────────────

/// The one that cost real data: `{"sword", "shield", gold = 100}` went into
/// every character save as `["sword","shield"]`.
#[test]
fn a_table_that_is_both_a_list_and_a_map_is_refused_by_name() {
    let msg = err_of(r#"return {"sword", "shield", gold = 100}"#);
    assert!(msg.contains("'gold'"), "the error must name the key at risk: {msg}");
    assert!(msg.contains("2 list entries"), "and say what it clashed with: {msg}");
}

/// The failure has to name the field, not just the fact. A character save is a
/// dozen fields deep and "conversion failed" is not actionable.
#[test]
fn the_refusal_says_which_field_is_at_fault() {
    let msg = err_of(r#"return { stats = { level = 3 }, custom = { bag = {1, 2, cap = 9} } }"#);
    assert!(
        msg.contains("custom.bag"),
        "expected a path to the offending table, got: {msg}"
    );
}

#[test]
fn nan_and_infinity_are_refused_instead_of_becoming_zero() {
    for src in ["return 0/0", "return 1/0", "return -1/0"] {
        let msg = err_of(src);
        assert!(
            msg.contains("no JSON representation"),
            "{src} should be refused, got: {msg}"
        );
    }
}

#[test]
fn a_function_is_refused_instead_of_becoming_null() {
    let msg = err_of("return { on_tick = function() end }");
    assert!(msg.contains("function"), "got: {msg}");
    assert!(msg.contains("on_tick"), "and it should name the field: {msg}");
}

#[test]
fn a_key_that_is_neither_string_nor_integer_is_refused_instead_of_vanishing() {
    let msg = err_of("local t = {} t[1.5] = 'x' return t");
    assert!(msg.contains("key of type"), "got: {msg}");
}

/// This one used to kill the process, so it is the most important of the five.
#[test]
fn a_self_referential_table_raises_rather_than_exhausting_the_stack() {
    let msg = err_of("local t = {} t.self = t return t");
    assert!(msg.contains("nesting is deeper"), "got: {msg}");
}

#[test]
fn a_mutually_referential_pair_raises_too() {
    let msg = err_of("local a, b = {}, {} a.b = b b.a = a return a");
    assert!(msg.contains("nesting is deeper"), "got: {msg}");
}

/// A table that is shallow but enormous, or a list so sparse that filling its
/// holes would allocate wildly, has to be bounded too.
#[test]
fn a_pathologically_sparse_list_hits_the_node_budget_rather_than_allocating() {
    let msg = err_of("local t = {} t[1] = 1 t[5000000] = 2 return t");
    assert!(msg.contains("values, giving up"), "got: {msg}");
}

// ─── what must keep working ──────────────────────────────────────────────────

#[test]
fn ordinary_shapes_still_convert() {
    assert_eq!(convert("return 42").unwrap(), serde_json::json!(42));
    assert_eq!(convert("return 1.5").unwrap(), serde_json::json!(1.5));
    assert_eq!(convert("return true").unwrap(), serde_json::json!(true));
    assert_eq!(convert("return 'hi'").unwrap(), serde_json::json!("hi"));
    assert_eq!(
        convert("return {1, 2, 3}").unwrap(),
        serde_json::json!([1, 2, 3])
    );
    assert_eq!(
        convert("return { name = 'Aldric', level = 7 }").unwrap(),
        serde_json::json!({"name": "Aldric", "level": 7})
    );
}

/// Neither Lua nor JSON can tell an empty list from an empty map, so `{}`
/// becomes an object. Documented rather than fixed — there is no right answer.
#[test]
fn an_empty_table_becomes_an_empty_object() {
    assert_eq!(convert("return {}").unwrap(), serde_json::json!({}));
}

/// A list with a hole in it is a real shape in Lua (`t[3] = nil` on a
/// three-item inventory), so it must survive. Holes become `null`, which
/// `json_to_lua` skips on the way back — so the round trip is exact.
#[test]
fn a_list_with_a_hole_survives_the_round_trip() {
    let json = convert("local t = {'a', 'b', 'c'} t[2] = nil return t").unwrap();
    assert_eq!(json, serde_json::json!(["a", null, "c"]));

    let lua = Lua::new();
    let back = json_to_lua(&lua, &json).unwrap();
    lua.globals().set("back", back).unwrap();
    assert_eq!(lua.load("return back[1]").eval::<String>().unwrap(), "a");
    assert_eq!(lua.load("return tostring(back[2])").eval::<String>().unwrap(), "nil");
    assert_eq!(lua.load("return back[3]").eval::<String>().unwrap(), "c");
}

/// Non-positive integer keys are a map, not a list, and used to be rendered as
/// string keys. Keep that — changing it would break stored data.
#[test]
fn non_positive_integer_keys_are_still_object_keys() {
    assert_eq!(
        convert("local t = {} t[0] = 'zero' t[-1] = 'neg' return t").unwrap(),
        serde_json::json!({"0": "zero", "-1": "neg"})
    );
}

/// Nesting well inside the limit must not trip it.
#[test]
fn deep_but_bounded_nesting_is_fine() {
    let src = "local t = {} local c = t for _ = 1, 50 do c.n = {} c = c.n end c.leaf = 1 return t";
    assert!(convert(src).is_ok());
}

/// The shape a character save actually has: a map of fields whose values are
/// lists and nested maps.
#[test]
fn a_realistic_character_save_round_trips() {
    let src = r#"return {
        stats = { hp = 42, max_hp = 50, level = 3 },
        inventory = { "sword", "lantern" },
        quest_flags = { manasteel_taken = true },
        gold = 120,
        channels = { "chat", "newbie" },
        custom = {},
    }"#;
    let json = convert(&src.replace('\n', " ")).unwrap();
    assert_eq!(json["stats"]["hp"], serde_json::json!(42));
    assert_eq!(json["inventory"], serde_json::json!(["sword", "lantern"]));
    assert_eq!(json["custom"], serde_json::json!({}));

    let lua = Lua::new();
    let back = json_to_lua(&lua, &json).unwrap();
    lua.globals().set("d", back).unwrap();
    assert_eq!(lua.load("return d.stats.hp").eval::<i64>().unwrap(), 42);
    assert_eq!(lua.load("return d.inventory[2]").eval::<String>().unwrap(), "lantern");
    assert_eq!(lua.load("return d.gold").eval::<i64>().unwrap(), 120);
}
