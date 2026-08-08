//! The **driver** suite — everything that is the Rust engine's job.
//!
//! The rule that decides what lives here:
//!
//! > If you deleted `mudlib.default/` and wrote your own from scratch, would
//! > you keep this test or rewrite it? **Keep it → here.** Rewrite it →
//! > `tests/mudlib/`.
//!
//! So this covers the Lua VM and its sandbox, the instruction budget, the efun
//! surface, the two-root file jail, the database layer, telnet/GMCP framing,
//! sessions, permission *enforcement*, the debugger and the compute workers.
//! Several of these boot the real mudlib — but only as a vehicle, because the
//! assertion is about Rust behaviour underneath it.
//!
//! This directory must stay green with the mudlib deleted:
//!
//! ```bash
//! mkdir ../away
//! mv game.example tests/demo_world mudlib.default tests/mudlib ../away/
//! cargo test --test driver
//! ```
//!
//! Note the *shipped* trees by name. `game/` and `mudlib/` are the creator's
//! own working copies — gitignored, absent on a clean clone — and nothing in
//! the suite loads them; moving those aside would prove nothing.

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
mod timer_identity;
mod yield_pause;
