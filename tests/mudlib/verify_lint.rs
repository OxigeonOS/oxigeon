//! `verify` as a content linter.
//!
//! The old one answered "does this file parse", which is worth knowing and is
//! not the question a builder has. A file can compile perfectly and still
//! describe a room with an exit into nothing, a creature whose loot names an
//! item that does not exist, or a passage you can walk down and not back.
//!
//! Two properties above all others:
//!
//! * **a clean area produces zero findings.** A linter with one false positive
//!   is a linter people learn to ignore, and then it catches nothing at all;
//! * **each finding fires exactly once**, at the right severity. An over-firing
//!   check is as useless as a silent one, in a way that is harder to notice.

use crate::common::RealVm;

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
        vm.eval("CG = require('daemons.codegen_d') V = DAEMON.verify return 'ready'")
            .unwrap();
        Self { vm, game }
    }
    fn run(&mut self, src: &str) -> String {
        self.vm.eval(&one_line(src)).unwrap()
    }
    /// Write an area's files straight to disk, bypassing OLC.
    fn write(&mut self, lua: &str) {
        self.run(lua);
    }
    /// Every finding, as `level|where|what|message` lines.
    fn lint(&mut self, area: &str) -> String {
        self.run(&format!(
            "local r = V.area('{area}') local out = {{}} \
             for _, f in ipairs(r.findings) do \
               out[#out+1] = f.level .. '|' .. f.where .. '|' .. tostring(f.what) \
                             .. '|' .. f.message end \
             return table.concat(out, '\\n')"
        ))
    }
    fn counts(&mut self, area: &str) -> String {
        self.run(&format!(
            "local c = V.area('{area}').counts \
             return c.error .. ',' .. c.warn .. ',' .. c.note .. ',' .. c.lossy"
        ))
    }
}

/// A clean area is silent.
///
/// The load-bearing one. Everything else here is about catching problems, and a
/// linter that also reports non-problems gets turned off.
#[test]
fn a_clean_area_produces_no_findings() {
    let mut vm = Vm::new();

    vm.write(
        "CG.write_meta('crypt', { title = 'Crypt', author = 'Wren', entrance = 'crypt.entrance' })",
    );
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.entrance', short = 'The Entrance', description = 'A way down.', \
             tags = { 'indoor' }, exits = { down = 'crypt.hall' } }, \
           { id = 'crypt.hall', short = 'The Hall', description = 'Bones.', \
             tags = { 'indoor' }, exits = { up = 'crypt.entrance' } } })",
    );
    vm.write(
        "CG.write_kind('crypt', 'item', { \
           { id = 'crypt_lantern', short = 'a lantern', description = 'Tin.', weight = 2 } })",
    );
    vm.write(
        "CG.write_kind('crypt', 'mob', { \
           { id = 'crypt_eel', name = 'eel', short = 'a grey eel', \
             description = 'Six feet of muscle.', spawn_room = 'crypt.hall', \
             loot_table = { { item_id = 'crypt_lantern', chance = 0.4 } } } })",
    );

    let findings = vm.lint("crypt");
    assert_eq!(findings, "", "a clean area should be silent, got:\n{findings}");
    assert_eq!(vm.counts("crypt"), "0,0,0,0");
}

/// An exit into nothing is an error; a one-way passage is a warning.
///
/// One-way is legal and sometimes deliberate — a chute, a trapdoor — so it is
/// not an error. It is also the single easiest thing to do by accident.
#[test]
fn a_dangling_exit_errors_and_a_one_way_warns() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.entrance', short = 'A', description = 'a', tags = { 'x' }, \
             exits = { north = 'crypt.nowhere', down = 'crypt.pit' } }, \
           { id = 'crypt.pit', short = 'B', description = 'b', tags = { 'x' }, exits = {} } })",
    );

    let findings = vm.lint("crypt");

    let dangling: Vec<_> = findings
        .lines()
        .filter(|l| l.contains("crypt.nowhere"))
        .collect();
    assert_eq!(dangling.len(), 1, "should fire exactly once:\n{findings}");
    assert!(dangling[0].starts_with("error|"), "{findings}");

    let one_way: Vec<_> = findings.lines().filter(|l| l.contains("one-way")).collect();
    assert_eq!(one_way.len(), 1, "should fire exactly once:\n{findings}");
    assert!(one_way[0].starts_with("warn|"), "{findings}");
}

