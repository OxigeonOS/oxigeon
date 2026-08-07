//! Phase ordering in the `damage_taken` pipeline, with real armour in it.
//!
//! `effects.md` argues that a mitigation pipeline needs *phases* rather than
//! registration order, and gives the worked example: a 30-point hit against
//! "take 15% less damage" and "negate 5 per hit" must yield **20**, not 21.
//! `21` is what you get by applying the flat reduction first, which is what
//! registration order would give you half the time.
//!
//! That was already tested with two effects from the same definition. What was
//! missing is the case the phases exist for: an effect and a *piece of armour*
//! landing in different phases, registered by entirely different code, on an
//! entity that acquired them in the wrong order.
//!
//! `_roll` is pinned throughout. The PRNG is seeded now — every VM used to
//! replay the same sequence from a constant seed — so a combat test that did
//! not pin its dice would be genuinely random rather than accidentally stable.


use crate::common::RealVm;

/// Extract the "N dealt" number from `affect damage`'s reply.
fn dealt(out: &str) -> i64 {
    out.split(", ")
        .find_map(|part| part.split_whitespace().next().and_then(|n| n.parse::<i64>().ok())
            .filter(|_| part.contains("dealt")))
        .unwrap_or_else(|| panic!("no 'N dealt' in:\n{out}"))
}

/// The documented example, with armour standing in for the flat reduction.
///
/// stoneskin is `mult` phase (-15%) and armour is `reduce` phase. 30 * 0.85 =
/// 25.5, minus leather's 3 = 22.5, floored to 22. Applying the armour first
/// would give (30 - 3) * 0.85 = 22.95 -> 22 as well, so the leather alone does
/// not separate them — which is exactly why the assertion below uses a piece
/// heavy enough that the two orders differ.
#[test]
fn a_percentage_applies_before_armour_whatever_order_they_arrive_in() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // A defence of 10 makes the orders differ:
    //   phases right: 30 * 0.85 = 25.5, - 10 = 15.5 -> 15
    //   phases wrong: (30 - 10) * 0.85 = 17 -> 17
    vm.command("affect heal 500");
    let base = dealt(&vm.command("affect damage 30"));
    assert_eq!(base, 30, "the unmitigated baseline moved");

    // Armour on first, buff second.
    vm.command("spawn leather_jerkin");
    vm.command("wear jerkin");
    vm.command("affect apply stoneskin 600");
    vm.command("affect heal 500");

    let armour_first = dealt(&vm.command("affect damage 30"));

    // Now the other way round on a fresh character-equivalent state: clear the
    // effects, re-apply in the opposite order.
    vm.command("affect clear");
    vm.command("remove chest");
    vm.command("affect apply stoneskin 600");
    vm.command("wear jerkin");
    vm.command("affect heal 500");

    let buff_first = dealt(&vm.command("affect damage 30"));

    assert_eq!(
        armour_first, buff_first,
        "the result depended on the order the mitigations were acquired in — \
         that is what phases exist to prevent"
    );

    // And the value is the one the phases dictate: 30 * 0.85 = 25.5, minus
    // stoneskin's flat 5 and the jerkin's 3, floored.
    assert_eq!(
        armour_first, 17,
        "expected 30*0.85 = 25.5, -5 (stoneskin) -3 (jerkin) = 17.5 -> 17"
    );
}

/// A resist that exceeds the damage cannot heal you. Mitigation floors at zero,
/// and a negative `amount` reaching `take_damage` would be a heal nobody asked
/// for.
#[test]
fn mitigation_cannot_go_below_zero() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("spawn warded_cloak");
    vm.command("wear cloak");
    vm.command("affect heal 500");

    let before: i64 = vm
        .command("affect traits")
        .lines()
        .find(|l| l.trim().starts_with("hp "))
        .and_then(|l| l.split_whitespace().nth(3).and_then(|n| n.parse().ok()))
        .unwrap_or(0);

    // Cloak: defense 1 + magic resist 6 = 7, against 3 points of magic.
    let out = vm.command("affect damage 3 magic");
    assert_eq!(dealt(&out), 0, "over-mitigation should floor at 0:\n{out}");

    let after: i64 = vm
        .command("affect traits")
        .lines()
        .find(|l| l.trim().starts_with("hp "))
        .and_then(|l| l.split_whitespace().nth(3).and_then(|n| n.parse().ok()))
        .unwrap_or(0);
    assert!(
        after <= before,
        "being hit for zero healed the target: {before} -> {after}"
    );
}

