//! The **driver** suite — everything that is the Rust engine's job.
//!
//! The rule that decides what lives here:
//!
//! > If you threw the Lua layer away and wrote your own from scratch, would you
//! > keep this test or rewrite it? **Keep it → here.** Rewrite it → nowhere,
//! > now: the suite that held those went with `mudlib.default/`.
//!
//! So this covers the Lua VM and its sandbox, the instruction budget, the efun
//! surface, the two-root file jail, the database layer, telnet/GMCP framing,
//! sessions, permission *enforcement*, the debugger and the compute workers.
//! Several of these boot a mudlib — but only as a **vehicle**, because the
//! assertion is about Rust behaviour underneath it.
//!
//! That vehicle is `tests/fixture/`, and it used to be `mudlib.default/` and
//! `game.example/` at the repository root. Nothing develops them, the one game
//! built on this engine forked the mudlib long ago, and the two suites that
//! asserted their *content* — `tests/mudlib/` and `tests/demo_world/` — are
//! gone. What survived is the only job they still do: booting.
//!
//! It follows that a failure here is **never** a complaint about the fixture's
//! content. If a test in this directory can be made to pass by editing Lua
//! under `tests/fixture/`, it was asking the wrong question and belongs
//! rewritten or deleted rather than accommodated.
//!
//! Never `game/` or `mudlib/`: those are the creator's own working copies —
//! gitignored, absent on a clean clone — and nothing in this suite loads them.

#[path = "../common/mod.rs"]
mod common;

mod account_store;
mod auth_off_thread;
mod character_store;
mod clean_shutdown;
mod command_dispatch;
mod compute_bridge;
mod dap_attach;
mod debug_hook_spike;
mod debug_parked_gc;
mod debug_paths;
mod debug_ret_spike;
mod debug_trace;
mod document_efuns;
mod document_store;
mod file_jail_two_roots;
mod game_logger;
mod hot_reload;
mod instruction_limit;
mod json_bridge;
mod list_dir_jail;
mod observability;
mod output_backpressure;
mod permission_config;
mod permission_refresh;
mod permissions;
mod sandbox;
mod sandbox_reality_check;
mod staff;
mod telnet_mxp;
mod telnet_tls;
mod timer_identity;
mod websocket_relay;
mod yield_pause;
