//! Which field names a creature in a sentence.
//!
//! A creature carries two names and they answer different questions: `name` is
//! what you type to attack it, `short` is what it reads as in prose. The wisp is
//! `name = "wisp"` and `short = "a pale wisp"`.
//!
//! Which one belongs in a *message* is a decision about what kind of game this
//! is, not about how messages work:
//!
//! ```text
//! "short"   You hit a pale wisp for 9 damage.      roleplay
//! "name"    You hit wisp for 9 damage.             hack-and-slash
//! ```
//!
//! So it is `game.display_name_prefers`, and the default is `"name"` because
//! that is what combat did before the key existed.
//!
//! This file exists mostly to prove the plumbing: `display_name_prefers` is not
//! a typed field on `GameConfig`, it rides the `#[serde(flatten)] extra` map, and
//! "an unrecognised `[game]` key is readable from Lua like any other" is a claim
//! worth an assertion rather than a comment.

use crate::common::{RealVm, TestCtx};

/// Boot with one `[game]` key set, the way `config/server.toml` would.
fn with_preference(value: Option<&str>) -> RealVm {
    let mut extra = std::collections::HashMap::new();
    if let Some(v) = value {
        extra.insert(
            "display_name_prefers".to_string(),
            toml::Value::String(v.to_string()),
        );
    }
    RealVm::boot_fixture_with_probe_opts(TestCtx { game_extra: extra, ..Default::default() })
}

/// The name of a creature carrying both, under each setting.
fn naming(vm: &mut RealVm) -> String {
    vm.eval(
        "local R = require('lib.render') \
         local m = { id = 'mob:9', name = 'wisp', short = 'a pale wisp' } \
         return R.display_name(m)",
    )
    .unwrap()
}

/// Unset means `"name"` — what `combat_d` did before this key existed.
#[test]
fn the_default_is_name_which_is_what_combat_always_did() {
    let mut vm = with_preference(None);
    assert_eq!(vm.eval("return tostring(config('game.display_name_prefers'))").unwrap(), "nil");
    assert_eq!(naming(&mut vm), "wisp");
}

/// **The plumbing assertion.** An unrecognised `[game]` key reaches Lua.
#[test]
fn an_unrecognised_game_key_reaches_lua_and_changes_the_naming() {
    let mut vm = with_preference(Some("short"));
    assert_eq!(
        vm.eval("return tostring(config('game.display_name_prefers'))").unwrap(),
        "short",
        "GameConfig::extra should carry a key the driver has no opinion about"
    );
    assert_eq!(naming(&mut vm), "a pale wisp");
}

/// Setting it explicitly to `"name"` is the same as leaving it out.
#[test]
fn naming_it_explicitly_matches_the_default() {
    let mut vm = with_preference(Some("name"));
    assert_eq!(naming(&mut vm), "wisp");
}

/// Anything else falls back rather than breaking. A typo in a config file must
/// not stop the game naming things.
#[test]
fn a_nonsense_value_falls_back_to_the_default() {
    let mut vm = with_preference(Some("elbow"));
    assert_eq!(naming(&mut vm), "wisp");
}

/// Whichever is preferred, the other is the fallback — so a thing carrying only
/// one of the two still has a name under either setting.
#[test]
fn the_other_field_is_always_the_fallback() {
    for (preference, want_named_only, want_short_only) in
        [(Some("name"), "Wren", "a rat"), (Some("short"), "Wren", "a rat")]
    {
        let mut vm = with_preference(preference);
        assert_eq!(
            vm.eval("return require('lib.render').display_name({ name = 'Wren' })").unwrap(),
            want_named_only,
            "preference {preference:?}"
        );
        assert_eq!(
            vm.eval("return require('lib.render').display_name({ short = 'a rat' })").unwrap(),
            want_short_only,
            "preference {preference:?}"
        );
        assert_eq!(
            vm.eval("return require('lib.render').display_name({ id = 'x' })").unwrap(),
            "something",
            "and a thing with neither still renders"
        );
    }
}

/// It reaches a rendered line, which is the whole point of the setting.
#[test]
fn the_preference_reaches_an_authored_message() {
    let mut vm = with_preference(Some("short"));
    let out = vm
        .eval(
            "local R = require('lib.render') \
             return R.render('$Actor $actor.v(hit) $target.', \
               { actor = { char_id = 1, name = 'Wren', gender = 'female' }, \
                 target = { id = 'mob:9', name = 'wisp', short = 'a pale wisp' } }, nil)",
        )
        .unwrap();
    assert_eq!(out, "Wren hits a pale wisp.");
}
