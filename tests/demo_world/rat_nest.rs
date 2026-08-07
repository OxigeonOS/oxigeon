//! The workshop's rat nest — a spawner and a prototype chain, together.
//!
//! Between them these two features answer a question the demo world could not
//! answer before: *where does a creature come from, and what kind is it?* The
//! pantry's `spawn_table` decides the first and `vermin.rat`'s three children
//! decide the second, and neither one knows about the other.
//!
//! What is asserted here is the shipped content. The spawner *mechanism* is
//! `tests/mudlib/spawners.rs`, against a fixture, because none of it is a claim
//! about this game.

use crate::common::RealVm;

fn world() -> RealVm {
    RealVm::boot_real_mudlib_with_probe()
}

/// The three rats are four lines each, and every number comes from the chain.
///
/// This is the case for prototypes stated as an assertion: `black_rat`'s file
/// entry is `{ id, prototype }` and nothing else, and it still has health,
/// damage, experience, loot, a body layout and a name.
#[test]
fn a_rat_is_four_lines_and_inherits_everything_else() {
    let mut vm = world();

    let authored = vm
        .eval(
            "local CG = require('daemons.codegen_d') \
             for _, d in ipairs(CG.read('wizard_workshop', 'mobs')) do \
                 if d.id == 'black_rat' then \
                     local keys = {} \
                     for k in pairs(d) do keys[#keys + 1] = k end \
                     table.sort(keys) \
                     return table.concat(keys, ',') \
                 end \
             end \
             return 'not found'",
        )
        .unwrap();
    assert_eq!(
        authored, "id,prototype",
        "the file should name only what makes this rat different, which is \
         nothing but which prototype it is"
    );

    // …and the registered template has everything.
    let t = vm
        .eval(
            "local t = DAEMON.mobs.get('black_rat') \
             return t.name .. '|' .. tostring(t.stats.max_hp_flat) .. '|' \
                 .. tostring(t.xp_award) .. '|' .. tostring(t.body) .. '|' \
                 .. tostring(#t.loot_table)",
        )
        .unwrap();
    assert_eq!(t, "rat|24|12|beast|1", "the chain did not resolve: {t}");
}

/// The three are genuinely different creatures, not three names for one.
#[test]
fn the_three_rats_differ_where_they_are_meant_to() {
    let mut vm = world();

    let read = |vm: &mut RealVm, id: &str| -> String {
        vm.eval(&format!(
            "local t = DAEMON.mobs.get('{id}') \
             return tostring(t.stats.max_hp_flat) .. '|' .. tostring(t.xp_award) \
                 .. '|' .. tostring(t.aggressive) .. '|' .. tostring(t.stats.constitution)"
        ))
        .unwrap()
    };

    assert_eq!(read(&mut vm, "black_rat"), "24|12|false|8", "the baseline");
    assert_eq!(read(&mut vm, "scrawny_rat"), "14|6|false|8", "weaker, worth less");
    assert_eq!(read(&mut vm, "muscular_rat"), "40|30|true|12", "and one to be careful of");

    // `stats` is a schema `map`, so a child naming four stats **merges** them.
    // The scrawny rat names `hp`, `max_hp_flat`, `strength` and `dexterity` and
    // keeps `vermin.rat`'s constitution of 8 — which is why the assertion above
    // reads 8 and not nil. An array field would have replaced the whole block.
    assert_eq!(
        vm.eval("return tostring(DAEMON.mobs.get('scrawny_rat').stats.wisdom)").unwrap(),
        "4",
        "a stat the child never mentions must survive the patch"
    );
}

/// The pantry fills to its `spawn_max` at load, and stops there.
#[test]
fn the_nest_fills_the_pantry_and_stops() {
    let mut vm = world();

    assert_eq!(
        vm.eval("return tostring(#DAEMON.mobs.in_room('wizard_workshop.pantry'))").unwrap(),
        "3",
        "`fill_all` runs beside `populate` so the room is not empty at boot"
    );

    // Ticked repeatedly, it adds nothing — the cap is across all three kinds,
    // which is the thing `mob_d.populate()` could not express.
    for _ in 0..10 {
        vm.eval("DAEMON.spawner.tick('wizard_workshop.pantry') return 'ok'").unwrap();
    }
    assert_eq!(
        vm.eval("return tostring(#DAEMON.mobs.in_room('wizard_workshop.pantry'))").unwrap(),
        "3",
        "three rats of any kind is the cap, however often the nest is asked"
    );
}

/// Clear the pantry and it comes back one rat at a time.
#[test]
fn a_cleared_pantry_refills_one_at_a_time() {
    let mut vm = world();
    vm.eval(
        "for _, m in ipairs(DAEMON.mobs.in_room('wizard_workshop.pantry')) do \
             DAEMON.mobs.despawn(m) \
         end return 'ok'",
    )
    .unwrap();
    assert_eq!(
        vm.eval("return tostring(#DAEMON.mobs.in_room('wizard_workshop.pantry'))").unwrap(),
        "0"
    );

    for expected in 1..=3 {
        vm.eval("DAEMON.spawner.tick('wizard_workshop.pantry') return 'ok'").unwrap();
        assert_eq!(
            vm.eval("return tostring(#DAEMON.mobs.in_room('wizard_workshop.pantry'))")
                .unwrap(),
            expected.to_string(),
            "one per tick"
        );
    }
}

