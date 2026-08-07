//! The defence channels, now that this game defines the traits that turn them on.
//!
//! `combat_d` decides what a fighter can do by **which traits they store**, and
//! until `game/traits/core.lua` declared `defense_dodge`, `defense_parry` and
//! `defense_block`, nobody in the game stored any of them. Every fight took the
//! no-configuration path — one implicit dodge worth the whole pool — so parry
//! and block could not occur to anyone, and `channel` came back `"dodge"` for
//! every swing ever thrown.
//!
//! Asserted through `attack_once`, which is what the game calls, rather than
//! through the `defences` local it uses. Exporting that local so a test could
//! reach it would be putting a hole in the shipped game for the convenience of
//! the suite, and `attack_once` reports everything needed anyway: the channel by
//! name, and a threshold that the contest's own identity inverts back into the
//! defence value.
//!
//!     threshold = base + (accuracy - defence) * step
//!
//! Three things could each be wrong on their own, so there is a test for each:
//! that the channels are reachable at all, that the arithmetic is *unchanged*
//! for two ordinary characters, and that picking up a shield does not make you
//! easier to hit — which is the failure mode the design invites, because the
//! best channel is a share of a pool and more channels means smaller shares.

use crate::common::RealVm;

/// A player, and a creature to swing at them.
fn defender() -> RealVm {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval(
        "local Player = require('lib.player') \
         p = Player:from_save(1, { name = 'Probe', account_id = 1 }, {}) \
         attacker = DAEMON.mobs.in_room('wizard_workshop.pantry')[1] \
         return tostring(attacker ~= nil)",
    )
    .unwrap();
    vm
}

/// What the last swing says about the defender: the channel it beat, and the
/// defence value recovered from the threshold.
fn swing(vm: &mut RealVm) -> (String, f64) {
    let out = vm
        .eval(
            "local r = DAEMON.combat.attack_once(attacker, p, {}) \
             local base = config('game.combat_base_hit_chance') or 60 \
             local step = config('game.combat_hit_step') or 3 \
             local acc = DAEMON.trait.value(attacker, 'accuracy') \
             local defence = acc - (r.threshold - base) / step \
             return tostring(r.channel) .. '|' .. string.format('%.2f', defence)",
        )
        .unwrap();
    let (channel, defence) = out.split_once('|').expect("malformed probe reply");
    (channel.to_string(), defence.parse().expect("defence"))
}

fn equip(vm: &mut RealVm, template: &str) {
    vm.eval(&format!(
        "local E = require('lib.equipment') \
         local id = DAEMON.items.spawn('{template}', nil) \
         E.equip(p, id, DAEMON.items.resolve(id)) return 'ok'"
    ))
    .unwrap();
}

/// The traits exist and are *present*, which is the question `combat_d` asks.
///
/// Presence is decided by storage. These are derived, so they are present
/// exactly when everything they read is — which for a character is always.
#[test]
fn a_character_holds_every_defence_trait() {
    let mut vm = defender();

    for trait_id in ["defense", "defense_dodge", "defense_parry", "defense_block"] {
        assert_eq!(
            vm.eval(&format!("return tostring(DAEMON.trait.has(p, '{trait_id}'))"))
                .unwrap(),
            "true",
            "'{trait_id}' is absent, so combat_d takes the no-configuration path \
             and this channel can never occur"
        );
    }
}

/// Bare-handed, dodge is the only usable channel and is worth the whole pool.
///
/// The `available` predicates decide this, not the weights: parry needs
/// something in hand and block needs a shield, so both drop out and `shares`
/// normalises what is left to 1.0.
#[test]
fn an_unarmed_defender_dodges_with_the_whole_pool() {
    let mut vm = defender();

    let pool: f64 = vm
        .eval("return string.format('%.2f', p:trait('defense'))")
        .unwrap()
        .parse()
        .unwrap();

    let (channel, defence) = swing(&mut vm);
    assert_eq!(channel, "dodge", "nothing else should be usable bare-handed");
    assert!(
        (defence - pool).abs() < 0.01,
        "dodge is the only live channel, so it should be worth the entire pool: \
         {defence} against a pool of {pool}"
    );
}

/// **Armed, the numbers are what they were before channels existed.**
///
/// `rating()` fell back to `dexterity` when a game had defined no combat traits,
/// so an ordinary level-1 character defended at 10. The weights are chosen so
/// that is still true once dodge and parry are both live — otherwise turning
/// this on would have quietly rebalanced every fight in the game, and the
/// existing combat expectations would have been re-tuned to hide it.
#[test]
fn an_armed_defender_is_worth_what_they_were_before() {
    let mut vm = defender();
    equip(&mut vm, "apprentice_dagger");

    let (channel, defence) = swing(&mut vm);
    assert_eq!(channel, "parry", "parry carries the heavier weight");
    assert!(
        (defence - 10.0).abs() < 0.01,
        "a level-1 defender used to be worth `dexterity`, which is 10 — the \
         number every existing combat expectation was tuned against. Got {defence}"
    );
}

/// **A shield must not make you easier to hit**, which is the trap.
///
/// The best channel is a *share* of a pool, so a third live channel divides that
/// pool three ways. Without the buckler's `stat_bonus` raising both the pool and
/// the block weight, equipping it would drop the best channel from 10 to about
/// 6 — a shield that helps the person swinging at you.
#[test]
fn a_shield_raises_the_best_channel_rather_than_diluting_it() {
    let mut vm = defender();
    equip(&mut vm, "apprentice_dagger");
    let (_, armed) = swing(&mut vm);

    equip(&mut vm, "oak_buckler");
    let (channel, shielded) = swing(&mut vm);

    assert_eq!(channel, "block", "the shield should make block the best channel");
    assert!(
        shielded > armed,
        "equipping a shield made this defender easier to hit — {shielded} against \
         {armed} unshielded. The pool divides across live channels, so a shield \
         has to bring its own weight and pool with it."
    );
}
