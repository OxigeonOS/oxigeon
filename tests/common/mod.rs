//! The test harness, which now lives in the crate.
//!
//! It moved to `oxigeon::testkit`, behind a non-default `testkit` feature, so
//! that a **game** repository can boot its own `mudlib/` and `game/` inside a
//! real `ScriptEngine` and ask what its content does. That is not a
//! hypothetical: a game layer's rules — dice pools, effect ordering, what a
//! talent adds to a check — are exactly the kind of thing this harness was
//! built to see, and until it was publishable the only way to test them was to
//! drive a live server over telnet and read the prose back.
//!
//! **This does not weaken the rule that no test here may name `game/` or
//! `mudlib/`.** Oxigeon's own suite still names neither and is still green with
//! both absent. The testkit is a *capability*; the caller supplies the roots.
//!
//! This file stays so the four test binaries keep saying `mod common;` and the
//! ~1300 tests behind them do not move.

#![allow(unused_imports)]

pub use oxigeon::testkit::*;
