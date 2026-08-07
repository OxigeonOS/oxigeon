//! `spawner_d` — a place that produces creatures.
//!
//! The thing that could not be said before this existed is the **cap across
//! kinds**. `mob_d.populate()` counts per template, so three rat templates with
//! `count = 2` is six rats and there is no way to write "six rats of any kind is
//! too many for one pantry". That, and a top-up on a clock rather than one
//! scheduled per death.
//!
//! Against the fixture world throughout: a spawner is a mudlib mechanism, and
//! none of what is asserted here is a claim about Thornhollow.

use crate::common::RealVm;

/// A VM with a room that makes things, written by the test.
///
/// Two creature templates that differ only in id, so a weighted pick has
/// something to choose between and the assertions can name what it chose.
fn nest() -> RealVm {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval(
        "DAEMON.mobs.register_all({ \
             { id = 'probe_common', name = 'thing', short = 'a common thing', \
               description = 'Common.', \
               stats = { hp = 5, max_hp_flat = 5, level = 1 } }, \
             { id = 'probe_rare', name = 'thing', short = 'a rare thing', \
               description = 'Rare.', \
               stats = { hp = 5, max_hp_flat = 5, level = 1 } }, \
         }) return 'ok'",
    )
    .unwrap();
    vm
}

/// Give a fixture room a spawner and register it, which is what tells the
/// daemon about it — the same path an area load takes.
fn give_spawner(vm: &mut RealVm, room_id: &str, max: i64, interval: i64, table: &str) {
    vm.eval(&format!(
        "local room = DAEMON.world.get_room('{room_id}') \
         room.spawn_max = {max} room.spawn_interval = {interval} \
         room.spawn_table = {table} \
         DAEMON.world.register_room(room) return 'ok'"
    ))
    .unwrap();
}

fn population(vm: &mut RealVm, room_id: &str) -> i64 {
    vm.eval(&format!("return tostring(#DAEMON.mobs.in_room('{room_id}'))"))
        .unwrap()
        .parse()
        .unwrap()
}

/// **The cap spans the whole table, which is the point.**
///
/// Two kinds, three slots. `populate()` could only have said "two of each",
/// which is four; the question "how many creatures should this room hold" had no
/// answer before a spawner existed.
#[test]
fn the_cap_is_across_kinds_and_not_per_template() {
    let mut vm = nest();
    give_spawner(
        &mut vm,
        "fixture.cellar",
        3,
        60,
        "{ { template = 'probe_common', weight = 1 }, { template = 'probe_rare', weight = 1 } }",
    );

    vm.eval("return tostring(DAEMON.spawner.fill('fixture.cellar'))").unwrap();
    assert_eq!(population(&mut vm, "fixture.cellar"), 3, "filled to spawn_max");

    // …and it stops there, however often it is asked.
    for _ in 0..5 {
        vm.eval("DAEMON.spawner.tick('fixture.cellar') return 'ok'").unwrap();
    }
    assert_eq!(
        population(&mut vm, "fixture.cellar"),
        3,
        "a full room must not keep filling — this is the assertion the whole \
         daemon exists for"
    );
}

/// Killing one lets exactly one more in, one tick at a time.
#[test]
fn a_tick_replaces_one_and_only_one() {
    let mut vm = nest();
    give_spawner(&mut vm, "fixture.cellar", 3, 60,
                 "{ { template = 'probe_common', weight = 1 } }");
    vm.eval("DAEMON.spawner.fill('fixture.cellar') return 'ok'").unwrap();

    vm.eval(
        "local room = DAEMON.mobs.in_room('fixture.cellar') \
         DAEMON.mobs.despawn(room[1]) DAEMON.mobs.despawn(room[2]) return 'ok'",
    )
    .unwrap();
    assert_eq!(population(&mut vm, "fixture.cellar"), 1);

    assert_eq!(
        vm.eval("return tostring(DAEMON.spawner.tick('fixture.cellar'))").unwrap(),
        "1",
        "one tick, one creature"
    );
    assert_eq!(
        population(&mut vm, "fixture.cellar"),
        2,
        "a tick tops up by one rather than refilling — a cleared room should \
         come back at a rate the player can outrun"
    );
}

/// The weights are relative to each other, and the pick is where they are read.
#[test]
fn the_pick_is_weighted() {
    let mut vm = nest();
    give_spawner(
        &mut vm,
        "fixture.cellar",
        1,
        60,
        "{ { template = 'probe_common', weight = 9 }, { template = 'probe_rare', weight = 1 } }",
    );

    // Bottom of the range picks the first entry, top picks the last. Pinning
    // the roll is what makes a weighted pick testable at all.
    let low = vm
        .eval(
            "DAEMON.spawner._random = function() return 0.0 end \
             DAEMON.spawner.tick('fixture.cellar') \
             local m = DAEMON.mobs.in_room('fixture.cellar')[1] \
             local id = m.template_id DAEMON.mobs.despawn(m) return id",
        )
        .unwrap();
    assert_eq!(low, "probe_common");

    let high = vm
        .eval(
            "DAEMON.spawner._random = function() return 0.999 end \
             DAEMON.spawner.tick('fixture.cellar') \
             local m = DAEMON.mobs.in_room('fixture.cellar')[1] \
             local id = m.template_id DAEMON.mobs.despawn(m) return id",
        )
        .unwrap();
    assert_eq!(high, "probe_rare");
}

