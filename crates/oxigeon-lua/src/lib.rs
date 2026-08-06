//! The parts of Oxigeon that touch a Lua VM and are needed by *both* the server
//! and the compute worker.
//!
//! This crate exists because of one hard constraint: `mlua-sys` permits exactly
//! one Lua version per build, and LuaJIT and PUC Lua export the same C symbols,
//! so a single binary cannot link both. The game thread may want Lua 5.5 — for a
//! debug hook that can yield — while a compute job wants LuaJIT's compiler,
//! which is worth 2.10× on the arithmetic-heavy work compute exists for. The
//! only way to have both is two processes, and the only way to have two
//! processes without two copies of this code is a crate they can each build with
//! their own runtime.
//!
//! So everything here is compiled twice, with different features, in different
//! cargo invocations. Nothing in it may depend on the server: no config types,
//! no error enum, no efuns. What crosses between the two processes is
//! [`marshal::LuaData`], framed by [`wire`].

pub mod lua_path;
pub mod marshal;
pub mod sandbox;
pub mod settings;
pub mod vm;
pub mod wire;

pub use marshal::{Key, Limits, LuaData, MarshalError, Table};
pub use settings::ComputeSettings;
pub use vm::{Ending, Outcome};
