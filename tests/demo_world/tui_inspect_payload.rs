//! The Inspect tab's Lua payload, run against a real booted mudlib.
//!
//! This compiles `src/bin/tui/inspect_payload.rs` itself rather than a copy, so
//! it asks what the TUI actually sends. A test against a hand-written fixture
//! would stay green while `DAEMON.trait.all` was renamed underneath it — the
//! failure shape `CLAUDE.md`'s testing section exists to prevent.
//!
//! The payload has to survive two limits in
//! `src/core/scripting/debugger/introspect.lua`: `MAX_STR = 256` per value and
//! `MAX_CHILDREN = 200` rows. Both are checked here against the real trait set.

#[path = "../../src/bin/tui/inspect_payload.rs"]
mod inspect_payload;


use crate::common::RealVm;
use inspect_payload::{expression, parse_row, Row};

/// `introspect.lua` truncates any single variable value at this many characters.
const MAX_STR: usize = 256;
/// …and stops expanding a table at this many rows.
const MAX_CHILDREN: usize = 200;

/// Build the row list into a global, then read it back one entry at a time.
/// The probe channel is line-oriented, so a multi-row answer cannot come back
/// in one reply — which is the same reason the real client pages it through
/// `variables` rather than asking for one big string.
fn rows_for(vm: &mut RealVm, target: &str) -> Vec<Row> {
    vm.eval(&format!("_rows = {}", expression(target)))
        .unwrap();
    let n: usize = vm.eval("return #_rows").unwrap().parse().unwrap();
    assert!(
        n <= MAX_CHILDREN,
        "{} rows exceeds introspect.lua's {}-row expansion limit",
        n,
        MAX_CHILDREN
    );

    (1..=n)
        .map(|i| {
            let raw = vm.eval(&format!("return _rows[{}]", i)).unwrap();
            assert!(
                raw.chars().count() <= MAX_STR,
                "row {} is {} chars, past introspect.lua's {}-char value limit: {}",
                i,
                raw.chars().count(),
                MAX_STR,
                raw
            );
            parse_row(&raw).unwrap_or_else(|| panic!("row {} did not parse: {:?}", i, raw))
        })
        .collect()
}

fn traits(rows: &[Row]) -> Vec<&inspect_payload::TraitRow> {
    rows.iter()
        .filter_map(|r| match r {
            Row::Trait(t) => Some(t),
            _ => None,
        })
        .collect()
}

/// A character with the real trait set seeded onto it.
fn character(vm: &mut RealVm) {
    vm.eval(
        "_p = { char_id = 4242, name = 'Probe', xp = 0, inventory = {}, \
                equipment = {}, quest_flags = {}, stats = {}, \
                send = function() end, message_room = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'")
        .unwrap();
    vm.eval("DAEMON.trait.attach(_p) DAEMON.trait.seed(_p, 'character') return 'seeded'")
        .unwrap();
}

#[test]
fn the_payload_reads_the_real_trait_set_off_a_real_character() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    character(&mut vm);

    let rows = rows_for(&mut vm, "_p");
    let traits = traits(&rows);
    assert!(
        !traits.is_empty(),
        "a seeded character must expose traits; got {:?}",
        rows
    );

    // The trait ids `game/traits/core.lua` defines, and that the pane names.
    for id in ["hp", "max_hp", "strength", "level"] {
        let t = traits
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("no `{}` among {:?}", id, traits));
        assert!(!t.value.is_empty(), "`{}` came back with no value", id);
        assert!(!t.kind.is_empty(), "`{}` came back with no kind", id);
    }
}

#[test]
fn a_derived_trait_reports_the_computed_value_not_the_stored_one() {
    // This is the reason the pane exists. `max_hp` is `kind = "derived"`: it
    // stores nothing and is computed from constitution and level, so the raw
    // table a debugger's variables pane shows has no answer for it at all.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    character(&mut vm);

    let rows = rows_for(&mut vm, "_p");
    let traits = traits(&rows);

    let max_hp = traits.iter().find(|t| t.id == "max_hp").expect("max_hp");
    assert_eq!(max_hp.kind, "derived");
    assert_eq!(
        max_hp.value,
        vm.eval("return tostring(_p:trait('max_hp'))").unwrap(),
        "the pane must agree with entity:trait()"
    );

    // And the stored table really has nothing under it — which is exactly what
    // makes reading `entity.stats[id]` the wrong answer.
    assert_eq!(
        vm.eval("return tostring(_p.stats.max_hp)").unwrap(),
        "nil",
        "a derived trait stores nothing; if this changes the pane's premise changed"
    );
}

