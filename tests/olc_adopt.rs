//! `olc adopt` — bringing a hand-authored area under OLC.
//!
//! OLC regenerates an area's data files wholesale, which is only safe when the
//! data is data. A hand-authored area is not: thornhollow's square carries two
//! inline action functions and greywater_marsh's descriptions are lfuns keyed on
//! the weather. Regenerating either would delete them, and the file would still
//! compile — which is exactly the failure that would go unnoticed.
//!
//! So `_meta.managed` gates every write and this is the only thing that sets it,
//! in two steps: report, then `--confirm`.
//!
//! **No Lua source is ever parsed.** The obvious implementation lifts each
//! function body into the new `custom.lua`, which is a source transformation and
//! would fail subtly. The original is copied aside instead and referenced.

mod common;

use common::RealVm;

fn one_line(src: &str) -> String {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

struct Vm {
    vm: RealVm,
    game: std::path::PathBuf,
}

impl Vm {
    fn new() -> Self {
        let mut vm = RealVm::boot_fixture_with_probe();
        let game = vm.game_root().unwrap().to_path_buf();
        vm.eval("A = DAEMON.adopt CG = require('daemons.codegen_d') return 'ready'")
            .unwrap();
        Self { vm, game }
    }
    fn run(&mut self, src: &str) -> String {
        self.vm.eval(&one_line(src)).unwrap()
    }
    fn report(&mut self, area: &str, confirm: bool) -> String {
        self.run(&format!(
            "return table.concat(A.run(nil, '{area}', {confirm}), '\\n')"
        ))
    }
    /// A hand-authored area: prose data plus a room action and an lfun.
    fn write_hand_authored(&mut self, area: &str) {
        self.run(&format!(
            "write_file('game:areas/{area}/rooms.lua', [==[ \
             local function pull(session_id) return 'pulled' end \
             return {{ \
               {{ id = '{area}.square', short = 'The Square', \
                  description = 'Packed earth and a stone well.', \
                  tags = {{ 'outdoor' }}, exits = {{ north = '{area}.well' }}, \
                  actions = {{ pull = {{ func = pull, hint = 'pull the rope' }} }}, \
                  puzzle_seed = 17 }}, \
               {{ id = '{area}.well', short = 'The Well', \
                  description = function(room) return 'It depends on the weather.' end, \
                  tags = {{ 'outdoor' }}, exits = {{ south = '{area}.square' }} }} \
             }} ]==])"
        ));
    }
}

/// The dry run reports and writes nothing.
#[test]
fn the_dry_run_reports_and_writes_nothing() {
    let mut vm = Vm::new();
    vm.write_hand_authored("oldtown");

    let out = vm.report("oldtown", false);
    assert!(out.contains("not OLC-managed"), "{out}");
    assert!(out.contains("rooms.lua"), "{out}");
    assert!(out.contains("Nothing has been written"), "{out}");

    // Nothing on disk moved, and the gate is still closed.
    assert!(!vm.game.join("areas/oldtown/legacy_rooms.lua").exists());
    assert!(!vm.game.join("areas/oldtown/custom.lua").exists());
    assert!(!vm.game.join("areas/oldtown/_meta.lua").exists());
    assert_eq!(vm.run("return tostring((CG.is_managed('oldtown')))"), "false");
}

/// The report names every field that would move, and why.
#[test]
fn the_report_names_what_would_move_and_what_is_merely_unknown() {
    let mut vm = Vm::new();
    vm.write_hand_authored("oldtown");

    let out = vm.report("oldtown", false);

    // A room action and an lfun description both move — they are code.
    assert!(out.contains("Moves to custom.lua"), "{out}");
    assert!(out.contains("actions"), "{out}");
    assert!(out.contains("hand-written"), "{out}");
    assert!(out.contains("oldtown.well"), "the lfun room should be named: {out}");

    // A field no schema names is **kept**, and reported separately. Dropping it
    // silently is the bug class this whole design exists to end.
    assert!(out.contains("Named by no schema"), "{out}");
    assert!(out.contains("puzzle_seed"), "{out}");
}

/// `--confirm` copies the originals aside, writes `custom.lua`, and sets the gate.
#[test]
fn confirming_copies_the_originals_and_sets_the_gate() {
    let mut vm = Vm::new();
    vm.write_hand_authored("oldtown");

    let out = vm.report("oldtown", true);
    assert!(out.contains("now OLC-managed"), "{out}");

    // Nothing was deleted: the original is beside the new one.
    assert!(vm.game.join("areas/oldtown/legacy_rooms.lua").exists());
    assert!(vm.game.join("areas/oldtown/custom.lua").exists());
    assert!(vm.game.join("areas/oldtown/rooms.lua").exists());
    assert_eq!(vm.run("return tostring((CG.is_managed('oldtown')))"), "true");

    // The generated file is data — no functions survived into it.
    let rooms = std::fs::read_to_string(vm.game.join("areas/oldtown/rooms.lua")).unwrap();
    assert!(!rooms.contains("function"), "a function reached the data file:\n{rooms}");
    assert!(rooms.contains("The Square"), "{rooms}");
    // …and the unknown field came through verbatim.
    assert!(rooms.contains("puzzle_seed"), "an unknown field was dropped:\n{rooms}");

    // `custom.lua` references the copy rather than containing the source.
    let custom = std::fs::read_to_string(vm.game.join("areas/oldtown/custom.lua")).unwrap();
    assert!(custom.contains("legacy_rooms"), "{custom}");
    assert!(custom.contains("actions"), "{custom}");
    assert!(
        !custom.contains("return 'pulled'"),
        "the function body was extracted — that is a source transformation:\n{custom}"
    );
}

/// Everything the adoption wrote compiles, and the area loads with its
/// behaviour intact.
#[test]
fn an_adopted_area_loads_with_its_behaviour_restored() {
    let mut vm = Vm::new();
    vm.write_hand_authored("oldtown");
    vm.report("oldtown", true);

    for file in ["rooms", "custom", "legacy_rooms", "_meta"] {
        assert_eq!(
            vm.run(&format!(
                "return tostring((verify_file('game:areas/oldtown/{file}.lua')))"
            )),
            "true",
            "{file}.lua does not compile"
        );
    }

    let loaded = vm.run(
        "local a = require('lib.areaload') local ok, err = a.load('oldtown') \
         return tostring(ok) .. '|' .. tostring(err)",
    );
    assert_eq!(loaded, "true|nil");

    // The data survived…
    assert_eq!(vm.run("return DAEMON.world.get_room('oldtown.square').short"), "The Square");
    // …and so did the behaviour, through custom.lua.
    assert_eq!(
        vm.run("return tostring(DAEMON.world.get_room('oldtown.square').actions.pull ~= nil)"),
        "true",
        "the room action was lost"
    );
    assert_eq!(
        vm.run("return type(DAEMON.world.get_room('oldtown.well').long)"),
        "function",
        "the lfun description was lost"
    );
}

/// An adoption never overwrites hand-written code.
#[test]
fn an_existing_custom_lua_stops_the_adoption() {
    let mut vm = Vm::new();
    vm.write_hand_authored("oldtown");
    vm.run("write_file('game:areas/oldtown/custom.lua', 'return { rooms = {} }')");

    let out = vm.report("oldtown", true);
    assert!(out.contains("custom.lua already exists"), "{out}");
    assert!(out.contains("merge it by hand"), "{out}");
    assert!(out.contains("still unmanaged"), "{out}");

    // The gate is still closed, which is the point: a half-finished adoption
    // must leave OLC unable to touch the area.
    assert_eq!(vm.run("return tostring((CG.is_managed('oldtown')))"), "false");
    // And the hand-written file is untouched.
    assert_eq!(
        std::fs::read_to_string(vm.game.join("areas/oldtown/custom.lua")).unwrap(),
        "return { rooms = {} }"
    );
}

/// A second adoption over a previous one's leftovers is refused.
#[test]
fn a_leftover_legacy_file_stops_the_adoption() {
    let mut vm = Vm::new();
    vm.write_hand_authored("oldtown");
    vm.run("write_file('game:areas/oldtown/legacy_rooms.lua', 'return {}')");

    let out = vm.report("oldtown", true);
    assert!(out.contains("legacy_rooms.lua already exists"), "{out}");
    assert_eq!(vm.run("return tostring((CG.is_managed('oldtown')))"), "false");
}

/// `_meta.lua` is written **last**, so a failure part-way leaves the area
/// unmanaged and OLC still refuses it.
#[test]
fn the_gate_is_the_last_thing_written() {
    let mut vm = Vm::new();
    vm.write_hand_authored("oldtown");
    vm.run("write_file('game:areas/oldtown/custom.lua', 'return {}')");

    // This adoption fails at the custom.lua step.
    vm.report("oldtown", true);

    // The copy happened, the gate did not — which is the safe half-state.
    assert!(vm.game.join("areas/oldtown/legacy_rooms.lua").exists());
    assert!(!vm.game.join("areas/oldtown/_meta.lua").exists());
    assert_eq!(vm.run("return tostring((CG.is_managed('oldtown')))"), "false");
}

/// Adopting an already-managed area is refused rather than repeated.
#[test]
fn an_already_managed_area_is_refused() {
    let mut vm = Vm::new();
    vm.run("CG.write_meta('crypt', { title = 'Crypt' })");
    vm.run(
        "CG.write_kind('crypt', 'room', { { id = 'crypt.a', short = 'A', \
           description = 'a', exits = {} } })",
    );

    let out = vm.report("crypt", false);
    assert!(out.contains("already OLC-managed"), "{out}");
}

/// An area with no code to move needs no `custom.lua`.
#[test]
fn an_area_that_is_already_data_adopts_without_a_custom_file() {
    let mut vm = Vm::new();
    vm.run(
        "write_file('game:areas/plain/rooms.lua', [==[ return { \
           { id = 'plain.a', short = 'A', description = 'a', tags = { 'x' }, exits = {} } \
         } ]==])",
    );

    let out = vm.report("plain", true);
    assert!(out.contains("now OLC-managed"), "{out}");
    assert!(
        !vm.game.join("areas/plain/custom.lua").exists(),
        "an empty custom.lua is a file somebody has to wonder about"
    );
    assert!(vm.game.join("areas/plain/legacy_rooms.lua").exists());
}
