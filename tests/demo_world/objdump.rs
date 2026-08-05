//! `objdump` dumps anything, not just players and rooms.
//!
//! It resolved two kinds — an **online** player by exact name, and a room by
//! exact id — and answered "Player or room not found." to everything else. So
//! `objdump rat` failed on a creature standing in front of you, and there was
//! no way at all to inspect an item instance, which is the thing most likely to
//! be carrying state you did not expect.
//!
//! These assertions pin the resolution chain, the forced `kind:` prefixes, and
//! the raw-field dump that makes the command worth running on something whose
//! interesting field nobody thought to curate.


use crate::common::RealVm;

/// The reported bug: a creature in the room is dumpable by keyword.
#[test]
fn objdump_resolves_a_creature_in_the_room() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("goto wizard_workshop.laboratory");

    for name in ["mephit", "dust", "dust_mephit"] {
        let out = vm.command(&format!("objdump {name}"));
        assert!(
            !out.contains("not found"),
            "`objdump {name}` did not resolve a creature in the room:\n{out}"
        );
        assert!(
            out.contains("Creature"),
            "`objdump {name}` resolved something that was not the creature:\n{out}"
        );
        assert!(
            out.contains("Instance:") && out.contains("Template:"),
            "a creature dump must name both the instance and its template:\n{out}"
        );
    }
}

/// A creature anywhere, not just the one you are standing next to.
#[test]
fn objdump_finds_a_creature_in_another_room() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("goto thornhollow.square"); // the watchman patrols from here
    vm.command("goto wizard_workshop.laboratory"); // now stand somewhere else

    let out = vm.command("objdump watchman");
    assert!(
        !out.contains("not found"),
        "`objdump` should find a live creature anywhere, not only in your room:\n{out}"
    );
}

/// No argument means the room you are standing in.
#[test]
fn objdump_with_no_argument_dumps_here() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("goto thornhollow.square");

    let out = vm.command("objdump");
    assert!(
        out.contains("thornhollow.square"),
        "bare `objdump` should dump the current room:\n{out}"
    );
    assert!(!out.contains("Usage:"), "it should not be a usage error:\n{out}");
}

/// The two original kinds still work.
#[test]
fn objdump_still_resolves_rooms_and_players() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let room = vm.command("objdump thornhollow.square");
    assert!(room.contains("Room"), "room dump regressed:\n{room}");
    assert!(
        room.contains("Exits:"),
        "room dump lost its exits line:\n{room}"
    );

    let player = vm.command("objdump benchuser");
    assert!(player.contains("Player"), "player dump regressed:\n{player}");
    assert!(
        player.contains("Char ID:"),
        "player dump lost its identity line:\n{player}"
    );
}

/// A room dump must list what is actually in the room, including the things
/// that live in a daemon's index rather than on the room table.
#[test]
fn a_room_dump_lists_creatures_and_ground_items() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("goto thornhollow.square");
    vm.command("spawn apprentice_dagger");
    vm.command("drop dagger");

    let out = vm.command("objdump");
    assert!(
        out.contains("Ground items:") && out.contains("apprentice_dagger"),
        "a room dump omitted an item lying in it:\n{out}"
    );
    assert!(
        out.contains("Creatures:"),
        "a room dump must have a creatures line even when empty:\n{out}"
    );
}

/// Item instances — the kind with the surprising state on it.
#[test]
fn objdump_resolves_an_item_instance() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("goto thornhollow.square");
    vm.command("spawn apprentice_dagger");

    let out = vm.command("objdump dagger");
    assert!(
        !out.contains("not found"),
        "`objdump` could not see an item in your own inventory:\n{out}"
    );
    assert!(out.contains("Item"), "expected an item dump:\n{out}");
    assert!(
        out.contains("Location:"),
        "an item dump must say where the instance is:\n{out}"
    );
}

/// Templates, so you can inspect something that has never been spawned.
#[test]
fn objdump_resolves_unspawned_templates() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let item = vm.command("objdump template:apprentice_dagger");
    assert!(
        item.contains("template") && item.contains("Not spawned"),
        "an item template should dump and say it is not an instance:\n{item}"
    );

    let mob = vm.command("objdump template:dust_mephit");
    assert!(
        mob.contains("template"),
        "a mob template should dump:\n{mob}"
    );
}

/// The raw dump is the point: it shows fields nobody curated a line for.
#[test]
fn every_dump_ends_with_raw_fields() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("goto wizard_workshop.laboratory");

    for target in ["mephit", "benchuser", "wizard_workshop.laboratory"] {
        let out = vm.command(&format!("objdump {target}"));
        assert!(
            out.contains("Raw fields:"),
            "`objdump {target}` produced no raw section:\n{out}"
        );
    }
}

/// A `kind:` prefix forces one branch.
#[test]
fn a_kind_prefix_forces_the_branch() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("goto wizard_workshop.laboratory");

    // Forced to a kind it is not, so it must fail rather than fall through to
    // the creature that shares the name.
    let out = vm.command("objdump room:mephit");
    assert!(
        out.contains("Nothing called"),
        "`room:` should not have resolved a creature:\n{out}"
    );

    let forced = vm.command("objdump mob:mephit");
    assert!(
        forced.contains("Creature"),
        "`mob:` should resolve the creature:\n{forced}"
    );
}

/// A failure has to say what it looked at, or you cannot tell a typo from a
/// thing that is genuinely not loaded.
#[test]
fn the_failure_names_what_was_searched() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let out = vm.command("objdump definitely_not_a_thing");
    assert!(out.contains("Searched:"), "failure said nothing useful:\n{out}");
    for kind in ["players", "rooms", "creatures", "items", "template"] {
        assert!(
            out.contains(kind),
            "the failure should name '{kind}' among what it searched:\n{out}"
        );
    }
}

/// Cycles must be marked, not followed. An Object's parent pointers close a
/// loop, and a stack overflow inside an admin command takes the game thread.
#[test]
fn the_raw_dump_survives_a_cycle() {
    // The probe boot, because this reaches into the command's internals rather
    // than typing a verb — and one line, because the probe path is
    // line-oriented and splits before Lua ever sees the source.
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let out = vm
        .eval(
            "local objdump = require('cmds.admin.objdump') \
             local a = { name = 'a' } a.self = a \
             a.child = { parent = a, leaf = 1 } \
             local lines = {} \
             local ok = pcall(objdump._dump_fields, lines, a, '  ', 3, nil) \
             return tostring(ok) .. '|' .. table.concat(lines, ' / ')",
        )
        .unwrap();

    assert!(
        out.starts_with("true"),
        "dumping a cyclic table raised instead of marking the cycle:\n{out}"
    );
    assert!(
        out.contains("(cycle)"),
        "the cycle was not marked:\n{out}"
    );
}