/// A room nothing leads to is a warning, from the declared entrance.
///
/// Guessing an entrance — "the room with no inbound edges" — would pick a
/// different one after every edit and make the list flap between runs, which is
/// how a check stops being read.
#[test]
fn an_unreachable_room_is_reported_from_the_declared_entrance() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.entrance' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.entrance', short = 'A', description = 'a', tags = { 'x' }, exits = {} }, \
           { id = 'crypt.hidden', short = 'B', description = 'b', tags = { 'x' }, exits = {} } })",
    );

    let findings = vm.lint("crypt");
    let orphans: Vec<_> = findings.lines().filter(|l| l.contains("unreachable")).collect();
    assert_eq!(orphans.len(), 1, "exactly the one orphan:\n{findings}");
    assert!(orphans[0].contains("crypt.hidden"), "{findings}");
    assert!(orphans[0].starts_with("warn|"), "{findings}");

    // With no entrance at all it says so rather than reporting every room.
    vm.write("CG.write_meta('lost', { author = 'Wren' })");
    vm.write(
        "CG.write_kind('lost', 'room', { \
           { id = 'lost.a', short = 'A', description = 'a', tags = { 'x' }, exits = {} } })",
    );
    let findings = vm.lint("lost");
    assert!(findings.contains("no entrance"), "{findings}");
    // "unreachable rooms cannot be detected" is the *explanation*, not a
    // finding — so match the finding's own shape rather than the word.
    assert!(
        !findings.lines().any(|l| l.contains("|lost.a|unreachable from")),
        "it should not guess an entrance:\n{findings}"
    );
}

/// Two records with one id: the later wins and the earlier is gone.
#[test]
fn a_duplicate_id_is_an_error() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.a', short = 'First', description = 'a', tags = { 'x' }, exits = {} }, \
           { id = 'crypt.a', short = 'Second', description = 'b', tags = { 'x' }, exits = {} } })",
    );

    let findings = vm.lint("crypt");
    let dupes: Vec<_> = findings.lines().filter(|l| l.contains("duplicate")).collect();
    assert_eq!(dupes.len(), 1, "{findings}");
    assert!(dupes[0].starts_with("error|"), "{findings}");
}

/// A reference to something that does not exist yet is a warning, not an error.
///
/// Forward references are the normal case while building — you set `spawn_room`
/// before digging it — so an area mid-build must not read as broken.
#[test]
fn unresolved_references_warn_rather_than_error() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.a', short = 'A', description = 'a', tags = { 'x' }, exits = {} } })",
    );
    vm.write(
        "CG.write_kind('crypt', 'mob', { \
           { id = 'picker', name = 'picker', short = 'a picker', description = 'x', \
             spawn_room = 'crypt.not_dug', \
             loot_table = { { item_id = 'no_such_item', chance = 1 } }, \
             inventory = { 'also_missing' } } })",
    );

    let findings = vm.lint("crypt");
    for needle in ["crypt.not_dug", "no_such_item", "also_missing"] {
        let hits: Vec<_> = findings.lines().filter(|l| l.contains(needle)).collect();
        assert_eq!(hits.len(), 1, "'{needle}' should fire once:\n{findings}");
        assert!(hits[0].starts_with("warn|"), "'{needle}' should be a warning:\n{findings}");
    }
    assert_eq!(
        vm.counts("crypt").split(',').next().unwrap(),
        "0",
        "a forward reference is not an error:\n{findings}"
    );
}

/// A trait nothing defines is stored and ignored, so it is a warning.
#[test]
fn an_unknown_trait_is_reported() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.a', short = 'A', description = 'a', tags = { 'x' }, exits = {} } })",
    );
    vm.write(
        "CG.write_kind('crypt', 'mob', { \
           { id = 'picker', name = 'p', short = 'a p', description = 'x', \
             stats = { level = 4, swimming = 20 } } })",
    );

    let findings = vm.lint("crypt");
    let hits: Vec<_> = findings.lines().filter(|l| l.contains("swimming")).collect();
    assert_eq!(hits.len(), 1, "{findings}");
    assert!(hits[0].starts_with("warn|"), "{findings}");
    // `level` is a real trait and must not be reported.
    assert!(!findings.contains("stats.level"), "a real trait was flagged:\n{findings}");
}

/// A component that does not exist is an error — nothing will build it.
#[test]
fn an_unknown_component_is_an_error() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.a', short = 'A', description = 'a', tags = { 'x' }, exits = {} } })",
    );
    vm.write(
        "CG.write_kind('crypt', 'item', { \
           { id = 'thing', short = 'a thing', description = 'x', \
             components = { 'sparkly' } } })",
    );

    let findings = vm.lint("crypt");
    let hits: Vec<_> = findings.lines().filter(|l| l.contains("sparkly")).collect();
    assert_eq!(hits.len(), 1, "{findings}");
    assert!(hits[0].starts_with("error|"), "{findings}");
    assert!(hits[0].contains("weapon"), "it should list what exists:\n{findings}");
}

