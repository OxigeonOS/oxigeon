//! What happens when a compute job never comes back.
//!
//! **This binary deliberately leaks a thread spinning at 100% for the rest of
//! the process.** Rust cannot kill a thread, so a job that ignores its deadline
//! runs until the process exits — that is the documented cost of keeping the
//! compiler on, and the point of these tests is to prove it costs *one worker*
//! rather than the game.
//!
//! That is also why this is a separate test binary rather than more cases in
//! `compute_bridge.rs`. Cargo runs the tests within one binary concurrently, so
//! a burning core in there would skew every timing assertion its neighbours
//! make. Its own binary means the thread dies when this binary exits. Please
//! do not tidy it back in.

mod common;

use std::time::{Duration, Instant};

use common::RealVm;
use oxigeon::config::server_config::ComputeConfig;

const RUNAWAY: &str = r#"
local M = {}
function M.hog() while true do end end
function M.ok() return { marker = "fine" } end
return M
"#;

fn vm_with(cfg: ComputeConfig) -> RealVm {
    let mut vm = RealVm::boot_with_compute(cfg, |root| {
        let dir = root.join("compute");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("runaway.lua"), RUNAWAY).unwrap();
    });
    vm.eval("_compute_session = this_session() return 'ok'").unwrap();
    vm
}

/// The isolation claim, stated as plainly as it can be tested: an
/// uninterruptible job costs its worker and nothing else.
///
/// The compiler is on here (`instruction_limit = 0`), so nothing can stop the
/// job — no hook fires inside a compiled trace. The caller still has to be
/// told, the game still has to answer, and a later submission still has to be
/// refused rather than blocking forever.
#[test]
fn a_wedged_job_costs_one_worker_and_not_the_game() {
    let mut vm = vm_with(ComputeConfig {
        enabled: true,
        workers: 1,
        queue_depth: 1,
        default_deadline_ms: 400,
        instruction_limit: 0, // compiler on: the job is genuinely uninterruptible
        ..Default::default()
    });

    vm.eval("return compute('compute.runaway', 'hog', nil)").unwrap();

    let started = Instant::now();
    let reply = vm.next_compute_result();
    assert_eq!(reply.kind, "timeout", "{:?}", reply.error);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the caller waited {:?} to be told about a deadline",
        started.elapsed()
    );

    // The game thread is untouched — this is the whole point.
    for i in 0..10 {
        assert_eq!(vm.eval(&format!("return {i}")).unwrap(), i.to_string());
    }

    // And the loss is visible rather than mysterious.
    let wedged = vm.eval("return tostring(server_info().compute.wedged)").unwrap();
    assert_eq!(wedged, "1", "a wedged worker must be counted, not just lost");
}

/// With a budget armed the compiler is off, the hook fires, and the worker
/// comes back. This is what `compute.instruction_limit` actually buys, and
/// without this test the recommendation to consider it would be an assertion
/// rather than a measurement.
#[test]
fn a_budget_makes_a_runaway_job_recoverable() {
    let mut vm = vm_with(ComputeConfig {
        enabled: true,
        workers: 1,
        queue_depth: 4,
        instruction_limit: 2_000_000,
        ..Default::default()
    });

    vm.eval("return compute('compute.runaway', 'hog', nil)").unwrap();
    let reply = vm.next_compute_result();
    assert_eq!(reply.kind, "budget", "{:?}", reply.error);

    // The worker survived and takes the next job — the difference between a
    // recoverable pool and a permanently degraded one.
    vm.eval("return compute('compute.runaway', 'ok', nil)").unwrap();
    let next = vm.next_compute_result();
    assert_eq!(next.kind, "ok", "{:?}", next.error);
    assert_eq!(next.value, "fine");

    assert_eq!(
        vm.eval("return tostring(server_info().compute.wedged)").unwrap(),
        "0",
        "a job stopped by its budget is not a wedged worker"
    );
}
