//! Experience becoming levels.
//!
//! `Player:award_xp` accumulated experience and **nothing ever read it**:
//! `level` was a counter that started at 1 and stayed there. Everything gated on
//! it was therefore unreachable by an ordinary player — the quest chain, the
//! silver dagger, three of the four spells — and `player.levelup` was a
//! documented event with no emitter.
//!
//! The curve is game policy, so `level_d` lives in `game/daemons/` and listens
//! to `player.xp_gained` exactly as `aggro_d` listens to `room.entered`.

mod common;

use common::RealVm;

/// The table and the level it implies, at the boundaries.
#[test]
fn the_curve_is_a_table_and_it_reads_as_one() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    for (xp, expected) in [
        (0, 1),
        (99, 1),
        (100, 2),      // exactly on a threshold counts
        (249, 2),
        (250, 3),
        (449, 3),
        (450, 4),
        (3_200, 10),
        (8_700, 15),
    ] {
        assert_eq!(
            vm.eval(&format!("return DAEMON.level.level_for({xp})")).unwrap(),
            expected.to_string(),
            "{xp} experience should be level {expected}"
        );
    }

    // Past the end of the table the last gap repeats, so the curve does not
    // stop and nothing has to special-case the top.
    assert_eq!(vm.eval("return DAEMON.level.threshold(16)").unwrap(), "10200");
    assert_eq!(vm.eval("return DAEMON.level.threshold(17)").unwrap(), "11700");
    assert_eq!(
        vm.eval("return DAEMON.level.level_for(11700)").unwrap(),
        "17",
        "the tail has to be consistent in both directions"
    );
}

/// Awarding experience raises the level, and the derived ceilings move with it.
#[test]
fn experience_raises_the_level_and_the_ceilings() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let level = |vm: &mut RealVm| -> i64 {
        vm.command("affect traits")
            .lines()
            .find(|l| l.trim_start().starts_with("level "))
            .and_then(|l| l.split_whitespace().nth(3).and_then(|n| n.parse().ok()))
            .unwrap_or(-1)
    };
    let max_hp = |vm: &mut RealVm| -> i64 {
        vm.command("affect traits")
            .lines()
            .find(|l| l.trim_start().starts_with("max_hp "))
            .and_then(|l| l.split_whitespace().nth(3).and_then(|n| n.parse().ok()))
            .unwrap_or(-1)
    };

    assert_eq!(level(&mut vm), 1);
    assert_eq!(max_hp(&mut vm), 100, "50 + con 10 * 5 + (level 1 - 1) * 10");

    let out = vm.command("affect xp 120");
    assert!(out.contains("now level 2"), "no level-up message:\n{out}");
    assert_eq!(level(&mut vm), 2);
    assert_eq!(
        max_hp(&mut vm),
        110,
        "max_hp is derived from level, so the ceiling should have moved"
    );

    // Several levels at once are announced one at a time — "you are now level
    // 4" alone hides that you passed 3.
    let out = vm.command("affect xp 400");
    assert!(out.contains("now level 3"), "{out}");
    assert!(out.contains("now level 4"), "{out}");
    assert_eq!(level(&mut vm), 4);
}

/// Levelling refills the gauges, because the ceilings just moved.
#[test]
fn levelling_up_fills_the_gauges() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("affect damage 60");
    let hurt: i64 = vm
        .command("affect traits")
        .lines()
        .find(|l| l.trim_start().starts_with("hp "))
        .and_then(|l| l.split_whitespace().nth(3).and_then(|n| n.parse().ok()))
        .unwrap_or(-1);
    assert!(hurt < 100, "the damage did not land: {hurt}");

    vm.command("affect xp 120");
    let healed: i64 = vm
        .command("affect traits")
        .lines()
        .find(|l| l.trim_start().starts_with("hp "))
        .and_then(|l| l.split_whitespace().nth(3).and_then(|n| n.parse().ok()))
        .unwrap_or(-1);
    assert_eq!(healed, 110, "a level-up should fill to the new maximum");
}

