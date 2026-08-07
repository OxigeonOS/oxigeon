//! The mudlib works against a world that is not this repository's `game/`.
//!
//! `game/` is content — "this game, and policy the driver has no view on". So
//! somebody who deletes it to build their own world should not inherit a broken
//! test suite, and the mudlib's own tests should not silently depend on
//! Thornhollow existing.
//!
//! `boot_with_fixture_world` writes a three-room world into a temp directory
//! and boots the real mudlib against it. These assertions are what say that
//! world is genuinely enough to play in — which is the same as saying the
//! mudlib does not need the demo game.
//!
//! The end-to-end check for the whole exercise is coarser and lives outside the
//! suite: stash `game/` and `tests/demo_world/`, run `cargo test`, and it should
//! be green.

use crate::common::RealVm;

#[test]
fn a_player_can_log_in_and_look_around() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    let room = vm.command("look");
    assert!(room.contains("A Plain Hall"), "{room}");
    assert!(room.contains("door at each end"), "{room}");
}

#[test]
fn the_world_is_walkable_in_both_directions() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    assert!(vm.command("north").contains("Store Room"));
    assert!(vm.command("south").contains("Plain Hall"));
    // And the collapsed direction commands work against it too.
    assert!(vm.command("s").contains("Dark Cellar") || vm.command("look").contains("dark"));
}

#[test]
fn the_trait_set_is_the_games_to_supply_and_the_fixture_supplies_one() {
    // Traits are game-layer by design, so a world with none has no `hp` for
    // anything to lose. This is the part a naive stub forgets.
    let mut vm = RealVm::boot_with_fixture_world(0);
    let score = vm.command("score");

    assert!(score.contains("Health") || score.contains("hp"), "{score}");
    assert!(score.contains("Strength"), "{score}");
}

#[test]
fn a_creature_spawns_and_can_be_fought() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("north");

    let room = vm.command("look");
    assert!(room.contains("mouse"), "the fixture mob should be here:\n{room}");

    // It has the authored health, not the level-1 curve — the same
    // `max_hp_flat` rule the shipped game uses.
    let out = vm.command("attack mouse");
    assert!(
        !out.contains("don't see"),
        "the mouse should be attackable:\n{out}"
    );
}

#[test]
fn an_item_can_be_spawned_dropped_and_picked_up() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("spawn fixture_stone");

    assert!(vm.command("inventory").contains("stone"));
    vm.command("drop stone");
    assert!(vm.command("look").contains("stone"), "it should be on the floor");
    vm.command("get stone");
    assert!(vm.command("inventory").contains("stone"));
}

#[test]
fn the_fixture_does_not_reach_into_the_shipped_game() {
    // The point of the whole thing: nothing from `game/` is registered here.
    let mut vm = RealVm::boot_with_fixture_world(0);

    let out = vm.command("goto wizard_workshop.entrance");
    assert!(
        !out.contains("Entrance to the Workshop"),
        "the fixture world should not contain the demo world:\n{out}"
    );
}
