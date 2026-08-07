//! Sparse traits, through the real VM.
//!
//! The claim under test is that **storage decides what an entity has**, rather
//! than the definition table deciding that everything has everything. Two
//! consequences follow, and both are asserted here rather than argued:
//!
//!  1. A sword has `dps` because it has damage and speed, and has no
//!     `willpower` because it has no `wisdom`. Nothing declares that.
//!  2. A recompute is O(traits this entity holds), not O(traits the game
//!     defines — which is the whole performance change.
//!
//! Presence and learning are covered in `tests/traits_effects.rs`, which came
//! first. What is here is the rest of the plan's verification list: `all()`
//! filtering, `category` as a lens that cannot change behaviour, which command
//! shows what, and the evaluation-count property.
//!
//! Per `CLAUDE.md`, everything goes through `tests/common/mod.rs`'s real
//! `ScriptEngine` — a helper called in isolation would answer a question about
//! a function rather than about what game code can do.

use crate::common::RealVm;

/// Reading a trait the entity does not have answers with the default and writes
/// nothing. Arithmetic stays safe; `has` answers the other question.
///
/// The "writes nothing" half is the one worth pinning: a `value` that
/// materialised on read would silently turn every entity dense again, and the
/// only symptom would be the performance regression this whole change exists to
/// avoid.
#[test]
fn an_absent_read_returns_the_default_and_stores_nothing() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();
    vm.eval("_T.define({ id = 'sp_luck', kind = 'attribute', default = 7 }); _T.seal()").unwrap();
    vm.eval("_e = { stats = {} }").unwrap();

    assert_eq!(vm.eval("return _T.value(_e, 'sp_luck')").unwrap(), "7");
    assert_eq!(
        vm.eval("return tostring(_e.stats.sp_luck)").unwrap(),
        "nil",
        "reading an absent trait materialised it on the entity"
    );
    assert_eq!(vm.eval("return _T.has(_e, 'sp_luck')").unwrap(), "false");
    assert_eq!(
        vm.eval("return #_T.present(_e)").unwrap(),
        "0",
        "a read should not have added anything to the present set"
    );
}

/// **The performance property, counted rather than timed.**
///
/// Define two hundred derived traits, hand an entity the two inputs one of them
/// needs, and count how many formulas actually run. A timing assertion would be
/// flaky; a call count is exact.
///
/// Before the present-set cache, `recompute` walked the global order, so this
/// number was 200 for every entity in the game, forever, growing with every
/// skill anyone ever authored.
#[test]
fn a_recompute_is_proportional_to_the_entity_not_the_registry() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval("_T = DAEMON.trait; _evals = 0").unwrap();

    // Two hundred derived traits, each over its own pair of inputs, so an
    // entity holding one pair can be present for exactly one of them.
    vm.eval(
        "for i = 1, 200 do \
           local a, b = 'perf_a' .. i, 'perf_b' .. i \
           _T.define({ id = a, kind = 'attribute', default = 1, sets = 'nobody' }) \
           _T.define({ id = b, kind = 'attribute', default = 1, sets = 'nobody' }) \
           _T.define({ id = 'perf_d' .. i, kind = 'derived', depends = { a, b }, \
             sets = 'nobody', formula = function(t) \
               _evals = _evals + 1 return t[a] + t[b] end }) \
         end; _T.seal()",
    )
    .unwrap();

    // Sanity: the registry really is large now.
    vm.eval("_n = 0; for _ in pairs(_T.defs()) do _n = _n + 1 end").unwrap();
    let defined: i64 = vm.eval("return _n").unwrap().parse().unwrap();
    assert!(defined >= 600, "expected 600+ definitions, got {defined}");

    // An entity holding exactly one pair. `perf_d7` is present because both its
    // dependencies are; the other 199 are not, and nothing had to say so.
    vm.eval("_small = { stats = { perf_a7 = 3, perf_b7 = 4 } }").unwrap();
    vm.eval("_evals = 0").unwrap();
    assert_eq!(vm.eval("return _T.value(_small, 'perf_d7')").unwrap(), "7");

    let evals: i64 = vm.eval("return _evals").unwrap().parse().unwrap();
    assert_eq!(
        evals, 1,
        "a full recompute of an entity holding one derived trait ran {evals} \
         formulas; walking the global order would run 200"
    );
    assert_eq!(
        vm.eval("return #_T.present(_small)").unwrap(),
        "3",
        "the entity holds two attributes and the one derived trait over them"
    );

    // Reading a trait it does not have still runs nothing extra: the answer is
    // the default, computed from no formula at all.
    vm.eval("_evals = 0").unwrap();
    assert_eq!(vm.eval("return _T.value(_small, 'perf_d100')").unwrap(), "0");
    assert_eq!(
        vm.eval("return _evals").unwrap(),
        "0",
        "reading an absent derived trait evaluated its formula"
    );
}

/// **`category` is a lens, not behaviour.**
///
/// The same trait, defined twice under two different categories, must compute
/// an identical value and settle identically. This is the test that stops
/// `category` quietly becoming a second `kind`: if adding a category can ever
/// change a number, the field has grown a meaning it was not supposed to have.
#[test]
fn a_category_cannot_change_what_a_trait_is_worth() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();

    // Identical in every way that could matter, different only in `category`.
    vm.eval(
        "for _, c in ipairs({ 'stat', 'reputation' }) do \
           _T.define({ id = 'lens_base_' .. c, kind = 'attribute', category = c, default = 9 }) \
           _T.define({ id = 'lens_derived_' .. c, kind = 'derived', category = c, \
             depends = { 'lens_base_' .. c }, round = 'floor', \
             formula = function(t) return t['lens_base_' .. c] * 3 + 1 end }) \
         end; _T.seal()",
    )
    .unwrap();

    vm.eval("_a = { stats = { lens_base_stat = 9 } }").unwrap();
    vm.eval("_b = { stats = { lens_base_reputation = 9 } }").unwrap();

    assert_eq!(vm.eval("return _T.value(_a, 'lens_derived_stat')").unwrap(), "28");
    assert_eq!(
        vm.eval("return _T.value(_b, 'lens_derived_reputation')").unwrap(),
        "28",
        "the same formula under a different category produced a different number"
    );

    // Presence is unaffected too: both are present for the same reason, and it
    // is not the category.
    assert_eq!(vm.eval("return #_T.present(_a)").unwrap(), "2");
    assert_eq!(vm.eval("return #_T.present(_b)").unwrap(), "2");

    // The only observable difference is which lens lists them.
    assert_eq!(vm.eval("return #_T.all(_a, 'stat')").unwrap(), "2");
    assert_eq!(vm.eval("return #_T.all(_a, 'reputation')").unwrap(), "0");
    assert_eq!(vm.eval("return #_T.all(_b, 'reputation')").unwrap(), "2");
    assert_eq!(vm.eval("return #_T.all(_b, 'stat')").unwrap(), "0");
}

// ═════════════════════════════════════════════════════════════════════════════
//  Command routing — through the real dispatcher, as a player meets it
// ═════════════════════════════════════════════════════════════════════════════