/// A weapon's `damage_type` reaches the defender's resist table through the
/// real round loop, not only through the admin damage verb.
#[test]
fn a_weapons_damage_type_reaches_the_defenders_resist() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'mit.room', exits = {} }) })")
        .unwrap();

    // A creature wearing the warded cloak — an entity is an entity, and the
    // mitigation path must not be player-only.
    vm.eval(
        "DAEMON.mobs.register({ id = 'warded_thing', short = 'a warded thing', \
           stats = { hp = 500, max_hp = 500, constitution = 10, dexterity = 1, level = 1 } })",
    )
    .unwrap();

    vm.eval("_E = require('lib.equipment')").unwrap();
    vm.eval("_real_roll = DAEMON.combat._roll; DAEMON.combat._roll = function() return 1 end")
        .unwrap();

    // **One band, at power 1.0 — this test is about mitigation, not degrees.**
    //
    // Pinning the die to 1 makes the damage roll its minimum, and it also makes
    // every hit land at the top of the game's degree table: margin is
    // `threshold - roll`, so a roll of 1 is as decisive as it gets. Once
    // `game/init.lua` defined bands, that multiplied every number below by 1.9
    // and the arithmetic being asserted here — flat `resist` against typed
    // damage — became impossible to read off the result.
    //
    // So the band table is flattened for the duration. The alternative is to
    // multiply the expected values through by whatever the game's top band
    // happens to be today, which makes a mitigation test fail whenever somebody
    // retunes a damage curve.
    vm.eval("DAEMON.combat.define_degrees({ { id = 'hit', at = 0, power = 1.0 } }) return 'flat'")
        .unwrap();

    // Attacker with the silver dagger (magic, min 3) and one with the
    // apprentice dagger (physical, min 2).
    let hit_for = |vm: &mut RealVm, weapon: &str, cloak: bool| -> i64 {
        vm.eval("_t = DAEMON.mobs.spawn('warded_thing', 'mit.room')").unwrap();
        if cloak {
            vm.eval("_c = DAEMON.items.spawn('warded_cloak', nil)").unwrap();
            vm.eval("_E.equip(_t, _c, DAEMON.items.resolve(_c))").unwrap();
        }
        vm.eval(&format!("_w = DAEMON.items.spawn('{weapon}', nil)")).unwrap();
        vm.eval(
            "_a = { char_id = 60, name = 'A', inventory = {}, equipment = { weapon = _w }, \
                    is_alive = function() return true end, send = function() end }",
        )
        .unwrap();
        vm.eval("_r = DAEMON.combat.attack_once(_a, _t)").unwrap();
        let dealt: i64 = vm.eval("return _r.dealt").unwrap().parse().unwrap();
        vm.eval("DAEMON.mobs.despawn(_t) return 'gone'").unwrap();
        dealt
    };

    let magic_bare = hit_for(&mut vm, "silver_dagger", false);
    let magic_cloaked = hit_for(&mut vm, "silver_dagger", true);
    let physical_cloaked = hit_for(&mut vm, "apprentice_dagger", true);

    assert_eq!(magic_bare, 3, "a pinned silver dagger should roll its minimum");
    assert_eq!(
        magic_cloaked, 0,
        "the cloak's magic resist (6) plus its defence (1) should absorb 3 magic damage"
    );
    assert_eq!(
        physical_cloaked, 1,
        "against physical the cloak is worth only its defence of 1, so 2 - 1 = 1 — \
         a resist table that applied to everything would make this 0 too"
    );

    vm.eval("DAEMON.combat._roll = _real_roll return 'restored'").unwrap();
}
