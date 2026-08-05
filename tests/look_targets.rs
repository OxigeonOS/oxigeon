//! `look <target>` and `examine <target>` must resolve the same things.
//!
//! They did not. `look` knew about room scenery and **exact** player names and
//! nothing else, so standing in a room whose description ended
//!
//!     a swirling dust mephit is here.
//!
//! and typing `l mephit` answered "You don't see that here." `examine mephit`
//! worked the whole time, which is the shape of the bug: two commands with two
//! copies of "what is in front of me", one of them three categories short.
//!
//! Every test that ever looked at a creature used `examine`, so the suite was
//! green. These assertions run both verbs against the same targets and compare.

mod common;

use common::RealVm;

/// The exact sequence from the bug report: stand in the lab, look at the mephit.
#[test]
fn look_finds_a_creature_in_the_room() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.command("goto wizard_workshop.laboratory");

    // It is in the room description...
    let room = vm.command("look");
    assert!(
        room.contains("dust mephit"),
        "the mephit should be in the room description:\n{room}"
    );

    // ...so every name it answers to must be lookable.
    for name in ["mephit", "dust", "swirling"] {
        let out = vm.command(&format!("look {name}"));
        assert!(
            !out.contains("don't see that here"),
            "`look {name}` failed on a creature standing in the room:\n{out}"
        );
        assert!(
            out.contains("mephit") || out.contains("dust"),
            "`look {name}` described something, but not the mephit:\n{out}"
        );
    }
}

/// The two verbs are one resolver, so they cannot drift apart again.
#[test]
fn look_and_examine_agree_on_every_category() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.command("goto wizard_workshop.laboratory");
    vm.command("affect learn strength 18"); // so nothing is refused for want of it

    for target in [
        "mephit",    // a creature
        "cauldron",  // room scenery
        "workbench", // room scenery, second keyword
    ] {
        let looked = vm.command(&format!("look {target}"));
        let examined = vm.command(&format!("examine {target}"));

        let look_failed = looked.contains("don't see that here");
        let exa_failed = examined.contains("You see no");

        assert_eq!(
            look_failed, exa_failed,
            "`look {target}` and `examine {target}` disagree about whether it exists\n\
             look:    {looked}\n\
             examine: {examined}"
        );
    }
}

/// An item on the floor is a thing in front of you. `look` could not see one.
#[test]
fn look_finds_an_item_on_the_floor() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.command("goto thornhollow.square");
    vm.command("spawn apprentice_dagger");
    vm.command("drop dagger");

    let out = vm.command("look dagger");
    assert!(
        !out.contains("don't see that here"),
        "`look` cannot see an item lying on the ground:\n{out}"
    );
    assert!(
        out.contains("lying here"),
        "`look` should describe it as being on the floor:\n{out}"
    );
}

/// Carried items too — and the same one `drop` would pick.
#[test]
fn look_finds_a_carried_item() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.command("goto thornhollow.square");
    vm.command("spawn apprentice_dagger");

    let out = vm.command("look dagger");
    assert!(
        !out.contains("don't see that here"),
        "`look` cannot see an item in your own hands:\n{out}"
    );
}

/// Looking closely at something needs light, not just reading the room.
///
/// `look` checked `Light.can_see` before every branch. `examine` checked
/// nothing, so in a pitch-dark room you could not read the description but
/// could still inspect the strongbox in it.
#[test]
fn examine_needs_light_the_same_way_look_does() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.command("goto collapsed_mine.adit");
    vm.command("spawn apprentice_dagger");

    let looked = vm.command("look");
    assert!(
        looked.contains("dark") || looked.contains("see nothing"),
        "the mine adit should be dark; this test proves nothing otherwise:\n{looked}"
    );

    let examined = vm.command("examine dagger");
    assert!(
        !examined.contains("Weight"),
        "`examine` described an item in a pitch-dark room:\n{examined}"
    );
}
