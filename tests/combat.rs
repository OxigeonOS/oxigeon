//! Combat, which exists so the trait and effect systems have something real to
//! act on.
//!
//! Two halves. The first drives the game the way a player does — walk into a
//! room, look, attack — through the real dispatcher. The second replaces the
//! dice with a stub and asserts the arithmetic, because a test that depends on
//! `math.random` is a test that fails one morning for no reason.

mod common;

use common::{RealVm, TestCtx};

/// Always hit, always for the top of the range. `_roll(100)` is the to-hit
/// check, where low is good; everything else is damage, where high is.
const LOADED_DICE: &str = "DAEMON.combat._roll = function(n) \
                             if n == 100 then return 1 else return n end end";

/// A probe VM with the real game world, a player, and a rat to hit.
fn arena() -> RealVm {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval(&format!(
        "local Player = require('lib.player') \
         p = Player:from_save(1, {{ name = 'Probe', account_id = 1 }}, {{}}) \
         rat = DAEMON.mobs.in_room('wizard_workshop.pantry')[1] \
         {LOADED_DICE} \
         return rat and rat.name or 'NO RAT'"
    ))
    .unwrap();
    vm
}

#[test]
fn the_game_layer_spawned_its_mobs() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    assert_eq!(
        vm.eval("return tostring(#DAEMON.mobs.in_room('wizard_workshop.pantry'))").unwrap(),
        "2",
        "the area file asks for two rats"
    );
    assert_eq!(
        vm.eval("return DAEMON.mobs.find_in_room('wizard_workshop.pantry', 'rat').name").unwrap(),
        "rat",
        "targeting by keyword is what `attack rat` depends on"
    );
}

#[test]
fn a_mob_has_its_own_health_not_the_templates() {
    let mut vm = arena();
    vm.eval("local rats = DAEMON.mobs.in_room('wizard_workshop.pantry') \
             DAEMON.trait.adjust(rats[1], 'hp', -10) \
             return 'ok'")
        .unwrap();
    assert_eq!(
        vm.eval("local rats = DAEMON.mobs.in_room('wizard_workshop.pantry') \
                 return tostring(rats[1]:stat('hp') ~= rats[2]:stat('hp'))")
            .unwrap(),
        "true",
        "wounding one rat must not wound every rat that shares the template"
    );
}

#[test]
fn a_round_resolves_an_attack_in_both_directions() {
    let mut vm = arena();
    vm.eval("DAEMON.combat.engage(p, rat) \
             _rat_before = rat:stat('hp') _p_before = p:stat('hp') return 'ok'")
        .unwrap();
    vm.eval("DAEMON.combat.round() return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(rat:stat('hp') < _rat_before)").unwrap(),
        "true",
        "the attacker did no damage"
    );
    assert_eq!(
        vm.eval("return tostring(p:stat('hp') < _p_before)").unwrap(),
        "true",
        "the target did not fight back"
    );
}

/// The point of the whole exercise: an effect changes what a hit does, and it
/// does so through the same pipeline everything else uses.
#[test]
fn a_mitigation_effect_changes_what_a_hit_costs() {
    let mut vm = arena();
    // The rat rolls max damage every time, so the only variable is the effect.
    let unbuffed = vm
        .eval("DAEMON.trait.set_cur(p, 'hp', 100) \
               DAEMON.combat.attack_once(rat, p) \
               return tostring(100 - p:stat('hp'))")
        .unwrap();

    let buffed = vm
        .eval("DAEMON.trait.set_cur(p, 'hp', 100) \
               DAEMON.effect.apply(p, 'stoneskin', { duration = 600 }) \
               DAEMON.combat.attack_once(rat, p) \
               return tostring(100 - p:stat('hp'))")
        .unwrap();

    let (unbuffed, buffed): (i64, i64) = (unbuffed.parse().unwrap(), buffed.parse().unwrap());
    assert!(
        buffed < unbuffed,
        "stoneskin made no difference: {unbuffed} unbuffed, {buffed} buffed"
    );
}

#[test]
fn killing_a_mob_awards_experience_through_the_pipeline() {
    let mut vm = arena();
    vm.eval("p.xp = 0 DAEMON.trait.set_cur(p, 'hp', 1000) \
             DAEMON.combat.engage(p, rat) return 'ok'")
        .unwrap();
    // The rat has 24 health and the loaded dice give five a swing.
    vm.eval("for _ = 1, 20 do DAEMON.combat.round() end return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(rat:is_alive())").unwrap(),
        "false",
        "twenty rounds should have been plenty"
    );
    assert_eq!(
        vm.eval("return tostring(p.xp)").unwrap(),
        "12",
        "the template awards 12"
    );
}

