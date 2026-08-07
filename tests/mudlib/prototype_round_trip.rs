//! The regression test for the whole prototype feature.
//!
//! A prototyped record on disk is `prototype` plus what differs. Everything else
//! about the design is negotiable; this is not, and it is one edit away from
//! being lost at any time.
//!
//! The failure it exists to catch: `olc.draft` used to seed from
//! `olc.live_data`, which after resolution is the **flattened** template. Seeded
//! that way a draft holds every inherited value, and the very first `olc save`
//! writes them all out — so opening a child in OLC and changing its name would
//! silently turn it back into a hand-written twelve-key record that no longer
//! tracks its prototype. Everything would look fine. The diff would be enormous
//! and nobody would read it.
//!
//! Hence: **never subtract, never infer intent from value equality.** The draft
//! *is* the override set, so `serialize`, `codegen` and `olc.merged` needed to
//! learn nothing at all — and a builder who deliberately sets a value equal to
//! the inherited one keeps that intent, because nothing goes looking for it.

use crate::common::RealVm;

/// A logged-in builder in an OLC-managed area, with a prototype library.
fn building() -> RealVm {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    let dir = vm.game_root().unwrap().join("prototypes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("fixture.lua"),
        r#"return {
            mobs = {
                ["beast"] = {
                    race = "beast", faction = "wild", aggressive = true,
                    damage = { min = 1, max = 3 }, xp_award = 10,
                    tags = { "beast" }, respawn_time = 300,
                    stats = { strength = 10, dexterity = 12, constitution = 11 },
                },
                ["beast.lurker"] = {
                    prototype = "beast", name = "lurker",
                    stats = { dexterity = 15 }, patrol = { "crypt.entrance" },
                },
            },
        }"#,
    )
    .unwrap();
    vm.lua("require('prototypes').flush_cache() return 'ok'");

    vm.command("olc new area crypt The Sunken Crypt");
    vm
}

/// Read a generated file back off disk.
fn file(vm: &RealVm, area: &str, name: &str) -> String {
    std::fs::read_to_string(
        vm.game_root().unwrap().join("areas").join(area).join(format!("{name}.lua")),
    )
    .unwrap_or_default()
}

