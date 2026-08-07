//! Combat, which exists so the trait and effect systems have something real to
//! act on.
//!
//! Two halves. The first drives the game the way a player does — walk into a
//! room, look, attack — through the real dispatcher. The second replaces the
//! dice with a stub and asserts the arithmetic, because a test that depends on
//! `math.random` is a test that fails one morning for no reason.


use crate::common::{RealVm, TestCtx};

/// Always hit, always for the top of the range. `_roll(100)` is the to-hit
/// check, where low is good; everything else is damage, where high is.
const LOADED_DICE: &str = "DAEMON.combat._roll = function(n) \
                             if n == 100 then return 1 else return n end end";

/// A probe VM with the real game world, a player, and a rat to hit.
///
/// **The pantry has a spawner now, and a spawner picks a kind at random.** These
/// tests assert exact damage, exact experience and exact occupancy, so taking
/// whatever the nest happened to make would make them fail one morning for no
/// reason — the muscular rat awards 30 experience where the black one awards 12.
///
/// So the room is cleared and given one known creature. `black_rat` is the
/// drop-in: it overrides nothing but prose, so it inherits `vermin.rat`'s 24
/// health, 2–5 damage and 12 experience — the numbers the old `workshop_rat`
/// carried and the numbers every expectation below was written against.
fn arena() -> RealVm {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval(&format!(
        "local Player = require('lib.player') \
         p = Player:from_save(1, {{ name = 'Probe', account_id = 1 }}, {{}}) \
         for _, m in ipairs(DAEMON.mobs.in_room('wizard_workshop.pantry')) do \
             DAEMON.mobs.despawn(m) \
         end \
         rat = DAEMON.mobs.spawn('black_rat', 'wizard_workshop.pantry') \
         {LOADED_DICE} \
         return rat and rat.name or 'NO RAT'"
    ))
    .unwrap();
    vm
}

/// Put a second known rat in the pantry, for the tests that need two.
fn second_rat(vm: &mut RealVm) {
    vm.eval("rat2 = DAEMON.mobs.spawn('black_rat', 'wizard_workshop.pantry') return 'ok'")
        .unwrap();
}

#[test]
fn the_game_layer_spawned_its_mobs() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    assert_eq!(
        vm.eval("return tostring(#DAEMON.mobs.in_room('wizard_workshop.pantry'))").unwrap(),
        "3",
        "the pantry's nest fills to its spawn_max at load"
    );
    // All three rat kinds share `name = "rat"`, inherited from `vermin.rat`, so
    // `attack rat` works whichever one the nest made. That is not incidental:
    // a spawner whose output could not be addressed by one noun would need the
    // player to know which kind they were looking at before they could hit it.
    // A bare keyword matching three creatures resolves to nothing and hands
    // back the list instead. An ordinal picks one.
    let ambiguous = vm
        .eval(
            "local m, why = DAEMON.mobs.find_in_room('wizard_workshop.pantry', 'rat') \
             return tostring(m) .. '|' .. tostring(why ~= nil)",
        )
        .unwrap();
    assert_eq!(ambiguous, "nil|true", "three rats should be ambiguous: {ambiguous}");

    assert_eq!(
        vm.eval("return DAEMON.mobs.find_in_room('wizard_workshop.pantry', '1.rat').name")
            .unwrap(),
        "rat",
        "targeting by keyword is what `attack 1.rat` depends on"
    );
}

#[test]
fn a_mob_has_its_own_health_not_the_templates() {
    let mut vm = arena();
    second_rat(&mut vm);
    vm.eval("DAEMON.trait.adjust(rat, 'hp', -10) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return tostring(rat:trait('hp') ~= rat2:trait('hp'))").unwrap(),
        "true",
        "wounding one rat must not wound every rat that shares the template"
    );
}

#[test]
fn a_round_resolves_an_attack_in_both_directions() {
    let mut vm = arena();
    vm.eval("DAEMON.combat.engage(p, rat) \
             _rat_before = rat:trait('hp') _p_before = p:trait('hp') return 'ok'")
        .unwrap();
    vm.eval("DAEMON.combat.round() return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(rat:trait('hp') < _rat_before)").unwrap(),
        "true",
        "the attacker did no damage"
    );
    assert_eq!(
        vm.eval("return tostring(p:trait('hp') < _p_before)").unwrap(),
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
               return tostring(100 - p:trait('hp'))")
        .unwrap();

    let buffed = vm
        .eval("DAEMON.trait.set_cur(p, 'hp', 100) \
               DAEMON.effect.apply(p, 'stoneskin', { duration = 600 }) \
               DAEMON.combat.attack_once(rat, p) \
               return tostring(100 - p:trait('hp'))")
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
        "0",
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

    // Three rats in the pantry, so a bare `rat` is ambiguous and says so
    // rather than picking one — see `mudlib/lib/matching.lua`.
    let ambiguous = vm.command("attack rat");
    assert!(
        ambiguous.contains("rat matched 3"),
        "a bare keyword matching three creatures should ask which:\n{ambiguous}"
    );
    assert!(
        !ambiguous.contains("You attack"),
        "it picked one anyway, which on `attack` can start the wrong fight:\n{ambiguous}"
    );

    let attack = vm.command("attack 1.rat");
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
         _before = rat:trait('hp') return 'ok'"
    ))
    .unwrap();

    vm.engine().send(oxigeon::core::scripting::LuaCommand::TimerFired {
        id: "combat.round".to_string(),
    });

    assert_eq!(
        vm.eval("return tostring(rat:trait('hp') < _before)").unwrap(),
        "true",
        "the round did not resolve through the engine's timer dispatch"
    );
}

