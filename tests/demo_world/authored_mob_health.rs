//! Authored creature health means what it says.
//!
//! It did not. `max_hp` is derived — `50 + constitution * 5 + (level - 1) * 10`
//! — and a derived trait stores nothing, so the `max_hp` every mob template in
//! the game carefully authored was not merely ignored: `trait_d.attach` deletes
//! any value found under a derived trait, as a migration for characters saved
//! back when `max_hp` was stored. The scrawny rat described as having 24 hit
//! points had 90, and every one of the fifteen templates was between 1.1× and
//! 5.8× its authored toughness.
//!
//! Worse, the numbers were not reachable by tuning. The formula starts at 50,
//! so the weakest creature it can describe has 55 hit points: a 24-point rat, a
//! 20-point tavern drunk and a 25-point apprentice were not expressible at all.
//!
//! `max_hp_flat` is the authored value, and it wins outright when set.


use crate::common::RealVm;

/// What the curve produces, for comparing against.
fn curve(constitution: i64, level: i64) -> i64 {
    50 + constitution * 5 + (level - 1) * 10
}

#[test]
fn the_scrawny_rat_is_scrawny() {
    // Named rather than taken from the room. The pantry has a spawner now and a
    // spawner picks a kind at random, so `in_room(...)[1]` is whichever rat the
    // nest happened to make — and the three have different health, which is the
    // whole subject of this test.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval("_rat = DAEMON.mobs.spawn('scrawny_rat', 'wizard_workshop.pantry') return 'ok'")
        .unwrap();

    assert_eq!(vm.eval("return tostring(_rat:trait('max_hp'))").unwrap(), "14");
    assert_eq!(
        vm.eval("return tostring(_rat:trait('hp'))").unwrap(),
        "14",
        "and it spawns at full health rather than at 14 of a possible 90"
    );

    // The curve would have said 90 — that is the number that was showing. Note
    // the constitution is 8: `vermin.rat.scrawny` names four stats and `stats`
    // is a schema `map`, so the patch **merges key by key** and constitution
    // comes through from `vermin.rat` untouched. An array field would have
    // replaced the lot and this would read 50.
    assert_eq!(curve(8, 1), 90);
    assert_eq!(vm.eval("return tostring(_rat:trait('constitution'))").unwrap(), "8");
}

#[test]
fn every_shipped_template_gets_the_health_it_authored() {
    // One rat proves the mechanism; this proves the content. Reading the
    // registry rather than a hardcoded list, so a new mob is covered the day it
    // is written.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    // `mobs.all()` is a sorted array of template ids; `get` fetches the table.
    vm.eval(
        "_bad, _n = {}, 0 \
         for _, id in ipairs(DAEMON.mobs.all()) do \
           local t = DAEMON.mobs.get(id) \
           local want = t and t.stats and t.stats.max_hp_flat \
           if want then \
             _n = _n + 1 \
             local m = DAEMON.mobs.spawn(id, 'wizard_workshop.pantry') \
             if not m then _bad[#_bad+1] = id .. ':nospawn' \
             elseif m:trait('max_hp') ~= want then \
               _bad[#_bad+1] = id .. ':' .. tostring(m:trait('max_hp')) .. '~=' .. tostring(want) \
             end \
             if m then DAEMON.mobs.despawn(m) end \
           end \
         end return 'done'",
    )
    .unwrap();

    assert_eq!(
        vm.eval("return table.concat(_bad, ' ')").unwrap(),
        "",
        "every template's authored max_hp_flat should be its actual max_hp"
    );

    let checked: i64 = vm.eval("return _n").unwrap().parse().unwrap();
    assert!(
        checked >= 15,
        "only {checked} templates author health; this test proves little if it is 0"
    );
}

