//! The **mudlib** suite — everything that is the Lua layer's job.
//!
//! The rule that decides what lives here:
//!
//! > If you deleted `mudlib/` and wrote your own from scratch, would you keep
//! > this test or rewrite it? Keep it → `tests/driver/`. **Rewrite it → here.**
//!
//! So this covers the daemons, the libs, the schema, OLC, commands, prototypes,
//! abilities, combat, traits and effects. Some of it needs a world; that world
//! is the **fixture**, never Thornhollow — anything asserting shipped content
//! belongs in `tests/demo_world/`, which is deleted along with `game/`.
//!
//! Delete `mudlib/` and you delete this directory with it. That is the point of
//! the split: `tests/driver/` must still pass.

#[path = "../common/mod.rs"]
mod common;

mod abilities;
mod body_layouts;
mod broken_traits;
mod codegen;
mod command_layout;
mod components;
mod display_name;
mod editor_d;
mod fixture_world;
mod fs_shell;
mod gmcp_inbound;
mod gmcp_outbound;
mod interleaving;
mod lua_unit;
mod messaging;
mod objdump_flags;
mod olc_adopt;
mod olc_grammar;
mod prototype_round_trip;
mod prototypes;
mod queues;
mod schema;
mod serialize;
mod spawners;
mod state_cache;
mod state_retention;
mod trait_sparsity;
mod verify_lint;
