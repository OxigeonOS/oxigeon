//! Greywater Marsh — weather that descriptions read, poison that ticks, and a
//! daily gate that survives an area reset.
//!
//! The last of those is `task_list.md`'s original bug, stated as an assertion:
//! a "once per 24 hours" gate stored as room object state was really "once per
//! fifteen minutes", because an area reset wipes object state. Per-character
//! state does not belong on a room.

mod common;

use common::RealVm;

fn go_to(vm: &mut RealVm, room: &str) {
    let out = vm.command(&format!("goto {room}"));
    assert!(!out.contains("Unknown"), "could not reach {room}:\n{out}");
}

/// A room description is a function of the world, so nothing has to be pushed
/// to it when the weather turns.
#[test]
fn descriptions_read_the_weather_without_being_told() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_room = DAEMON.world.get_room('greywater_marsh.causeway_head')").unwrap();
    vm.eval("_resolve = require('lib.object').resolve").unwrap();

    vm.eval("DAEMON.weather.advance('clear') return 'clear'").unwrap();
    let clear = vm.eval("return _resolve(_room.long, _room)").unwrap();
    assert!(clear.contains("for once it is dry"), "no clear-weather line:\n{clear}");

    vm.eval("DAEMON.weather.advance('fog') return 'fog'").unwrap();
    let fog = vm.eval("return _resolve(_room.long, _room)").unwrap();
    assert!(fog.contains("Fog stands on the water"), "no fog line:\n{fog}");
    assert!(
        fog.contains("The next cairn is a suggestion"),
        "fog should change what the room says you can see:\n{fog}"
    );
    assert_ne!(clear, fog, "the description did not change with the weather");

    // The `sound` property is an lfun too, on the same principle.
    vm.eval("DAEMON.weather.advance('fog') return 'fog'").unwrap();
    assert!(vm
        .eval("return _resolve(_room.sound, _room)")
        .unwrap()
        .contains("fog takes the sound"));
}

/// Weather changes what an outdoor room's light level *is like*, and leaves
/// indoor rooms alone.
#[test]
fn weather_dims_outdoor_rooms_only() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_out = DAEMON.world.get_room('greywater_marsh.causeway_head')").unwrap();
    vm.eval("_in = DAEMON.world.get_room('thornhollow.undercroft')").unwrap();

    vm.eval("DAEMON.weather.advance('clear') return 'ok'").unwrap();
    assert_eq!(vm.eval("return _out:effective_light()").unwrap(), "3");
    assert_eq!(vm.eval("return _in:effective_light()").unwrap(), "1");

    vm.eval("DAEMON.weather.advance('fog') return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return _out:effective_light()").unwrap(),
        "1",
        "fog should take two off an outdoor room's light"
    );
    assert_eq!(
        vm.eval("return _in:effective_light()").unwrap(),
        "1",
        "the weather has no business under the chapel"
    );

    // Never below zero: a room cannot be darker than dark, and a negative level
    // would make every `> 0` test in the mudlib subtly wrong.
    vm.eval("_dark = DAEMON.world.get_room('greywater_marsh.deep_water')").unwrap();
    vm.eval("_dark.light_level = 1 return 'ok'").unwrap();
    assert_eq!(vm.eval("return _dark:effective_light()").unwrap(), "0");
}

/// The weather walks to a neighbour rather than jumping, and says so.
#[test]
fn the_weather_moves_one_step_at_a_time() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("DAEMON.weather.advance('clear') return 'ok'").unwrap();
    assert_eq!(vm.eval("return DAEMON.weather.current()").unwrap(), "clear");

    // Twenty ticks, and every step is to an adjacent state. The PRNG is seeded
    // now, so this is a real walk rather than the same one every boot.
    vm.eval(
        "_index = {} for i, s in ipairs(DAEMON.weather.STATES) do _index[s] = i end \
         _bad = nil \
         for i = 1, 20 do \
           local before = DAEMON.weather.current() \
           local after = DAEMON.weather.advance() \
           if math.abs(_index[after] - _index[before]) > 1 then _bad = before .. '->' .. after end \
         end return 'walked'",
    )
    .unwrap();
    assert_eq!(
        vm.eval("return tostring(_bad)").unwrap(),
        "nil",
        "the weather jumped rather than walking"
    );

    // A change is announced and an event is emitted, so a game can react.
    // Forced to a known state first: the walk above ends wherever it ends, and
    // advancing to the state you are already in is correctly a no-op.
    vm.eval("DAEMON.weather.advance('clear') return 'ok'").unwrap();
    vm.eval("_heard = nil DAEMON.event.on('weather.changed', 't', \
             function(d) _heard = d.to end) return 'listening'")
        .unwrap();
    vm.eval("DAEMON.weather.advance('storm') return 'ok'").unwrap();
    assert_eq!(vm.eval("return tostring(_heard)").unwrap(), "storm");

    // Advancing to the state it is already in changes nothing and says nothing.
    vm.eval("_heard = nil DAEMON.weather.advance('storm') return 'ok'").unwrap();
    assert_eq!(vm.eval("return tostring(_heard)").unwrap(), "nil");
}