/// **The cap counts this spawner's kinds, not everything present.**
///
/// Counting every occupant would let a player switch a nest off by luring
/// something unrelated into the room, and would stop a patrol route through a
/// spawner room from ever refilling.
#[test]
fn something_unrelated_in_the_room_does_not_switch_the_spawner_off() {
    let mut vm = nest();
    give_spawner(&mut vm, "fixture.cellar", 2, 60,
                 "{ { template = 'probe_common', weight = 1 } }");

    // The fixture's mouse is not in this spawner's table.
    vm.eval("DAEMON.mobs.spawn('fixture_mouse', 'fixture.cellar') return 'ok'").unwrap();
    vm.eval("DAEMON.spawner.fill('fixture.cellar') return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.spawner.population('fixture.cellar'))").unwrap(),
        "2",
        "the spawner should count its own kinds"
    );
    assert_eq!(
        population(&mut vm, "fixture.cellar"),
        3,
        "…and the room holds those two plus the creature that wandered in"
    );
}

/// A room is indexed as it registers, and drops out when its spawner goes.
///
/// `world_d.register_room` is the one moment every path goes through — an area
/// load, an area reset, a `dig`, a virtual room being realised — which is why
/// the index is fed there and not by a reindex somebody has to remember.
#[test]
fn the_index_follows_registration() {
    let mut vm = nest();
    assert_eq!(
        vm.eval("return table.concat(DAEMON.spawner.rooms(), ',')").unwrap(),
        "",
        "the fixture world ships no spawners"
    );

    give_spawner(&mut vm, "fixture.cellar", 2, 60,
                 "{ { template = 'probe_common', weight = 1 } }");
    assert_eq!(
        vm.eval("return table.concat(DAEMON.spawner.rooms(), ',')").unwrap(),
        "fixture.cellar"
    );

    vm.eval(
        "local room = DAEMON.world.get_room('fixture.cellar') \
         room.spawn_max = 0 DAEMON.world.register_room(room) return 'ok'",
    )
    .unwrap();
    assert_eq!(
        vm.eval("return table.concat(DAEMON.spawner.rooms(), ',')").unwrap(),
        "",
        "a room that lost its spawner must drop out rather than linger"
    );
}

/// Half a spawner spawns nothing, and says so rather than sitting in the index.
#[test]
fn a_spawner_with_no_table_is_not_a_spawner() {
    let mut vm = nest();
    vm.eval(
        "local room = DAEMON.world.get_room('fixture.cellar') \
         room.spawn_max = 3 room.spawn_table = {} \
         DAEMON.world.register_room(room) return 'ok'",
    )
    .unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.spawner.spec('fixture.cellar'))").unwrap(),
        "nil",
        "spawn_max with an empty table is not a spawner"
    );
    assert_eq!(vm.eval("return table.concat(DAEMON.spawner.rooms(), ',')").unwrap(), "");
}

/// A table naming a creature that does not exist warns and spawns nothing,
/// rather than raising on the game thread every heartbeat for ever.
#[test]
fn an_unknown_template_is_refused_rather_than_raised() {
    let mut vm = nest();
    give_spawner(&mut vm, "fixture.cellar", 2, 60,
                 "{ { template = 'no_such_creature', weight = 1 } }");

    assert_eq!(
        vm.eval("return tostring(DAEMON.spawner.fill('fixture.cellar'))").unwrap(),
        "0"
    );
    assert_eq!(population(&mut vm, "fixture.cellar"), 0);
}

/// `spawn_max` is read off the live room, so an OLC edit takes effect at once.
///
/// This is what a spawner would lose by caching its numbers when it was first
/// noticed: it would become the one field a builder had to reload for, in a tool
/// whose whole claim is that `set` changes the world as you type.
#[test]
fn raising_the_cap_takes_effect_without_a_reload() {
    let mut vm = nest();
    give_spawner(&mut vm, "fixture.cellar", 1, 60,
                 "{ { template = 'probe_common', weight = 1 } }");
    vm.eval("DAEMON.spawner.fill('fixture.cellar') return 'ok'").unwrap();
    assert_eq!(population(&mut vm, "fixture.cellar"), 1);

    // No re-registration: just the field, as `olc set spawn_max 4` leaves it.
    vm.eval(
        "DAEMON.world.get_room('fixture.cellar').spawn_max = 4 \
         DAEMON.spawner.fill('fixture.cellar') return 'ok'",
    )
    .unwrap();
    assert_eq!(population(&mut vm, "fixture.cellar"), 4);
}

/// `fill_all` is idempotent, which is what makes it safe on every area reset.
#[test]
fn filling_twice_does_not_double_the_room() {
    let mut vm = nest();
    give_spawner(&mut vm, "fixture.cellar", 3, 60,
                 "{ { template = 'probe_common', weight = 1 } }");

    assert_eq!(vm.eval("return tostring(DAEMON.spawner.fill_all())").unwrap(), "3");
    assert_eq!(vm.eval("return tostring(DAEMON.spawner.fill_all())").unwrap(), "0");
    assert_eq!(population(&mut vm, "fixture.cellar"), 3);
}
