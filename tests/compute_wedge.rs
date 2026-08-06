//! What happens when a compute job never comes back.
//!
//! This used to be the ugliest corner of the facility. Workers were threads,
//! Rust cannot kill a thread, and with the compiler on there is no hook to
//! interrupt a runaway job — so one burned a core until the *server* exited,
//! and all the pool could do was count it. The file even warned that it leaked a
//! spinning thread for the life of the test binary.
//!
//! Workers are child processes now, so the deadline can actually end the job:
//! the watchdog kills that worker and the next submission gets a fresh one.
//! These tests pin both halves — the caller is still told promptly, and the pool
//! genuinely recovers.
//!
//! Still its own test binary. Cargo runs the tests within one binary
//! concurrently, and a job spinning a core for its whole deadline would skew any
//! timing assertion a neighbour made.

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

/// The isolation claim, stated as plainly as it can be tested: a job that
/// cannot be interrupted costs its worker and nothing else.
///
/// The compiler is on here (`instruction_limit = 0`), so nothing *inside* the
/// VM can stop the job — no hook fires from a compiled trace. The caller still
/// has to be told, and the game still has to answer.
#[test]
fn a_wedged_job_costs_one_worker_and_not_the_game() {
    let mut vm = vm_with(ComputeConfig {
        enabled: true,
        workers: 1,
        queue_depth: 1,
        default_deadline_ms: 1_500,
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
    assert_eq!(wedged, "1", "a job killed at its deadline must be counted");
}

/// The pool comes back, with the compiler still on.
///
/// This is what moving workers out of process bought. The same runaway job, the
/// same `instruction_limit = 0`, one worker — and the very next submission is
/// answered, because the deadline killed a *process* rather than stranding a
/// thread. Under the thread pool this test could not have passed: the single
/// worker was gone for the life of the server and every later job queued behind
/// it for ever.
#[test]
fn the_pool_recovers_from_a_job_that_had_to_be_killed() {
    let mut vm = vm_with(ComputeConfig {
        enabled: true,
        workers: 1,
        queue_depth: 4,
        default_deadline_ms: 1_500,
        instruction_limit: 0, // compiler on: only a kill can end this job
        ..Default::default()
    });

    vm.eval("return compute('compute.runaway', 'hog', nil)").unwrap();
    assert_eq!(vm.next_compute_result().kind, "timeout");

    vm.eval("return compute('compute.runaway', 'ok', nil)").unwrap();
    let next = vm.next_compute_result();
    assert_eq!(
        next.kind, "ok",
        "the only worker never came back after a killed job: {:?}",
        next.error
    );
    assert_eq!(next.value, "fine");

    // It really was killed mid-run, rather than expiring in the queue — which
    // is the difference between this test and one that proves nothing.
    assert_eq!(
        vm.eval("return tostring(server_info().compute.wedged)").unwrap(),
        "1"
    );
    let spawned: u32 = vm
        .eval("return tostring(server_info().compute.spawned)")
        .unwrap()
        .parse()
        .unwrap();
    assert!(spawned >= 2, "the killed worker was never replaced (spawned = {spawned})");
}

/// An enabled pool that nobody uses spawns nothing.
///
/// Workers are processes now, so "compute is enabled" must not mean "there are
/// idle child processes on this box". They are started on first use, which is
/// also what keeps the cost of turning the feature on at zero until a job runs.
#[test]
fn an_idle_pool_has_started_no_worker_processes() {
    let mut vm = vm_with(ComputeConfig { enabled: true, workers: 2, ..Default::default() });

    assert_eq!(
        vm.eval("return tostring(server_info().compute.spawned)").unwrap(),
        "0",
        "a worker process was started before any job was submitted"
    );

    vm.eval("return compute('compute.runaway', 'ok', nil)").unwrap();
    assert_eq!(vm.next_compute_result().kind, "ok");

    assert_eq!(
        vm.eval("return tostring(server_info().compute.spawned)").unwrap(),
        "1",
        "one job should start exactly one worker, not the whole pool"
    );
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