/// Collapse whitespace runs.
///
/// `serialize` pads each key to the longest one in its record, so `prototype =`
/// gains or loses spaces as a sibling field is added — and player output is
/// word-wrapped, so a phrase can arrive with a newline through the middle of it.
/// Neither is what any assertion here is about.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// **The one that matters.** A prototyped record survives an edit and a save
/// still holding `prototype` and only its overrides.
#[test]
fn a_save_keeps_the_prototype_and_writes_only_the_overrides() {
    let mut vm = building();

    let out = vm.command("olc new mob shale_lurker from proto:beast.lurker");
    assert!(out.contains("shale_lurker"), "{out}");

    // A record from a prototype is two keys. If `create` seeded it with the
    // prototype's values, the first save would write them all back out.
    assert_eq!(
        vm.lua(
            "local d = require('lib.olc').draft(SESSION, 'mob', 'shale_lurker') \
             local n = 0 for _ in pairs(d) do n = n + 1 end \
             return n .. '|' .. tostring(d.prototype) .. '|' .. tostring(d.faction)"
        ),
        "2|beast.lurker|nil",
        "a new record from a prototype is `id` and `prototype`, and nothing else"
    );

    vm.command("olc set short something under the shale");
    vm.command("olc set xp_award 130");
    vm.command("olc save");

    let mobs = file(&vm, "crypt", "mobs");
    assert!(
        norm(&mobs).contains(r#"prototype = "beast.lurker""#),
        "the link must survive:\n{mobs}"
    );
    assert!(mobs.contains("something under the shale"), "{mobs}");
    assert!(norm(&mobs).contains("xp_award = 130"), "{mobs}");

    // Nothing inherited leaked into the file. Each of these is in the prototype
    // chain and in the live template, and in neither case did this record say it.
    for leaked in ["faction", "aggressive", "respawn_time", "race", "strength"] {
        assert!(
            !mobs.contains(leaked),
            "'{leaked}' is inherited and must not be written into the child:\n{mobs}"
        );
    }

    // And the live world has the resolved form, because that is what a reload
    // would build. Overrides on disk, effective values in the game.
    assert_eq!(vm.lua("return tostring(DAEMON.mobs.get('shale_lurker').faction)"), "wild");
    assert_eq!(vm.lua("return tostring(DAEMON.mobs.get('shale_lurker').xp_award)"), "130");
    assert_eq!(
        vm.lua("return tostring(DAEMON.mobs.get('shale_lurker').stats.dexterity)"),
        "15",
        "the nearer prototype wins on a merged map key"
    );
    assert_eq!(
        vm.lua("return tostring(DAEMON.mobs.get('shale_lurker').stats.strength)"),
        "10",
        "and the root's other stat keys survive the merge"
    );
}

/// Re-opening a saved record and saving again changes nothing.
///
/// The property `serialize.lua`'s header already states, extended across the
/// resolver: a second save must be byte-identical, or the file churns on every
/// edit and every diff is noise.
#[test]
fn a_second_save_is_byte_identical() {
    let mut vm = building();
    vm.command("olc new mob shale_lurker from proto:beast.lurker");
    vm.command("olc set short something under the shale");
    vm.command("olc save");
    let first = file(&vm, "crypt", "mobs");

    // Leave and come back, so the draft is re-seeded from disk rather than
    // being the one still in the session.
    vm.command("olc done");
    vm.command("olc crypt");
    vm.command("olc edit mob:shale_lurker");
    vm.command("olc set short something under the shale");
    vm.command("olc save");

    // Compared without the generated header, which carries a timestamp — two
    // saves either side of a second boundary differ there and nowhere else, and
    // that is not what "byte-identical" is claiming.
    let body = |s: &str| {
        s.split_once("return {").map(|(_, rest)| rest.to_string()).unwrap_or_default()
    };
    assert_eq!(
        body(&file(&vm, "crypt", "mobs")),
        body(&first),
        "a re-save must not churn the records"
    );
}

/// `olc show` prints effective values, marked by where they came from.
///
/// The draft holds only overrides, and a builder who could not see the rest
/// would be editing blind — which is why the value shown and the value stored
/// are deliberately different things.
#[test]
fn show_prints_effective_values_and_marks_their_origin() {
    let mut vm = building();
    vm.command("olc new mob shale_lurker from proto:beast.lurker");
    vm.command("olc set xp_award 130");

    let out = vm.command("olc show");
    assert!(out.contains("beast.lurker -> beast"), "the chain, root last:\n{out}");
    assert!(out.contains("wild"), "an inherited value is still shown:\n{out}");
    assert!(out.contains("[beast]"), "and marked with which ancestor supplied it:\n{out}");
    assert!(out.contains("[beast.lurker]"), "the nearer one, where that is the source:\n{out}");
    assert!(out.contains("inherited"), "the legend is printed:\n{out}");
    assert!(out.contains("130"), "an override reads as an ordinary value:\n{out}");
}

/// `olc thin` drops what only restates the prototype. `olc save` never does.
#[test]
fn thin_is_the_only_thing_that_subtracts_and_it_is_asked_for() {
    let mut vm = building();
    vm.command("olc new mob shale_lurker from proto:beast.lurker");

    // Set two values: one that restates the prototype, one that does not.
    vm.command("olc set faction wild");
    vm.command("olc set xp_award 130");
    vm.command("olc save");

    // A save does NOT subtract the redundant one. That is deliberate: setting a
    // value equal to the inherited one means "this is mine now, and it must not
    // move if the prototype moves".
    assert!(
        norm(&file(&vm, "crypt", "mobs")).contains(r#"faction = "wild""#),
        "a save must never subtract"
    );

    let out = vm.command("olc thin");
    assert!(out.contains("faction"), "asked for, it goes: {out}");
    assert!(!out.contains("xp_award"), "and only what actually restates it: {out}");

    vm.command("olc save");
    let mobs = file(&vm, "crypt", "mobs");
    assert!(!mobs.contains("faction"), "now it is gone:\n{mobs}");
    assert!(norm(&mobs).contains("xp_award = 130"), "and the real override stayed:\n{mobs}");
}

/// `strike` writes the sentinel; `unset` reverts to inherited. Different things.
#[test]
fn strike_removes_an_inherited_field_and_unset_reverts_to_it() {
    let mut vm = building();
    vm.command("olc new mob shale_lurker from proto:beast.lurker");

    // `unset` on a field the record does not own says what actually happened.
    vm.command("olc set faction mine");
    let out = norm(&vm.command("olc unset faction"));
    assert!(out.contains("beast"), "it must name the source it reverted to: {out}");
    assert!(out.contains("wild"), "and the value that came back: {out}");
    assert!(out.contains("olc strike"), "and point at the thing that removes it: {out}");
    assert_eq!(vm.lua("return tostring(DAEMON.mobs.get('shale_lurker').faction)"), "wild");

    let out = vm.command("olc strike faction");
    assert!(out.contains("struck"), "{out}");
    assert_eq!(
        vm.lua("return tostring(DAEMON.mobs.get('shale_lurker').faction)"),
        "nil",
        "struck means gone from the live template, not set to '@none'"
    );

    vm.command("olc save");
    let mobs = file(&vm, "crypt", "mobs");
    assert!(norm(&mobs).contains(r#"faction = "@none""#), "the strike is in the file:\n{mobs}");

    // And it survives the round trip, which is the reason it is a string.
    vm.command("olc done");
    vm.command("olc crypt");
    vm.command("olc edit mob:shale_lurker");
    assert!(
        vm.command("olc show").contains("removed here"),
        "a struck field must read as struck after a reload, not as the literal string"
    );
}

/// Striking something nothing inherits is refused, and told what to type instead.
#[test]
fn strike_refuses_when_nothing_inherits_the_field() {
    let mut vm = building();
    vm.command("olc new mob shale_lurker from proto:beast.lurker");

    let out = vm.command("olc strike title");
    assert!(out.contains("Nothing inherits"), "{out}");
    assert!(out.contains("olc unset title"), "the refusal must be actionable: {out}");
}

/// A child missing an inherited required field is not an error.
///
/// Without the raw/resolved split in `verify_d`, every prototyped record in the
/// file lints as an error and `olc save` refuses — the feature would be unusable
/// on the day it shipped.
#[test]
fn verify_does_not_report_an_inherited_field_as_missing() {
    let mut vm = building();
    vm.command("olc new mob shale_lurker from proto:beast.lurker");
    vm.command("olc set short something under the shale");
    vm.command("olc set spawn_room crypt.entrance");

    let out = vm.command("olc save");
    assert!(!out.contains("Not written"), "a prototyped record must not gate a save:\n{out}");
    assert!(file(&vm, "crypt", "mobs").contains("shale_lurker"), "it should be on disk");

    // A strike, on the other hand, is reported — as a note, so it is visible in
    // the linter as well as in the file and the diff. That visibility is what
    // `patch.lua`'s no-sentinel rule was protecting, restored by other means.
    vm.command("olc strike patrol");
    let out = vm.command("olc save");
    assert!(out.contains("struck"), "a strike must be reported:\n{out}");
}

/// A prototype that does not resolve is refused at the moment it is typed,
/// rather than silently inheriting nothing.
#[test]
fn setting_a_prototype_checks_that_it_resolves() {
    let mut vm = building();
    vm.command("olc new mob plain_rat");

    let out = vm.command("olc set prototype beast.nosuch");
    assert!(out.contains("does not exist"), "{out}");
    assert_eq!(vm.lua("return tostring(DAEMON.mobs.get('plain_rat').prototype)"), "nil");

    let out = vm.command("olc set prototype beast.lurker");
    assert!(out.contains("inherits from beast.lurker"), "{out}");
    assert_eq!(vm.lua("return tostring(DAEMON.mobs.get('plain_rat').faction)"), "wild");
}

/// `olc protos` and `olc show proto:<id>` — the answer to "what am I inheriting".
#[test]
fn prototypes_can_be_inspected_but_not_edited() {
    let mut vm = building();
    vm.command("olc new mob shale_lurker from proto:beast.lurker");

    let out = vm.command("olc protos mob");
    assert!(out.contains("beast.lurker"), "{out}");
    assert!(out.contains("<- beast"), "the parent is shown: {out}");
    assert!(out.contains("1 use"), "and how many records would be affected: {out}");

    let out = vm.command("olc show proto:beast");
    assert!(out.contains("prototype"), "{out}");
    assert!(out.contains("wild"), "a prototype renders with the ordinary field walk: {out}");
    assert!(
        out.contains("OLC never writes prototypes"),
        "and says so, because they are hand-written like custom.lua: {out}"
    );
}

/// The existing `from <component>` grammar is unchanged.
///
/// `from` already meant "a component" and is already a reserved OLC keyword for
/// it. Silently preferring a prototype would change what an already-typed
/// command does, which is the one thing a grammar may never do.
#[test]
fn from_still_means_a_component_first() {
    let mut vm = building();

    vm.command("olc new item shiv from weapon");
    assert_eq!(
        vm.lua(
            "local d = require('lib.olc').draft(SESSION, 'item', 'shiv') \
             return tostring(d.prototype) .. '|' .. tostring((d.components or {})[1])"
        ),
        "nil|weapon",
        "`from weapon` is the component, as it always was"
    );
}
