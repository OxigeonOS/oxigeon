//! G3/G4 — wearing things, and what that does to your numbers.
//!
//! `Mobile.equipment` was a `slot -> item` map that nothing ever wrote, and the
//! `armour` component's `defense`, `resist` and `stat_bonus` fields had no
//! reader anywhere. `combat_d.round()` ran the `damage_taken` pipeline
//! faithfully and no armour handler was ever registered in it, so **armour
//! never mitigated anything**.
//!
//! Equipping goes through the documented `equip:<slot>` source pattern rather
//! than through a second mechanism, so the assertions here are also assertions
//! about `set_source_effects` being idempotent and about `persist = false`
//! meaning what it says.

mod common;

use common::RealVm;

/// The base case: put something on, and it is on.
#[test]
fn a_player_can_wear_and_remove_armour() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("spawn leather_jerkin");
    let out = vm.command("wear jerkin");
    assert!(out.contains("You wear"), "wear said nothing:\n{out}");
    assert!(out.contains("chest"), "it should name the slot:\n{out}");

    let eq = vm.command("equipment");
    assert!(eq.contains("chest") && eq.contains("jerkin"), "not shown as worn:\n{eq}");
    assert!(eq.contains("defense"), "armour should say what it is worth:\n{eq}");

    // Worn means not loose in the pack.
    assert!(
        !vm.command("inventory").contains("jerkin"),
        "a worn item should not still be listed as carried"
    );

    let out = vm.command("remove chest");
    assert!(out.contains("You stop using"), "remove said nothing:\n{out}");
    assert!(vm.command("inventory").contains("jerkin"), "it should come back to the pack");
}

/// The refusal path — one rule in `lib/requires.lua`, shared by weapons and
/// armour, which is why there is one message rather than two.
#[test]
fn a_requirement_refuses_with_a_reason() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("spawn iron_greatsword");
    let out = vm.command("wield greatsword");
    assert!(
        out.contains("16 strength"),
        "the refusal should name what is missing:\n{out}"
    );
    assert!(
        !vm.command("equipment").contains("greatsword"),
        "a refused item must not end up equipped anyway"
    );

    // Meet it and the same command works. `affect learn` writes a trait base,
    // which is exactly what a strength buff would end up changing.
    vm.command("affect learn strength 18");
    let out = vm.command("wield greatsword");
    assert!(out.contains("You wield"), "still refused at 18 strength:\n{out}");
}

/// A two-handed weapon takes both hands, in either order.
#[test]
fn a_two_handed_weapon_clears_the_offhand() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("affect learn strength 18");

    vm.command("spawn oak_buckler");
    vm.command("spawn iron_greatsword");

    vm.command("wear buckler");
    assert!(vm.command("equipment").contains("buckler"));

    let out = vm.command("wield greatsword");
    assert!(out.contains("You wield"), "{out}");
    assert!(
        out.contains("stop using") && out.contains("buckler"),
        "displacing the shield should be said out loud, not done silently:\n{out}"
    );

    let eq = vm.command("equipment");
    assert!(eq.contains("greatsword"));
    assert!(!eq.contains("buckler"), "the offhand should be empty:\n{eq}");

    // And the other way round: putting a shield back takes the sword out.
    let out = vm.command("wear buckler");
    assert!(out.contains("You wear"), "{out}");
    let eq = vm.command("equipment");
    assert!(eq.contains("buckler"));
    assert!(
        !eq.contains("greatsword"),
        "both hands were on the greatsword; a shield cannot join it:\n{eq}"
    );
}

/// `stat_bonus` becomes a real `equip:` effect, visible in `score`, and gone
/// when the item comes off. This is the field that had no reader at all.
#[test]
fn a_stat_bonus_is_an_equip_effect_that_score_shows() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let before = vm.command("affect traits");
    let intelligence_line = |out: &str| -> String {
        out.lines()
            .find(|l| l.contains("intelligence"))
            .unwrap_or_else(|| panic!("no intelligence row in:\n{out}"))
            .to_string()
    };
    assert!(
        intelligence_line(&before).contains("10"),
        "expected a base of 10: {}",
        intelligence_line(&before)
    );

    vm.command("spawn scholar_circlet");
    vm.command("wear circlet");

    let after = intelligence_line(&vm.command("affect traits"));
    assert!(
        after.contains("10") && after.contains("12"),
        "expected base 10 and effective 12 while the circlet is worn, got: {after}"
    );

    // The effect names its source, so `effects` explains where the number came
    // from rather than leaving a mystery +2.
    let effects = vm.command("affect list");
    assert!(
        effects.contains("equip:head"),
        "the aura should be sourced to the slot:\n{effects}"
    );

    vm.command("remove head");
    let after = intelligence_line(&vm.command("affect traits"));
    assert!(
        !after.contains("12"),
        "the bonus outlived the item that granted it: {after}"
    );
}

