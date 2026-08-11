// Oxigeon library root — exposes all public modules for integration testing and external use.
pub mod config;
pub mod error;
pub mod core;
pub mod domain;
pub mod driver;

// ─── The test harness ────────────────────────────────────────────────────────
//
// Behind a non-default feature, so a release build compiles none of it and does
// not pull in `tempfile`.
//
// It is in the library rather than in `tests/` because a **game** repository
// needs it. A game layer is `mudlib/` plus `game/` and no Rust at all, and the
// only honest way to ask what its rules do — what a talent adds to a dice pool,
// which order two effects apply in, whether a daemon's state survives what the
// game does to it — is to boot a real `ScriptEngine` over those two roots and
// look. The alternative in use before this was driving a live server over
// telnet and matching on prose, which can establish that the game's messages
// are self-consistent and nothing else.
//
//   [dev-dependencies]
//   oxigeon = { path = "../oxigeon", features = ["testkit"] }
//
// then `RealVm::boot_roots_with_probe(&mudlib, &game, TestCtx::default())`.
//
// **This does not weaken the rule that no test in this repository may name
// `game/` or `mudlib/`.** Those roots are absent on a clean clone and this
// suite is still green without them. What is published is the *capability*; the
// caller supplies the paths.
#[cfg(feature = "testkit")]
pub mod testkit;
