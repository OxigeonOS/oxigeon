//! Tests of the world this repository ships — Thornhollow, the marsh, the mine,
//! the workshop — as opposed to tests of the mudlib that runs it.
//!
//! `game/` is content: "this game, and policy the driver has no view on". So it
//! is meant to be deleted by anyone building their own world, and until now
//! doing that left a suite full of failures about rooms they never wrote.
//!
//! Everything in here asserts an authored value, an authored id, or authored
//! prose. **Delete this directory when you delete `game/`** and the rest of the
//! suite stays green — that is the whole contract, and the way to check it is:
//!
//! ```text
//! git stash push game tests/demo_world && cargo test && git stash pop
//! ```
//!
//! Tests of mudlib *mechanics* do not belong here. If a test needs a world but
//! does not care which one, it wants `RealVm::boot_with_fixture_world` and a
//! place in `tests/` — see `tests/fixture_world.rs`.
//!
//! One cargo target rather than twenty-one, because cargo discovers
//! `tests/<dir>/main.rs` as a single integration test. The modules below are
//! the files as they were.

#[path = "../common/mod.rs"]
mod common;

mod abilities;
mod authored_mob_health;
mod authoring;
mod board;
mod gmcp_game;
mod combat;
mod combat_mitigation;
mod defence_channels;
mod degrees_and_bodies;
mod equipment;
mod items_ground;
mod levelling;
mod lifecycle;
mod look_targets;
mod marsh;
mod mine;
mod objdump;
mod quests;
mod rat_nest;
mod real_mudlib_harness;
mod roles;
mod shipped_areas;
mod shop;
mod thornhollow;
mod traits_breadth;
mod trait_categories;
mod traits_effects;
mod tui_inspect_payload;
mod virtual_rooms;
mod world_graph;