/// **G4** — armour finally mitigates, in the `reduce` phase, and a resist table
/// is applied by damage type.
#[test]
fn armour_reduces_damage_and_resist_is_by_type() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // Unarmoured baseline, through the real pipeline.
    let plain = vm.command("affect damage 30");
    assert!(plain.contains("30 requested, 30 dealt"), "unmitigated:\n{plain}");

    vm.command("affect heal 500");
    vm.command("spawn leather_jerkin");
    vm.command("wear jerkin");

    // defense 3, in the reduce phase.
    let armoured = vm.command("affect damage 30");
    assert!(
        armoured.contains("30 requested, 27 dealt"),
        "leather (defense 3) should take 3 off a 30-point hit:\n{armoured}"
    );

    // A resist table meets a damage type. The warded cloak blunts magic by 6
    // and does nothing at all to a sword — which is the whole reason
    // `damage_type` exists.
    vm.command("affect heal 500");
    vm.command("spawn warded_cloak");
    vm.command("wear cloak");

    let physical = vm.command("affect damage 30 physical");
    assert!(
        physical.contains("30 requested, 26 dealt"),
        "jerkin 3 + cloak 1 = 4 against physical:\n{physical}"
    );

    vm.command("affect heal 500");
    let magical = vm.command("affect damage 30 magic");
    assert!(
        magical.contains("30 requested, 20 dealt"),
        "jerkin 3 + cloak 1 + cloak's magic resist 6 = 10 against magic:\n{magical}"
    );
}

/// Two pieces of armour are two independent `equip:` sources, so taking one off
/// leaves the other's mitigation alone.
#[test]
fn each_slot_is_its_own_source() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("spawn leather_jerkin");
    vm.command("spawn warded_cloak");
    vm.command("wear jerkin");
    vm.command("wear cloak");

    vm.command("affect heal 500");
    assert!(vm.command("affect damage 30 physical").contains("26 dealt"));

    vm.command("remove back");
    vm.command("affect heal 500");
    assert!(
        vm.command("affect damage 30 physical").contains("27 dealt"),
        "removing the cloak should take away exactly the cloak's contribution"
    );
}

/// `equip:` effects are `persist = false` and are rebuilt from what is worn on
/// login. Persisting the aura as well would be a second copy of the truth that
/// can disagree with the first.
#[test]
fn the_equip_aura_is_derived_and_never_written() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_E = require('lib.equipment')").unwrap();
    vm.eval(
        "_p = { char_id = 71, name = 'Wearer', inventory = {}, equipment = {}, \
                stats = { strength = 10, dexterity = 10, constitution = 10, \
                          intelligence = 10, wisdom = 10, level = 1, hp = 100, mp = 50 }, \
                send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.mobile') }) return 'ok'").unwrap();
    vm.eval("DAEMON.trait.attach(_p)").unwrap();

    vm.eval("_circlet = DAEMON.items.spawn('scholar_circlet', nil)").unwrap();
    vm.eval("_E.equip(_p, _circlet, DAEMON.items.resolve(_circlet))").unwrap();
    assert_eq!(
        vm.eval("return _p:trait('intelligence')").unwrap(),
        "12",
        "the aura did not apply"
    );

    // Idempotent: refreshing twice must not double it. This is the property
    // that lets login call it without working out what it did last time.
    vm.eval("_E.refresh_all(_p) _E.refresh_all(_p) return 'refreshed'").unwrap();
    assert_eq!(
        vm.eval("return _p:trait('intelligence')").unwrap(),
        "12",
        "refreshing stacked the aura on top of itself"
    );

    // Nothing about it is written to the persisted namespace.
    assert_eq!(
        vm.eval(
            "local n = 0 for _, e in ipairs(DAEMON.effect.active(_p)) do \
             if e.ns == 'effects' then n = n + 1 end end return n"
        )
        .unwrap(),
        "0",
        "an equip aura reached the persisted effects namespace"
    );

    // And swapping to a *different* item in the same slot replaces the amount
    // rather than keeping the old one, which is what `set_source_effects`
    // matching by definition id would do if it were called only once.
    vm.eval(
        "DAEMON.items.register(require('lib.armor'){ id = 'test_crown', \
           short = 'a crown', slot = 'head', stat_bonus = { intelligence = 5 } })",
    )
    .unwrap();
    vm.eval("_crown = DAEMON.items.spawn('test_crown', nil)").unwrap();
    vm.eval("_E.equip(_p, _crown, DAEMON.items.resolve(_crown))").unwrap();
    assert_eq!(
        vm.eval("return _p:trait('intelligence')").unwrap(),
        "15",
        "swapping head gear kept the old item's bonus"
    );

    vm.eval("_E.unequip(_p, 'head') return 'off'").unwrap();
    assert_eq!(
        vm.eval("return _p:trait('intelligence')").unwrap(),
        "10",
        "the bonus outlived the item"
    );
}

/// A `stat_bonus` aimed at a gauge is refused with a message naming the item's
/// field, rather than producing a warning about an effect definition nobody
/// wrote. Effects modify attributes and derived traits, never gauges.
#[test]
fn a_stat_bonus_cannot_target_a_gauge() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_E = require('lib.equipment')").unwrap();
    vm.eval(
        "DAEMON.items.register(require('lib.armor'){ id = 'bad_amulet', \
           short = 'an amulet', slot = 'neck', stat_bonus = { hp = 50 } })",
    )
    .unwrap();
    vm.eval(
        "_p = { char_id = 72, inventory = {}, equipment = {}, \
                stats = { constitution = 10, level = 1, hp = 100 }, \
                send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.mobile') }) return 'ok'").unwrap();

    vm.eval("_a = DAEMON.items.spawn('bad_amulet', nil)").unwrap();
    // Equipping still succeeds — the item is wearable, one of its fields is
    // just meaningless. Refusing the whole item would make one bad number
    // unwearable rather than inert.
    assert_eq!(
        vm.eval("return tostring(_E.equip(_p, _a, DAEMON.items.resolve(_a)))").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.effect.get_def('equip_trait_hp'))").unwrap(),
        "nil",
        "an effect targeting a gauge should never have been defined"
    );
}