#[test]
fn a_creature_can_be_weaker_than_the_formula_floor() {
    // The point of the whole change. Nothing on the curve can be below 55.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    assert_eq!(curve(1, 1), 55);

    vm.eval("DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'ah.room', exits = {} }) })")
        .unwrap();
    vm.eval(
        "DAEMON.mobs.register({ id = 'ah_moth', short = 'a moth', \
           stats = { hp = 3, max_hp_flat = 3, constitution = 10, level = 1 } }) return 'ok'",
    )
    .unwrap();
    vm.eval("_m = DAEMON.mobs.spawn('ah_moth', 'ah.room') return 'ok'").unwrap();

    assert_eq!(vm.eval("return tostring(_m:trait('max_hp'))").unwrap(), "3");
    assert_eq!(vm.eval("return tostring(_m:trait('hp'))").unwrap(), "3");
}

#[test]
fn a_character_still_gets_the_curve() {
    // Players are untouched: no authored value, so the formula applies.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval(
        "local Player = require('lib.player') \
         _p = Player:from_save(1, { name = 'Probe', account_id = 1 }, {}) return 'ok'",
    )
    .unwrap();

    let con: i64 = vm.eval("return tostring(_p:trait('constitution'))").unwrap().parse().unwrap();
    let level: i64 = vm.eval("return tostring(_p:trait('level'))").unwrap().parse().unwrap();
    assert_eq!(
        vm.eval("return tostring(_p:trait('max_hp'))").unwrap(),
        curve(con, level).to_string()
    );
    assert_eq!(
        vm.eval("return tostring(_p:trait('max_hp_flat'))").unwrap(),
        "0",
        "zero means `use the curve`"
    );
}

#[test]
fn a_character_saved_before_this_trait_existed_keeps_its_health() {
    // The migration worry, stated as a test: `max_hp` gained a dependency, and
    // a derived trait with an absent dependency is absent — which would have
    // taken the `hp` gauge with it. Seeding on load is what prevents that.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval(
        "local Player = require('lib.player') \
         _old = Player:from_save(2, { name = 'Old', account_id = 1 }, \
           { stats = { constitution = 12, level = 3, hp = 40, strength = 10, \
                       dexterity = 10, intelligence = 10, wisdom = 10 } }) return 'ok'",
    )
    .unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.trait.has(_old, 'max_hp'))").unwrap(),
        "true",
        "max_hp must survive gaining a dependency"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.trait.has(_old, 'hp'))").unwrap(),
        "true",
        "and hp with it — a gauge whose ceiling vanished would be absent"
    );
    assert_eq!(
        vm.eval("return tostring(_old:trait('max_hp'))").unwrap(),
        curve(12, 3).to_string()
    );
    assert_eq!(
        vm.eval("return tostring(_old:trait('hp'))").unwrap(),
        "40",
        "and the saved current health is untouched"
    );
}

#[test]
fn the_authored_value_is_an_ordinary_trait_effects_can_move() {
    // It is an attribute, not a constant, so "+20% health on a boss" stays an
    // ordinary effect on an ordinary trait rather than a special case.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval("DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'ah2.room', exits = {} }) })")
        .unwrap();
    vm.eval(
        "DAEMON.mobs.register({ id = 'ah_boss', short = 'a boss', \
           stats = { hp = 100, max_hp_flat = 100, constitution = 10, level = 1 } }) return 'ok'",
    )
    .unwrap();
    vm.eval("_b = DAEMON.mobs.spawn('ah_boss', 'ah2.room') return 'ok'").unwrap();
    assert_eq!(vm.eval("return tostring(_b:trait('max_hp'))").unwrap(), "100");

    // `modifiers` desugars into an ordinary `trait:max_hp_flat` add handler.
    vm.eval(
        "DAEMON.effect.define({ id = 'ah_swole', label = 'Swole', \
           modifiers = { max_hp_flat = 50 } }) return 'ok'",
    )
    .unwrap();
    vm.eval("DAEMON.effect.apply(_b, 'ah_swole') return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(_b:trait('max_hp'))").unwrap(),
        "150",
        "the effect moves the authored value, and max_hp follows"
    );
}
