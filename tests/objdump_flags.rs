//! `objdump`'s flags, and the one that matters.
//!
//! Three gaps, all of which made the command silent about exactly the thing you
//! opened it for:
//!
//! * **lfuns were opaque.** A function-valued field printed `<function>`, so the
//!   field you were debugging was the one you could not see.
//! * **depth was fixed at 2.** A weapon showed at depth 1, `weapon.damage` at 2,
//!   and anything deeper as `<table>`.
//! * **only the instance table was walked**, so everything inherited through the
//!   metatable was invisible and read as "does not exist".
//!
//! And one thing it could not answer at all: *what am I about to lose?* `-s`
//! marks every field against the schema, and `!` is a field `olc save` would
//! drop.

mod common;

use common::RealVm;

/// **The defaults do not move.** Every existing invocation prints what it always
/// did, which is what keeps a dump diffable against the last one — the reason
/// the file sorts its keys in the first place.
#[test]
fn the_defaults_are_unchanged() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");

    let plain = vm.command("objdump");
    assert!(plain.contains("Raw fields"), "{plain}");
    // No marker column, no legend, no resolution.
    assert!(!plain.contains("OLC-editable"), "the legend leaked into a plain dump:\n{plain}");
    assert!(!plain.contains(" -> "), "lfuns were resolved without being asked:\n{plain}");

    // Twice in a row is the same output, minus nothing.
    let again = vm.command("objdump");
    assert_eq!(plain, again, "a dump is not stable");
}

/// `-s` marks each field, and `!` names what a save would drop.
#[test]
fn the_schema_flag_marks_what_olc_owns_and_what_it_would_drop() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");

    let out = vm.command("objdump -s room:thornhollow.square");
    assert!(out.contains("OLC-editable"), "the legend should be shown:\n{out}");

    // `short` is authorable, so it carries the editable mark.
    assert!(
        out.contains("· short") || out.contains("· long"),
        "an editable field should be marked:\n{out}"
    );

    // A Room carries runtime fields no schema names — `contents`, the character
    // list — and those are precisely what a regeneration would lose.
    assert!(
        out.contains("! ") && out.contains("would drop"),
        "unknown fields should be reported:\n{out}"
    );
}

/// `-r` resolves lfuns, and only the fields the schema types as one.
///
/// A dump that called every stored function would be a dump with side effects,
/// and an admin command that changes the world by looking at it is a trap.
#[test]
fn the_resolve_flag_shows_what_an_lfun_returns() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");

    // greywater_marsh's descriptions are lfuns keyed on the weather.
    let plain = vm.command("objdump -s room:greywater_marsh.causeway_head");
    assert!(
        plain.contains("<function>"),
        "the area should have an lfun description:\n{plain}"
    );
    assert!(!plain.contains(" -> "), "nothing should resolve without -r:\n{plain}");

    let resolved = vm.command("objdump -s -r room:greywater_marsh.causeway_head");
    assert!(
        resolved.contains("<function> -> "),
        "-r should show what the lfun returns:\n{resolved}"
    );
}

/// `-d` nests further, and is bounded.
#[test]
fn the_depth_flag_goes_deeper_and_is_bounded() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");

    // `warded_cloak` nests three deep: item -> armour -> resist. That is where
    // the default runs out, and it is the shape every component has.
    let shallow = vm.command("objdump -d 1 template:warded_cloak");
    let deep = vm.command("objdump -d 4 template:warded_cloak");
    assert!(
        shallow.contains("<table>"),
        "at depth 1 the resist map should collapse:\n{shallow}"
    );
    assert!(
        deep.contains("magic") && !deep.contains("<table>"),
        "at depth 4 it should be expanded:\n{deep}"
    );

    // Absurd depths are clamped rather than refused: the cycle guard is what
    // actually protects the game thread, and clamping keeps a typo cheap.
    let clamped = vm.command("objdump -d 99 template:warded_cloak");
    assert!(clamped.contains("Raw fields"), "{clamped}");

    // A malformed flag says so rather than being read as a spec.
    assert!(vm.command("objdump -d template:warded_cloak").contains("needs a depth"));
    assert!(vm.command("objdump -z something").contains("Unknown flag"));
}

/// `-i` shows what is inherited, with methods collapsed.
///
/// Expanded identically to data, every room dump grows forty lines of `Room:`
/// methods and nobody turns the flag on twice.
#[test]
fn the_inherit_flag_shows_the_metatable_chain_without_drowning_it() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");

    let plain = vm.command("objdump template:apprentice_dagger");
    assert!(!plain.contains("Inherited"), "{plain}");

    let out = vm.command("objdump -i template:apprentice_dagger");
    assert!(out.contains("Inherited"), "{out}");
    assert!(
        out.contains("^ methods:"),
        "inherited methods should collapse to one line:\n{out}"
    );
    // …and the collapse is real: `has_tag` is an inherited method and should
    // appear in that line rather than as its own entry. `__index` must not
    // appear at all — it is the mechanism of inheritance, not a field.
    assert!(!out.contains("__index"), "metamethods leaked into the dump:\n{out}");
    let methods_line = out
        .lines()
        .find(|l| l.contains("^ methods:"))
        .unwrap_or("");
    assert!(methods_line.contains("has_tag"), "{out}");
}

/// `-a` is all of them.
#[test]
fn the_all_flag_turns_everything_on() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");

    let out = vm.command("objdump -a room:greywater_marsh.causeway_head");
    assert!(out.contains("OLC-editable"), "schema:\n{out}");
    assert!(out.contains("Inherited"), "inherit:\n{out}");
    assert!(out.contains("<function> -> "), "resolve:\n{out}");
}

/// The stored-versus-derived trait split survives, because it is the file's best
/// idea: `stats` holds nothing for a derived trait and the *unbuffed* number for
/// a buffed one, so dumping `stats` alone reports neither.
#[test]
fn traits_are_still_read_through_the_daemon() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");

    let out = vm.command("objdump benchuser");
    assert!(out.contains("Traits:"), "{out}");
    assert!(
        out.contains("[derived/") || out.contains("derived"),
        "derived traits should be labelled as such:\n{out}"
    );
}