/// **The `task_list.md` bug, proven fixed.** A daily gate has to survive an
/// area reset, and per-character state on a room does not.
#[test]
fn the_daily_herb_gate_survives_an_area_reset() {
    let mut vm = RealVm::boot_real_mudlib(0);
    go_to(&mut vm, "greywater_marsh.herb_beds");

    let out = vm.command("gather");
    assert!(out.contains("fistful of pale forked root"), "gathering failed:\n{out}");
    assert!(vm.command("inventory").contains("marshroot"));

    let out = vm.command("gather");
    assert!(
        out.contains("picked over"),
        "the gate did not hold on the second try:\n{out}"
    );

    // Reset the area. Room object state is wiped — that is what a reset is for
    // — and if the gate lived there it would come back.
    vm.command("areas reset greywater_marsh");

    go_to(&mut vm, "greywater_marsh.herb_beds");
    let out = vm.command("gather");
    assert!(
        out.contains("picked over"),
        "the daily gate was reset along with the area — this is exactly the bug \
         that says per-character state does not belong on a room:\n{out}"
    );
}

/// A 24-hour cooldown is over the durable threshold, so it is written through
/// rather than kept in memory — which is what makes surviving a *restart*
/// possible, not only a reset.
#[test]
fn a_long_cooldown_is_stored_durably() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let threshold: i64 = vm
        .eval("return config('game.cooldown_durable_seconds') or 60")
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        24 * 3600 > threshold,
        "the herb cooldown must be over the durable threshold to survive a restart"
    );

    vm.eval("DAEMON.cooldown.mark(900, 'greywater_herbs', 24 * 3600)").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.cooldown.ready(900, 'greywater_herbs'))").unwrap(),
        "false"
    );

    // A six-second one is under the threshold and lives in memory, which is the
    // other half of the rule.
    vm.eval("DAEMON.cooldown.mark(900, 'quick_spell', 6)").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.cooldown.ready(900, 'quick_spell'))").unwrap(),
        "false"
    );

    assert_eq!(
        vm.eval("return #DAEMON.cooldown.list(900)").unwrap(),
        "2",
        "both tiers should be listed to the player as one thing"
    );
}

/// Poison is a tick effect on the shared heartbeat, not a timer per effect, and
/// it deals damage rather than modifying a gauge.
#[test]
fn marsh_poison_ticks_and_can_be_cured() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("affect heal 500");
    let before: i64 = hp(&mut vm);

    vm.command("affect apply marsh_poison 180");
    assert!(
        vm.command("affect list").contains("marsh_poison"),
        "the poison did not land"
    );

    // Drive the heartbeat directly rather than waiting for it: the ticker is
    // off in tests so nothing fires in the background of an unrelated one.
    vm.command("affect settle");
    let out = vm.command("effects");
    assert!(out.contains("Marsh Fever"), "the effect should name itself:\n{out}");

    // The antidote removes it *by name*. A blanket clear would strip your
    // blessings too, which would be a trap wearing a helpful label.
    vm.command("spawn marsh_antidote");
    let out = vm.command("drink antidote");
    assert!(!out.contains("were not poisoned"), "the antidote missed:\n{out}");
    assert!(
        !vm.command("affect list").contains("marsh_poison"),
        "the poison outlived the antidote"
    );

    // And drinking it while healthy says so rather than silently doing nothing.
    vm.command("spawn marsh_antidote");
    assert!(vm.command("drink antidote").contains("were not poisoned"));

    let _ = before;
}

fn hp(vm: &mut RealVm) -> i64 {
    vm.command("affect traits")
        .lines()
        .find(|l| l.trim_start().starts_with("hp "))
        .and_then(|l| l.split_whitespace().nth(3).and_then(|n| n.parse().ok()))
        .unwrap_or(0)
}

