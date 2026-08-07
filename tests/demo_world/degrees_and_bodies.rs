//! Degrees of success and hit locations, now that this game feeds them.
//!
//! Both were built, wired, and inert. `combat_d` computed `margin` on every
//! swing and looked it up in a band table the mudlib ships one entry of, so
//! every hit came back at power 1.0 and the margin was discarded. `Body.locate`
//! ran on every swing and returned nil, because no creature in the game named a
//! layout and there was no layout to name.
//!
//! Neither of those failed anything. That is the reason for this file: a system
//! that is switched off looks exactly like a system that is working, right up
//! until somebody deletes it as dead code.

use crate::common::RealVm;

/// A VM with a player, a target, and the die under control.
///
/// `hit` is what a d100 comes up as — low is good, so it selects the degree
/// band. Everything else returns its maximum, which makes the damage roll
/// deterministic without also pinning the to-hit.
fn arena(hit: u32) -> RealVm {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval(&format!(
        "local Player = require('lib.player') \
         p = Player:from_save(1, {{ name = 'Probe', account_id = 1 }}, {{}}) \
         target = DAEMON.mobs.spawn('town_guard', 'thornhollow.square') \
         DAEMON.combat._roll = function(n) \
             if n == 100 then return {hit} else return n end end \
         return tostring(target ~= nil)"
    ))
    .unwrap();
    vm
}

/// One swing's degree, power and damage.
fn swing(vm: &mut RealVm) -> (String, f64, i64) {
    let out = vm
        .eval(
            "DAEMON.trait.set_cur(target, 'hp', target:trait('max_hp')) \
             local r = DAEMON.combat.attack_once(p, target, {}) \
             return tostring(r.degree) .. '|' \
                 .. string.format('%.2f', r.power or 0) .. '|' .. tostring(r.dealt or 0)",
        )
        .unwrap();
    let f: Vec<&str> = out.split('|').collect();
    (f[0].to_string(), f[1].parse().unwrap(), f[2].parse().unwrap())
}

/// The game defines more than one band, so a margin means something.
#[test]
fn this_game_defines_a_degree_table() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let ids = vm
        .eval(
            "local out = {} \
             for _, b in ipairs(DAEMON.combat.degrees()) do out[#out + 1] = b.id end \
             return table.concat(out, ',')",
        )
        .unwrap();

    assert!(
        ids.contains(','),
        "only one band is registered, so `margin` is computed on every swing and \
         thrown away — which is indistinguishable from the mudlib default: {ids}"
    );
    for band in ["graze", "hit", "solid", "decisive"] {
        assert!(ids.contains(band), "'{band}' is missing from the band table: {ids}");
    }
}

/// **A wide margin hurts more than a narrow one**, which is the entire point.
///
/// Rolling 1 against a threshold near 60 is a margin near 60; rolling just under
/// the threshold is a margin near zero. The first is a skill differential
/// expressing itself and the second is luck — and the margin is deliberately
/// *not* divided by the threshold, so luck cannot produce a decapitation.
#[test]
fn a_wide_margin_hits_harder_than_a_narrow_one() {
    let mut vm = arena(1);
    let (best_degree, best_power, best_damage) = swing(&mut vm);

    // **The narrow roll is read off the contest, not guessed.** It has to be the
    // largest roll that still hits — one more and this measures a miss rather
    // than a graze, which is how the first version of this test failed.
    let threshold: u32 = vm
        .eval("return tostring(DAEMON.combat.attack_once(p, target, {}).threshold)")
        .unwrap()
        .parse()
        .unwrap();

    // Same VM, same target, same damage roll — only the to-hit moves.
    vm.eval(&format!(
        "DAEMON.combat._roll = function(n) \
             if n == 100 then return {threshold} else return n end end \
         return 'narrow'"
    ))
    .unwrap();
    let (worst_degree, worst_power, worst_damage) = swing(&mut vm);

    assert_eq!(
        worst_degree, "graze",
        "a margin of zero is the bottom band by construction: {worst_degree}"
    );

    assert_ne!(
        best_degree, worst_degree,
        "both margins landed in the same band, so this test proves nothing about \
         degrees: {best_degree} at power {best_power}"
    );
    assert!(
        best_power > worst_power,
        "the wider margin should be worth more: {best_degree} at {best_power} \
         against {worst_degree} at {worst_power}"
    );
    assert!(
        best_damage > worst_damage,
        "the power difference did not reach the damage: {best_damage} against \
         {worst_damage}"
    );
}

