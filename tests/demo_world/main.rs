//! Tests of the world this repository ships — Thornhollow, the marsh, the mine,
//! the workshop — as opposed to tests of the mudlib that runs it.
//!
//! A game layer is content: "this game, and policy the driver has no view on".
//! So the demo is meant to be deleted by anyone building their own world, and
//! until this bucket existed doing that left a suite full of failures about
//! rooms they never wrote.
//!
//! The world under test is **`game.example/`**, not the `game/` the server
//! loads. Those are different trees on purpose: `game/` and `mudlib/` are the
//! creator's own, gitignored and absent on a clean clone, while `game.example/`
//! and `mudlib.default/` are what this repository ships and the only copies a
//! reviewer sees. A suite pointed at the live trees would assert content nobody
//! here wrote — and would fail on any checkout that has none.
//!
//! Everything in here asserts an authored value, an authored id, or authored
//! prose. **Delete this directory when you delete `game.example/`** and the rest
//! of the suite stays green — that is the whole contract, and the way to check
//! it is:
//!
//! ```text
//! mkdir ../away && mv game.example ../away/ && mv tests/demo_world ../away/
//! cargo test --test driver --test mudlib --no-fail-fast
//! mv ../away/game.example . && mv ../away/demo_world tests/ && rmdir ../away
//! ```
//!
//! Tests of mudlib *mechanics* do not belong here. If a test needs a world but
//! does not care which one, it wants `RealVm::boot_with_fixture_world` and a
//! place in `tests/mudlib/` — see `tests/mudlib/fixture_world.rs`.
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