/// The same kill with an experience buff on: the award goes through
/// `award_xp`, so `xp_gained` applies and combat needs to know nothing about it.
#[test]
fn an_experience_buff_applies_to_a_kill() {
    let mut vm = arena();
    vm.eval("p.xp = 0 DAEMON.trait.set_cur(p, 'hp', 1000) \
             DAEMON.effect.apply(p, 'insight', { duration = 600 }) \
             DAEMON.combat.engage(p, rat) return 'ok'")
        .unwrap();
    vm.eval("for _ = 1, 20 do DAEMON.combat.round() end return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(p.xp)").unwrap(),
        "14",
        "12 experience with +20% is 14 after flooring"
    );
}

#[test]
fn a_dead_mob_leaves_the_world_and_the_fight_ends() {
    let mut vm = arena();
    vm.eval("DAEMON.trait.set_cur(p, 'hp', 1000) \
             DAEMON.combat.engage(p, rat) \
             for _ = 1, 20 do DAEMON.combat.round() end return 'ok'")
        .unwrap();

    assert_eq!(
        vm.eval("return tostring(#DAEMON.mobs.in_room('wizard_workshop.pantry'))").unwrap(),
        "1",
        "the corpse should not still be standing there"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.combat.is_fighting(p))").unwrap(),
        "false"
    );
}

/// Combat state is the textbook case for the memory tier: if the server
/// restarts, the fight is over.
#[test]
fn combat_state_never_reaches_the_database() {
    let mut vm = arena();
    vm.eval("DAEMON.combat.engage(p, rat) DAEMON.combat.round() return 'ok'").unwrap();
    vm.eval("DAEMON.cache.flush_all({ reason = 'test' }) return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.cache.spec('combat').tier)").unwrap(),
        "memory"
    );
    assert_eq!(
        vm.eval("return tostring(db_exists('combat', 'char:1'))").unwrap(),
        "false",
        "a fight in progress must never be written anywhere"
    );
    assert_eq!(
        vm.eval("local n = 0 for _, c in ipairs(db_collections()) do \
                 if c.name == 'combat' then n = n + 1 end end return tostring(n)")
            .unwrap(),
        "0",
        "the memory tier must not even create the collection"
    );
}

#[test]
fn a_fight_ends_when_one_side_walks_away() {
    let mut vm = arena();
    vm.eval("DAEMON.combat.engage(p, rat) return 'ok'").unwrap();
    assert_eq!(vm.eval("return tostring(DAEMON.combat.is_fighting(rat))").unwrap(), "true");

    vm.eval("DAEMON.combat.disengage_all(1) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.combat.is_fighting(rat))").unwrap(),
        "false",
        "leaving a mob engaged would have it swinging at someone who has gone"
    );
}

// ─── Through the real command dispatcher ─────────────────────────────────────

/// What a player actually types. No stubbed dice here — the assertions are
/// about the plumbing, not the numbers.
#[test]
fn a_player_can_walk_in_look_and_attack() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("goto wizard_workshop.pantry");

    let look = vm.command("look");
    assert!(
        look.contains("rat"),
        "the room description does not mention the rats standing in it:\n{look}"
    );

    let attack = vm.command("attack rat");
    assert!(
        attack.contains("You attack"),
        "the attack command did not engage:\n{attack}"
    );
    assert!(
        attack.contains("hit") || attack.contains("miss"),
        "a round should have resolved immediately rather than making the player \
         wait for the tick they just asked for:\n{attack}"
    );

    let flee = vm.command("flee");
    assert!(flee.contains("break off"), "flee did not work:\n{flee}");
    assert!(
        vm.command("flee").contains("not fighting"),
        "fleeing twice should say there is nothing to flee from"
    );
}

#[test]
fn attacking_something_that_is_not_there_says_so() {
    let mut vm = RealVm::boot_real_mudlib(0);
    let out = vm.command("attack dragon");
    assert!(out.contains("do not see"), "expected a refusal:\n{out}");
}

/// The round ticker is registered under the id the engine dispatches.
#[test]
fn the_combat_ticker_is_registered() {
    let mut vm = RealVm::boot_real_mudlib_with_probe_opts(TestCtx {
        combat_round_seconds: Some(3),
        ..Default::default()
    });
    assert_eq!(
        vm.eval("return tostring(DAEMON.ticker.is_active('combat.round'))").unwrap(),
        "true"
    );

    vm.eval(&format!(
        "local Player = require('lib.player') \
         p = Player:from_save(1, {{ name = 'Probe', account_id = 1 }}, {{}}) \
         rat = DAEMON.mobs.in_room('wizard_workshop.pantry')[1] \
         {LOADED_DICE} \
         DAEMON.combat.engage(p, rat) \
         _before = rat:stat('hp') return 'ok'"
    ))
    .unwrap();

    vm.engine().send(oxigeon::core::scripting::LuaCommand::TimerFired {
        id: "combat.round".to_string(),
    });

    assert_eq!(
        vm.eval("return tostring(rat:stat('hp') < _before)").unwrap(),
        "true",
        "the round did not resolve through the engine's timer dispatch"
    );
}
