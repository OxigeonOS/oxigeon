//! Run-to-completion assumptions in the mudlib, made explicit.
//!
//! The engine has always dispatched one command at a time, to completion, so
//! module-level guards were safe by construction. Suspending a dispatch — which
//! is what a yielding debug hook does on Lua 5.3+ — breaks that construction:
//! a flag left set by a paused coroutine is read by everyone else.
//!
//! These do not need a debugger to be meaningful. Each asserts that a guard is
//! scoped to the entity it is guarding rather than to the process, which is a
//! correctness property in its own right — a global flag meant settling *A* also
//! suppressed settling *B*, and that was already wrong.
//!
//! The failure mode in every case is a silent wrong answer, never a crash.

use crate::common::RealVm;

/// Two entities, one of them mid-settle. The other must still regenerate.
///
/// `trait_d` guards `touch` against re-entering itself through `value`. That
/// guard used to be one boolean, so *any* entity being settled suppressed
/// regeneration for every entity.
#[test]
fn one_entitys_settle_does_not_suppress_anothers_regeneration() {
    let mut vm = RealVm::boot_fixture_with_probe();

    // Two characters, both wounded, both with a regeneration anchor far enough
    // back to have earned points.
    vm.eval(
        "_a = { char_id = 9001, stats = {} }          _b = { char_id = 9002, stats = {} }          for _, e in ipairs({ _a, _b }) do            DAEMON.trait.seed(e, 'character')            DAEMON.trait.set_cur(e, 'hp', 5)            e.stats._at.hp = e.stats._at.hp - 60          end return 'ok'",
    )
    .unwrap();

    // Read B from *inside* A's settle. `regen_rate` is dispatched within
    // `touch`, and its handlers are game code that may read anything — which is
    // exactly how one entity's settle used to reach another's.
    vm.eval(
        "_b_seen = nil          DAEMON.effect.define({ id = 'itl_peek', label = 'Peek',            hooks = { regen_rate = { phase = 'add', fn = function(ev, ctx)              if _b_seen == nil then _b_seen = DAEMON.trait.value(_b, 'hp') end            end } } })          DAEMON.effect.apply(_a, 'itl_peek')          DAEMON.trait.touch(_a) return 'ok'",
    )
    .unwrap();

    let a_hp = vm.eval("return tostring(DAEMON.trait.value(_a, 'hp'))").unwrap();
    assert_ne!(a_hp, "5", "A should have regenerated: {a_hp}");

    // The number the handler saw. Under one global flag, B's settle was
    // suppressed because *A* was being settled, and this read 5.
    let b_seen = vm.eval("return tostring(_b_seen)").unwrap();
    assert_ne!(
        b_seen, "5",
        "B was read as unregenerated from inside A's settle — the guard is          global rather than per-entity (B={b_seen}, A={a_hp})"
    );
}

/// The pipeline depth cap and the settle guard are keyed per entity.
///
/// Deliberately *not* asserted here by leaving a guard set and checking another
/// entity: nothing in the mudlib can leak one today, because every handler is
/// `pcall`ed and every guard is cleared on the way out. Only a suspended
/// dispatch can, which needs a coroutine — so that assertion lives in
/// `tests/yield_pause.rs`, where one can actually be suspended.
///
/// What this pins is the weaker, still-useful property: two entities get the
/// same budget, so the cap has not quietly become shared.
#[test]
fn two_entities_each_get_the_full_pipeline_budget() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval(
        "_deep = { char_id = 9201, stats = {} }          _other = { char_id = 9202, stats = {} }          for _, e in ipairs({ _deep, _other }) do DAEMON.trait.seed(e, 'character') end          _ran = 0 return 'ok'",
    )
    .unwrap();

    // A handler that recurses into the same hook on the same entity, so the
    // depth cap is what stops it.
    vm.eval(
        "DAEMON.effect.define({ id = 'itl_nest', label = 'Nest',            hooks = { xp_gained = { phase = 'add', fn = function(ev, ctx)              _ran = _ran + 1              if _ran < 5 then DAEMON.effect.run(ctx.entity, 'xp_gained', { amount = 1 }) end            end } } })          DAEMON.effect.apply(_deep, 'itl_nest')          DAEMON.effect.apply(_other, 'itl_nest') return 'ok'",
    )
    .unwrap();

    vm.eval("DAEMON.effect.run(_deep, 'xp_gained', { amount = 1 }) return 'ok'")
        .unwrap();
    let after_first = vm.eval("return tostring(_ran)").unwrap();

    vm.eval("_ran = 0 DAEMON.effect.run(_other, 'xp_gained', { amount = 1 }) return 'ok'")
        .unwrap();
    assert_eq!(
        vm.eval("return tostring(_ran)").unwrap(),
        after_first,
        "the second entity ran a different number of levels than the first"
    );
}

/// The re-entrancy guard itself still works: it is scoped, not removed.
#[test]
fn a_pipeline_still_refuses_to_re_enter_itself() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval(
        "_e = { char_id = 9301, stats = {} } \
         DAEMON.trait.seed(_e, 'character') \
         _calls = 0 \
         DAEMON.effect.define({ id = 'itl_loop', label = 'Loop', \
           hooks = { heal_received = { phase = 'add', fn = function(ev, ctx) \
             _calls = _calls + 1 \
             DAEMON.effect.run(ctx.entity, 'heal_received', { amount = 1 }) \
           end } } }) \
         DAEMON.effect.apply(_e, 'itl_loop') \
         DAEMON.effect.run(_e, 'heal_received', { amount = 1 }) return 'ok'",
    )
    .unwrap();

    // Without the guard this is unbounded recursion; with it, exactly one pass.
    assert_eq!(
        vm.eval("return tostring(_calls)").unwrap(),
        "1",
        "a hook re-entering its own pipeline for the same entity must be refused"
    );
}