// ─── Who killed it ───────────────────────────────────────────────────────────
//
// `_killed_by` was set by `take_damage` on every hit, so it was really
// `_last_damaged_by`: trade blows with a rat and the rat was recorded as
// having been killed by you while it was still alive and biting back. And
// because it held the attacking *entity*, two fighters ended up pointing at
// each other — a reference cycle in the object graph, and a mob keeping a whole
// live `Player` alive past its own despawn.

#[test]
fn a_survivable_hit_does_not_record_a_killer() {
    let mut vm = arena();
    vm.eval("DAEMON.trait.set_cur(rat, 'hp', 50) \
             DAEMON.combat.attack_once(p, rat) return 'ok'")
        .unwrap();

    assert_eq!(
        vm.eval("return tostring(rat:trait('hp') > 0)").unwrap(),
        "true",
        "this test is meaningless if the blow was fatal"
    );
    assert_eq!(
        vm.eval("return tostring(rat._killed_by)").unwrap(),
        "nil",
        "a creature that is still alive has not been killed by anyone"
    );
}

#[test]
fn the_killing_blow_records_the_killer_by_identity_not_by_reference() {
    let mut vm = arena();
    vm.eval("DAEMON.trait.set_cur(rat, 'hp', 1) \
             DAEMON.combat.attack_once(p, rat) return 'ok'")
        .unwrap();

    assert_eq!(vm.eval("return tostring(rat:trait('hp') <= 0)").unwrap(), "true");
    assert_eq!(
        vm.eval("return tostring(rat._killed_by and rat._killed_by.char_id)").unwrap(),
        "1",
        "the death payload reads char_id"
    );
    // Identity, not the entity: no cycle to walk and no Player kept alive by a
    // corpse. `mob.died` only ever wanted `char_id` and `id`.
    assert_eq!(
        vm.eval("return tostring(rat._killed_by == p)").unwrap(),
        "false"
    );
    assert_eq!(
        vm.eval("return tostring(rat._killed_by.send)").unwrap(),
        "nil",
        "an identity table carries no methods and no reference back"
    );
}

#[test]
fn two_fighters_do_not_form_a_reference_cycle() {
    // The shape as seen in the debugger: you hit the rat, the rat hits you, and
    // each holds the other.
    let mut vm = arena();
    vm.eval("DAEMON.trait.set_cur(rat, 'hp', 500) DAEMON.trait.set_cur(p, 'hp', 500) \
             DAEMON.combat.attack_once(p, rat) \
             DAEMON.combat.attack_once(rat, p) return 'ok'")
        .unwrap();

    assert_eq!(
        vm.eval("return tostring(rat._killed_by == nil and p._killed_by == nil)").unwrap(),
        "true",
        "neither is dead, so neither has a killer"
    );
    assert_eq!(
        vm.eval("return tostring(rat._last_attacker.char_id)").unwrap(),
        "1",
        "the last attacker is still tracked — a poison tick carries no attacker"
    );
    assert_eq!(
        vm.eval("return tostring(type(p._last_attacker) == 'table' \
                 and p._last_attacker.id ~= nil and p._last_attacker.send == nil)").unwrap(),
        "true",
        "and it is an identity too, so there is no cycle either way"
    );
}

#[test]
fn a_kill_by_something_with_no_attacker_still_credits_the_last_one() {
    // Damage-over-time carries no attacker. The player who applied the poison
    // should still get the kill, which is why the last attacker is tracked
    // separately rather than `_killed_by` simply moving to the fatal blow.
    let mut vm = arena();
    vm.eval("DAEMON.trait.set_cur(rat, 'hp', 40) \
             DAEMON.combat.attack_once(p, rat) return 'ok'")
        .unwrap();
    assert_eq!(vm.eval("return tostring(rat._killed_by)").unwrap(), "nil");

    // Now finish it with anonymous damage, as a tick would.
    vm.eval("rat:take_damage(1000, { damage_type = 'poison' }) return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(rat._killed_by and rat._killed_by.char_id)").unwrap(),
        "1",
        "the poisoner keeps the credit"
    );
}

/// A wounded creature does not heal while the debugger has the world frozen.
///
/// Regeneration is a function of the clock — `trait_d.touch()` settles `hp`
/// against `os_time()` on every read — and stopping the VM at a breakpoint does
/// not stop the clock. A rat beaten down to 5 hit points came back to 20 across
/// a few `continue`s, and the fight looked endless.
///
/// The driver banks the frozen interval and `os_time()` subtracts it, so this
/// asserts the game-visible half of that: the anchor moving back by 45 seconds
/// of *game* time regenerates 15 points, and the same 45 seconds of frozen wall
/// time regenerates none.
#[test]
fn a_wounded_creature_regenerates_on_game_time_not_wall_time() {
    let mut vm = arena();
    vm.eval("DAEMON.trait.set_cur(rat, 'hp', 5) return 'ok'").unwrap();
    assert_eq!(vm.eval("return tostring(rat:trait('hp'))").unwrap(), "5");

    // 45 seconds of game time: 1 point per 3 seconds is 15 points.
    vm.eval("rat.stats._at.hp = rat.stats._at.hp - 45 DAEMON.trait.bump(rat) return 'ok'")
        .unwrap();
    assert_eq!(
        vm.eval("return tostring(rat:trait('hp'))").unwrap(),
        "20",
        "this is the regeneration rate the bug report measured"
    );

    // The rat is alive throughout — it never died and was replaced, which was
    // the other candidate explanation for health going back up. One in the room
    // and it is the same one: `arena` cleared the nest's output and put this
    // single creature there.
    assert_eq!(
        vm.eval("return tostring(#DAEMON.mobs.in_room('wizard_workshop.pantry'))").unwrap(),
        "1"
    );
    assert_eq!(vm.eval("return tostring(rat._killed_by)").unwrap(), "nil");
}