#[test]
fn a_gauge_carries_the_trait_that_is_its_ceiling() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    character(&mut vm);

    let rows = rows_for(&mut vm, "_p");
    let traits = traits(&rows);
    let hp = traits.iter().find(|t| t.id == "hp").expect("hp");

    assert_eq!(hp.kind, "gauge");
    // A gauge is bounded by another trait, and the pane renders it as `/max`.
    assert!(
        !hp.max.is_empty() && hp.max != "nil",
        "hp should report its ceiling, got {:?}",
        hp.max
    );
}

#[test]
fn an_effect_shows_up_as_a_row_and_moves_the_trait_it_modifies() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    character(&mut vm);

    // Any effect the game layer defines will do; take the first one there is,
    // so this does not rot when the effect list is edited.
    let def = vm
        .eval("local k for id in pairs(DAEMON.effect.defs()) do k = k or id end return tostring(k)")
        .unwrap();
    assert_ne!(def, "nil", "the game layer defines no effects to test with");

    vm.eval(&format!(
        "DAEMON.effect.apply(_p, '{}') return 'applied'",
        def
    ))
    .unwrap();

    let rows = rows_for(&mut vm, "_p");
    let effects: Vec<_> = rows
        .iter()
        .filter_map(|r| match r {
            Row::Effect(e) => Some(e),
            _ => None,
        })
        .collect();

    assert!(
        effects.iter().any(|e| e.id == def),
        "applied `{}` but the payload reported {:?}",
        def,
        effects
    );
}

#[test]
fn a_trait_bearing_entity_that_is_not_a_character_also_works() {
    // A trait is any numeric datum on any entity, not a character statistic —
    // so the target box takes a mob or an item just as happily as `player`.
    // Presence is decided by storage: this entity holds one number, so the pane
    // must show that one and not the whole registry.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval("_thing = { stats = { strength = 14 } } return 'ok'")
        .unwrap();

    let rows = rows_for(&mut vm, "_thing");
    let traits = traits(&rows);

    let strength = traits
        .iter()
        .find(|t| t.id == "strength")
        .unwrap_or_else(|| panic!("no `strength` among {:?}", traits));
    assert_eq!(strength.value, "14");

    // The sparsity claim, stated as the thing it prevents: an entity that is
    // not a character must not list the character set.
    assert!(
        !traits.iter().any(|t| t.id == "hp"),
        "an entity holding no hp should not report one; got {:?}",
        traits.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    let defined: i64 = {
        vm.eval("_n = 0 for _ in pairs(DAEMON.trait.defs()) do _n = _n + 1 end return 'ok'")
            .unwrap();
        vm.eval("return _n").unwrap().parse().unwrap()
    };
    assert!(
        (traits.len() as i64) < defined,
        "{} rows for a one-number entity against {} defined traits is not sparse",
        traits.len(),
        defined
    );
}

#[test]
fn a_target_that_resolves_to_nothing_is_empty_rather_than_an_error() {
    // The user types this expression by hand, so a typo must not blank the
    // debugger — it has to come back as "no rows" and let them retype.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    let rows = rows_for(&mut vm, "_no_such_local");
    assert!(rows.is_empty());
}

#[test]
fn the_payload_is_a_single_line_because_the_evaluate_path_is_line_oriented() {
    let expr = expression("player");
    assert!(!expr.contains('\n'), "a newline would truncate the request");
    assert!(expr.contains("DAEMON.trait.all"));
    assert!(expr.contains("DAEMON.effect.active"));
    // Each daemon read is wrapped, so one broken daemon does not blank the pane.
    assert_eq!(expr.matches("pcall").count(), 2);
}