/// A `custom.lua` patch naming an id nothing declares is an error.
///
/// The way a rename gets noticed: the patch that carried a room's actions now
/// points at nothing, and the room has quietly lost its behaviour.
#[test]
fn a_custom_patch_for_a_missing_id_is_an_error() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.a', short = 'A', description = 'a', tags = { 'x' }, exits = {} } })",
    );
    vm.write(
        "write_file('game:areas/crypt/custom.lua', \
           'return { rooms = { [\"crypt.gone\"] = { smell = \"x\" } } }')",
    );

    let findings = vm.lint("crypt");
    let hits: Vec<_> = findings.lines().filter(|l| l.contains("crypt.gone")).collect();
    assert_eq!(hits.len(), 1, "{findings}");
    assert!(hits[0].starts_with("error|custom.lua"), "{findings}");
}

/// Anything a save would destroy gets its own severity.
///
/// Not an error, because the area works; not a warning, because it is the only
/// category about *data loss*. Buried among the warnings it gets skimmed past.
#[test]
fn a_field_that_would_not_survive_a_save_is_lossy() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    // Written by hand rather than through codegen, which would refuse it —
    // this is the shape a hand-authored area has.
    vm.write(
        "write_file('game:areas/crypt/rooms.lua', [==[ return { { \
           id = 'crypt.a', short = 'A', tags = { 'x' }, exits = {}, \
           description = function(room) return 'computed' end, \
           actions = { pull = { func = function() end, hint = 'pull' } } } } ]==])",
    );

    let findings = vm.lint("crypt");
    let lossy: Vec<_> = findings.lines().filter(|l| l.starts_with("lossy|")).collect();
    assert!(
        lossy.iter().any(|l| l.contains("description")),
        "an lfun description should be lossy:\n{findings}"
    );
    assert!(
        lossy.iter().any(|l| l.contains("actions")),
        "room actions should be lossy:\n{findings}"
    );
    for line in &lossy {
        assert!(line.contains("custom.lua"), "it should say where they go: {line}");
    }
}

/// A field no schema names is **kept** and reported, never dropped silently.
#[test]
fn an_unknown_field_is_a_note_rather_than_a_loss() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.a', short = 'A', description = 'a', tags = { 'x' }, \
             exits = {}, puzzle_seed = 17 } })",
    );

    let findings = vm.lint("crypt");
    let hits: Vec<_> = findings.lines().filter(|l| l.contains("puzzle_seed")).collect();
    assert_eq!(hits.len(), 1, "{findings}");
    assert!(hits[0].starts_with("note|"), "a writable unknown field is not a loss:\n{findings}");

    // …and it really does survive a rewrite.
    assert_eq!(
        vm.run(
            "CG.write_kind('crypt', 'room', CG.read('crypt', 'rooms')) \
             return tostring(CG.read('crypt', 'rooms')[1].puzzle_seed)"
        ),
        "17"
    );
}

/// The report reads as a report.
#[test]
fn the_rendered_report_groups_by_severity() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.a', short = 'A', description = 'a', exits = { north = 'crypt.gone' } } })",
    );

    let rendered = vm.run(
        "return table.concat(V.render(V.area('crypt')), '\\n')",
    );
    assert!(rendered.contains("ERRORS"), "{rendered}");
    assert!(rendered.contains("NOTES"), "{rendered}");
    assert!(rendered.contains("1 error"), "the tally should be summarised:\n{rendered}");

    // A clean one says so plainly rather than printing empty headings.
    vm.write("CG.write_meta('ok_area', { author = 'Wren', entrance = 'ok_area.a' })");
    vm.write(
        "CG.write_kind('ok_area', 'room', { \
           { id = 'ok_area.a', short = 'A', description = 'a', tags = { 'x' }, exits = {} } })",
    );
    let clean = vm.run("return table.concat(V.render(V.area('ok_area')), '\\n')");
    assert!(clean.contains("no findings"), "{clean}");
    assert!(!clean.contains("ERRORS"), "empty headings should not print:\n{clean}");
}

/// The lint reads **disk**, not the registry.
///
/// The registry has already collapsed duplicate ids, dropped fields the loader
/// ignored and applied `custom.lua`, so it cannot answer "what will the next
/// reload do" — which is the only question a builder about to save has.
#[test]
fn the_lint_reads_what_is_on_disk() {
    let mut vm = Vm::new();
    vm.write("CG.write_meta('crypt', { author = 'Wren', entrance = 'crypt.a' })");
    vm.write(
        "CG.write_kind('crypt', 'room', { \
           { id = 'crypt.a', short = 'A', description = 'a', tags = { 'x' }, exits = {} } })",
    );
    assert_eq!(vm.counts("crypt"), "0,0,0,0");

    // Break the file behind the registry's back. Nothing has been loaded, so a
    // registry-based lint would still say it was fine.
    vm.write(
        "write_file('game:areas/crypt/rooms.lua', [==[ return { { \
           id = 'crypt.a', short = 'A', description = 'a', tags = { 'x' }, \
           exits = { north = 'crypt.vanished' } } } ]==])",
    );
    let findings = vm.lint("crypt");
    assert!(findings.contains("crypt.vanished"), "the lint did not read disk:\n{findings}");
}