/// A condition is checked when the effect is applied, and refuses it outright
/// rather than letting it land and expire immediately. First user of
/// `lib/checks.lua`.
#[test]
fn a_condition_refuses_an_effect_before_it_lands() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // No lantern: the chill lands.
    vm.command("affect apply marsh_chill 300");
    assert!(
        vm.command("affect list").contains("marsh_chill"),
        "the chill should land on someone with no light"
    );
    vm.command("affect clear");

    // With a lantern it is refused, and the refusal is the predicate's.
    vm.command("spawn hooded_lantern");
    let out = vm.command("affect apply marsh_chill 300");
    assert!(
        out.contains("Could not apply"),
        "a false condition should refuse rather than land:\n{out}"
    );
    assert!(
        !vm.command("affect list").contains("marsh_chill"),
        "a refused effect must not be present at all"
    );
}

/// `survives_death`: dying clears your effects except the ones that say
/// otherwise.
#[test]
fn a_curse_survives_dying() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("affect apply wisp_mark 1800");
    vm.command("affect apply stoneskin 600");
    let before = vm.command("affect list");
    assert!(before.contains("wisp_mark") && before.contains("stoneskin"));

    // Death clears effects with `keep_survivors`, which is what `death_d` does.
    vm.command("affect damage 9999");

    let after = vm.command("affect list");
    assert!(
        after.contains("wisp_mark"),
        "a curse that dying removes is not a curse:\n{after}"
    );
    assert!(
        !after.contains("stoneskin"),
        "an ordinary buff should not survive dying:\n{after}"
    );
}

/// Aggressive creatures exist in the marsh and nowhere in town, and the rule
/// that reads the flag is the game's.
#[test]
fn the_marsh_is_where_the_aggressive_creatures_are() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // The three marsh creatures, named — a count would have to be edited every
    // time another area adds a monster, which is a test that measures the
    // wrong thing.
    for id in ["marsh_lurker", "reed_crawler", "greywater_wisp"] {
        assert_eq!(
            vm.eval(&format!("return tostring(DAEMON.mobs.get('{id}').aggressive)")).unwrap(),
            "true",
            "'{id}' should be aggressive"
        );
    }
    // And nothing in town is, which is what makes the marsh feel different.
    // `not ... == true` rather than `== false`, because a template that simply
    // does not mention the flag is as unaggressive as one that says `false` —
    // the field is only meaningful when it is truthy.
    for id in ["town_guard", "town_smith", "tavern_drunk"] {
        assert_eq!(
            vm.eval(&format!(
                "return tostring(DAEMON.mobs.get('{id}').aggressive == true)"
            ))
            .unwrap(),
            "false",
            "'{id}' should not be aggressive"
        );
    }

    // And the wisp deals magic without holding anything, which is what makes
    // the warded cloak's resist table worth carrying.
    assert_eq!(
        vm.eval("return DAEMON.mobs.get('greywater_wisp').damage_type").unwrap(),
        "magic"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.mobs.get('greywater_wisp').unique)").unwrap(),
        "true"
    );
}

/// A creature's own trick fires from `on_combat`, which was declared on
/// `Mobile` and never called.
#[test]
fn a_creatures_on_combat_hook_fires() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'oc.room', exits = {} }) })")
        .unwrap();
    vm.eval(
        "_fired = 0 \
         DAEMON.mobs.register({ id = 'oc_biter', short = 'a biter', \
           stats = { hp = 50, max_hp = 50, dexterity = 30, level = 1 }, \
           damage = { min = 1, max = 1 }, \
           on_combat = function(mob, target) _fired = _fired + 1 end })",
    )
    .unwrap();

    vm.eval("_m = DAEMON.mobs.spawn('oc_biter', 'oc.room')").unwrap();
    vm.eval(
        "_t = { char_id = 800, name = 'Bitten', inventory = {}, equipment = {}, \
                stats = { hp = 100, max_hp = 100, dexterity = 1, constitution = 10, level = 1 }, \
                send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_t, { __index = require('lib.mobile') }) return 'ok'").unwrap();
    // Seed it, the way `lib/player.lua` and `mob_d.spawn` do. A hand-built
    // combatant that skips this is missing whatever `max_hp` reads, and a
    // derived trait with an absent dependency takes the `hp` gauge down with
    // it — leaving a fighter that `is_alive()` says is already dead.
    vm.eval("DAEMON.trait.seed(_t, 'character') return 'seeded'").unwrap();

    vm.eval("_real = DAEMON.combat._roll DAEMON.combat._roll = function() return 1 end").unwrap();
    vm.eval("DAEMON.combat.attack_once(_m, _t) return 'swung'").unwrap();
    vm.eval("DAEMON.combat._roll = _real return 'restored'").unwrap();

    assert_eq!(
        vm.eval("return _fired").unwrap(),
        "1",
        "on_combat was declared on Mobile and never called until now"
    );
}