/// Every creature this game ships is made of something.
///
/// A layout is optional by absence and nil is a legal answer — but a *game* with
/// no layouts anywhere means `hit_slot` is nil for every blow, so the per-slot
/// armour guard never runs and a helm protects a shin as well as a head.
#[test]
fn every_shipped_creature_has_a_body_layout() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let missing = vm
        .eval(
            "local Body = require('lib.body') local bad = {} \
             for _, t in ipairs(DAEMON.mobs.all()) do \
                 if not Body.of(t) then bad[#bad + 1] = t.id end \
             end \
             return #bad == 0 and 'all' or table.concat(bad, ',')",
        )
        .unwrap();

    assert_eq!(
        missing, "all",
        "these creatures have no layout, so nothing that hits them lands \
         anywhere: {missing}"
    );
}

/// A blow lands somewhere, and the place it lands is armourable.
#[test]
fn a_blow_lands_on_a_named_part() {
    let mut vm = arena(1);

    let out = vm
        .eval(
            "local r = DAEMON.combat.attack_once(p, target, {}) \
             return tostring(r.hit) .. '|' .. tostring(r.hit_part) .. '|' \
                 .. tostring(r.hit_slot)",
        )
        .unwrap();

    let f: Vec<&str> = out.split('|').collect();
    assert_eq!(f[0], "true", "the pinned die should always hit: {out}");
    assert_ne!(
        f[1], "nil",
        "the blow landed nowhere — `Body.locate` returned nil, which means the \
         guard has no layout: {out}"
    );

    // Forcing the location proves the slot is the one the layout declares, not
    // whatever the last roll happened to pick.
    let head = vm
        .eval(
            "local r = DAEMON.combat.attack_once(p, target, { location = 'head' }) \
             return tostring(r.hit_part) .. '|' .. tostring(r.hit_slot)",
        )
        .unwrap();
    assert_eq!(
        head, "head|head",
        "a hit forced to the head should report the head slot, which is what \
         makes a helm protect a head and not a shin: {head}"
    );
}

/// **Roundtime responds to the fighter**, rather than being a flat three seconds
/// and a warning in the journal.
///
/// `queue_d` falls back to a configured constant when no `round_length` trait
/// exists, and says so once per track — which it was doing for the life of the
/// queue. Exactly 3.0 at dexterity 10, because that is the number it replaces.
#[test]
fn round_length_is_a_trait_and_answers_to_dexterity() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval(
        "local Player = require('lib.player') \
         p = Player:from_save(1, { name = 'Probe', account_id = 1 }, {}) return 'ok'",
    )
    .unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.trait.has(p, 'round_length'))").unwrap(),
        "true",
        "without the trait, queue_d uses a flat constant and warns about it"
    );

    let base: f64 = vm
        .eval("return string.format('%.4f', DAEMON.queue.round_length(p, 'combat'))")
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        (base - 3.0).abs() < 0.001,
        "an ordinary character's round was 3s before this trait existed and has \
         to still be: {base}"
    );

    // Quicker hands, shorter round — through the trait graph, so an effect or a
    // piece of equipment reaches it the same way with no code here knowing.
    let quick: f64 = vm
        .eval(
            "DAEMON.trait.set_base(p, 'dexterity', 20) \
             return string.format('%.4f', DAEMON.queue.round_length(p, 'combat'))",
        )
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        quick < base,
        "roundtime did not answer to dexterity: {quick} against {base}"
    );
}