/// **No rat is fed by two sources.**
///
/// `mob_d` schedules a respawn from `respawn_time` when a creature dies, and the
/// nest tops up on its own clock. A template carrying both is fed twice and the
/// room drifts past `spawn_max` one kill at a time — slowly enough to read as a
/// balance problem rather than as a bug. `verify` reports it; this asserts the
/// shipped content is clean.
#[test]
fn no_rat_in_the_nest_also_respawns_on_its_own() {
    let mut vm = world();

    let bad = vm
        .eval(
            "local room = DAEMON.world.get_room('wizard_workshop.pantry') \
             local out = {} \
             for _, e in ipairs(room.spawn_table or {}) do \
                 local t = DAEMON.mobs.get(e.template) \
                 if t and (t.respawn_time or t.spawn_room) then \
                     out[#out + 1] = e.template \
                 end \
             end \
             return #out == 0 and 'clean' or table.concat(out, ',')",
        )
        .unwrap();

    assert_eq!(
        bad, "clean",
        "these are spawned by the nest *and* by mob_d, so the pantry will drift \
         past its cap: {bad}"
    );
}

/// The nest is authored data, so `verify` checks it and OLC can edit it.
#[test]
fn the_spawner_is_authored_data_that_olc_owns() {
    let mut vm = world();

    // It is in the generated file, not bolted on in custom.lua.
    let on_disk = vm
        .eval(
            "local CG = require('daemons.codegen_d') \
             for _, r in ipairs(CG.read('wizard_workshop', 'rooms')) do \
                 if r.id == 'wizard_workshop.pantry' then \
                     return tostring(r.spawn_max) .. '|' .. tostring(#r.spawn_table) \
                 end \
             end \
             return 'not found'",
        )
        .unwrap();
    assert_eq!(on_disk, "3|3");

    // And the schema knows the fields, which is what makes `olc set` and
    // `verify` work on them without either learning a new kind.
    let known = vm
        .eval(
            "local schema = require('lib.schema') local out = {} \
             for _, f in ipairs(schema.fields_for('room', {})) do \
                 if f.name:match('^spawn') then out[#out + 1] = f.name end \
             end \
             table.sort(out) return table.concat(out, ',')",
        )
        .unwrap();
    assert_eq!(known, "spawn_interval,spawn_max,spawn_table");
}

/// **A hostile ability aims at what you are already fighting.**
///
/// `perf emberlance` used to answer "At what?" while a rat was biting you, and
/// the mechanism to fix it existed — `cleave` declared `default_target =
/// "combat"` and nothing else did. It is defaulted from the declared *outcome*
/// now, so an ability that attacks or damages a creature aims at your fight
/// without every author remembering a line.
///
/// This is also what makes always-disambiguating safe: the case where a player
/// has no time to pick is served without a name at all.
#[test]
fn a_hostile_ability_needs_no_target_once_you_are_fighting() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    assert_eq!(
        vm.eval("return tostring(DAEMON.ability.get('emberlance').default_target)").unwrap(),
        "combat",
        "an attacking ability should aim at your fight by default"
    );

    // …and a healing one should not, which is what proves it reads the outcome
    // rather than assuming every creature-targeting ability is hostile.
    assert_ne!(
        vm.eval("return tostring(DAEMON.ability.get('mend').default_target)").unwrap(),
        "combat",
        "a heal should not default to whatever is biting you"
    );
}

/// **This server is configured to name creatures by their short.**
///
/// `game.display_name_prefers` defaulted to `name`, so a black rat, another
/// black rat and a muscular red rat all reported as `rat` — unreadable the
/// moment a nest puts three of them in one room.
///
/// Asserted against `config/server.toml` rather than through a booted VM,
/// because the harness supplies its own config and would only be told what this
/// test already assumed. The *mechanism* — that the key reaches Lua and changes
/// the naming — is `tests/mudlib/display_name.rs`; what is shipped is a
/// deployment decision and lives in a file neither layer owns.
#[test]
fn this_server_names_creatures_by_their_short() {
    let toml = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config/server.toml"),
    )
    .expect("config/server.toml");

    assert!(
        toml.contains(r#"display_name_prefers = "short""#),
        "the rats all read as `rat` again — three creatures in a room need \
         their shorts to be told apart"
    );

    // …and the three rats really do have distinguishable shorts to show.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    let shorts = vm
        .eval(
            "local out = {} \
             for _, id in ipairs({ 'black_rat', 'scrawny_rat', 'muscular_rat' }) do \
                 out[#out + 1] = DAEMON.mobs.get(id).short \
             end \
             return table.concat(out, '|')",
        )
        .unwrap();
    assert_eq!(
        shorts,
        "a black rat|a scrawny grey rat|a muscular red rat",
        "preferring shorts buys nothing if the shorts are the same"
    );
}