/// `reconcile` computes the level from the experience rather than
/// incrementing, so it is safe to call at any time — which is what makes the
/// login catch-up correct for a character saved before this daemon existed.
#[test]
fn reconciling_is_idempotent_and_never_goes_backwards() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 990, name = 'Late', xp = 700, inventory = {}, \
                equipment = {}, quest_flags = {}, \
                stats = { level = 1, hp = 20, mp = 10, constitution = 10, \
                          intelligence = 10, wisdom = 10, strength = 10, \
                          dexterity = 10 }, \
                send = function() end, message_room = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();
    vm.eval("DAEMON.trait.attach(_p) return 'attached'").unwrap();

    // 700 experience on a level-1 character: one call lands on 5, not on 2.
    assert_eq!(vm.eval("return DAEMON.level.reconcile(_p)").unwrap(), "5");
    assert_eq!(vm.eval("return _p:trait('level')").unwrap(), "5");

    // A second call changes nothing and reports nothing, which is what makes it
    // safe on every login.
    assert_eq!(
        vm.eval("return tostring(DAEMON.level.reconcile(_p))").unwrap(),
        "nil"
    );

    // Losing experience does not take a level back. A character whose gear and
    // quests assume level 5 becoming level 4 is a worse outcome than the
    // inconsistency.
    vm.eval("_p.xp = 0 return 'drained'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.level.reconcile(_p))").unwrap(),
        "nil"
    );
    assert_eq!(vm.eval("return _p:trait('level')").unwrap(), "5");
}

/// `player.levelup` — a documented event that had no emitter.
#[test]
fn levelling_emits_the_documented_event() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_heard = nil DAEMON.event.on('player.levelup', 'test', \
         function(d) _heard = d.from .. '->' .. d.new_level end) return 'listening'",
    )
    .unwrap();

    vm.eval(
        "_p = { char_id = 991, name = 'Riser', xp = 0, inventory = {}, \
                equipment = {}, quest_flags = {}, \
                stats = { level = 1, hp = 100, constitution = 10 }, \
                send = function() end, message_room = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();
    vm.eval("DAEMON.trait.attach(_p) _p.xp = 260 return 'ok'").unwrap();
    vm.eval("DAEMON.level.reconcile(_p) return 'reconciled'").unwrap();

    assert_eq!(
        vm.eval("return tostring(_heard)").unwrap(),
        "1->3",
        "the event should carry where the character came from as well as where \
         they arrived"
    );
}

/// `player.login` and `player.logout` are emitted. Both were documented event
/// names with no emitter, which is the same shape `room.entered` had.
#[test]
fn login_and_logout_are_announced() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    for event in ["player.login", "player.logout"] {
        vm.eval(&format!(
            "DAEMON.event.on('{event}', 'probe', function() end) return 'ok'"
        ))
        .unwrap();
    }

    // The emitters exist where they should: login's in the login flow, logout's
    // in the disconnect chain. Asserted by grep-through-Lua rather than by
    // driving a login, because driving one needs an Argon2 round trip the probe
    // harness cannot answer.
    assert_eq!(
        vm.eval(
            "local f = read_file('login.lua') \
             return tostring(f ~= nil and f:find('player.login', 1, true) ~= nil)"
        )
        .unwrap(),
        "true",
        "login.lua should emit player.login"
    );
    assert_eq!(
        vm.eval(
            "local f = read_file('init.lua') \
             return tostring(f ~= nil and f:find('player.logout', 1, true) ~= nil)"
        )
        .unwrap(),
        "true",
        "init.lua's disconnect chain should emit player.logout"
    );
}

/// The level gates in the world are reachable, which is the point of any of
/// this: without levelling they were not.
#[test]
fn the_level_gated_content_becomes_reachable() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // Level 1: two spells, and the level-3 one is not among them.
    let known = vm.command("cast");
    assert!(known.contains("emberlance"), "{known}");
    assert!(known.contains("mend"), "{known}");
    assert!(!known.contains("wardskin"), "wardskin is level 3:\n{known}");

    // Earn it rather than granting it.
    vm.command("affect xp 450");
    let known = vm.command("cast");
    assert!(known.contains("farsight"), "farsight is level 2:\n{known}");
    assert!(known.contains("wardskin"), "wardskin is level 3:\n{known}");

    // And the level-3 weapon can be wielded.
    vm.command("spawn silver_dagger");
    let out = vm.command("wield silver dagger");
    assert!(
        out.contains("You wield"),
        "a level-4 character should be able to hold a level-3 weapon:\n{out}"
    );
}
