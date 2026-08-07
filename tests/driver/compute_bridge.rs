//! The compute bridge, driven through the real engine.
//!
//! `src/core/compute/vm.rs` unit-tests a worker VM directly. These check the
//! parts only the whole assembly has: that the game thread genuinely keeps
//! serving, that every id gets exactly one answer, and that a value survives
//! the trip out to a worker and back.

use std::time::{Duration, Instant};

use crate::common::{ComputeReply, RealVm};
use oxigeon::config::server_config::ComputeConfig;

/// A compute root the probe mudlib can reach, written next to it.
const PROBE_MODULE: &str = r#"
local M = {}

function M.echo(args) return args end
function M.marker(args) return { marker = args and args.marker or "none" } end
function M.spin(args)
    local until_ms = args and args.ms or 1000
    local t0 = os.clock()
    while (os.clock() - t0) * 1000 < until_ms do end
    return { marker = "spun" }
end
function M.boom() error("kaboom") end
function M.reach()
    return { marker = tostring(send) .. "/" .. tostring(io) .. "/" .. tostring(set_object_state) }
end

return M
"#;

fn compute_on(cfg: ComputeConfig) -> RealVm {
    RealVm::boot_with_compute(cfg, |root| {
        let dir = root.join("compute");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.lua"), PROBE_MODULE).unwrap();
    })
}

fn enabled() -> ComputeConfig {
    ComputeConfig { enabled: true, ..Default::default() }
}

/// The property the whole facility exists for.
///
/// The assertion is on *ordering*, not just timing: every ordinary probe has
/// to complete before the compute result arrives. If the job ran on the game
/// thread they would queue behind it in the single command channel and could
/// not possibly finish first, whatever the machine's load. Timing alone would
/// be flaky; ordering is not.
#[test]
fn the_game_thread_keeps_serving_while_a_job_runs() {
    let mut vm = compute_on(enabled());

    let submitted = Instant::now();
    let id = vm
        .eval("_compute_session = this_session() return compute('compute.probe', 'spin', {ms = 700})")
        .unwrap();
    assert_ne!(id, "nil", "submitting should return an id");
    let dispatch_took = submitted.elapsed();

    assert!(
        dispatch_took < Duration::from_millis(200),
        "the dispatch that submitted the job took {dispatch_took:?} — it waited for the job"
    );

    for i in 0..5 {
        assert_eq!(vm.eval(&format!("return {i}")).unwrap(), i.to_string());
        assert!(
            !vm.compute_result_ready(),
            "the job finished before five trivial commands did — it cannot have been \
             running off-thread"
        );
    }

    let reply = vm.next_compute_result();
    assert_eq!(reply.kind, "ok", "{:?}", reply.error);
    assert_eq!(reply.value, "spun");
}

/// A value has to survive the trip out and back unchanged. This is the pair of
/// conversions `marshal` exists for, exercised through the real boundary.
#[test]
fn a_value_survives_the_round_trip() {
    let mut vm = compute_on(enabled());
    vm.eval("_compute_session = this_session() return 'ok'").unwrap();

    let same = vm
        .eval(
            "local sent = {'a', 'b', name = 'x', n = 3, deep = {f = 1.5, t = true}} \
             _sent = sent \
             return compute('compute.probe', 'echo', sent)",
        )
        .unwrap();
    assert_ne!(same, "nil");

    let reply = vm.next_compute_result();
    assert_eq!(reply.kind, "ok", "{:?}", reply.error);

    // Compared in Lua, so this checks the pair of conversions rather than one
    // leg of it. A table that is both a list and a map is deliberately used —
    // JSON could not carry it, which is why the bridge does not use JSON.
    let equal = vm
        .eval(
            "local function eq(a, b) \
               if type(a) ~= type(b) then return false end \
               if type(a) ~= 'table' then return a == b end \
               for k, v in pairs(a) do if not eq(v, b[k]) then return false end end \
               for k, v in pairs(b) do if not eq(v, a[k]) then return false end end \
               return true end \
             return tostring(eq(_sent, _last_compute_value))",
        )
        .unwrap();
    assert_eq!(equal, "true", "the value changed on the way there and back");
}

/// The compute analogue of `sandbox_reality_check`. A worker must not be able
/// to see the game — and the `set_object_state` case is the one that would
/// otherwise look like it worked, because a second VM gets its own empty copy.
#[test]
fn a_job_cannot_reach_the_game() {
    let mut vm = compute_on(enabled());
    vm.eval("_compute_session = this_session() set_object_state('room', 'k', 'v') return 'ok'")
        .unwrap();

    vm.eval("return compute('compute.probe', 'reach', nil)").unwrap();
    let reply = vm.next_compute_result();
    assert_eq!(reply.kind, "ok", "{:?}", reply.error);
    assert_eq!(
        reply.value, "nil/nil/nil",
        "a compute job reached something it should not have"
    );
}

/// A job that raises comes back as a failure with its message, not silence.
#[test]
fn a_job_that_raises_is_reported() {
    let mut vm = compute_on(enabled());
    vm.eval("_compute_session = this_session() return 'ok'").unwrap();

    vm.eval("return compute('compute.probe', 'boom', nil)").unwrap();
    let reply = vm.next_compute_result();
    assert_eq!(reply.kind, "error");
    assert!(reply.error.unwrap().contains("kaboom"));
}

/// Call-site mistakes come back as `nil, err` and fire no hook, so a caller
/// can tell "I never started" from "it started and failed".
#[test]
fn a_call_site_mistake_returns_nil_and_fires_no_hook() {
    let mut vm = compute_on(enabled());
    vm.eval("_compute_session = this_session() return 'ok'").unwrap();

    for (probe, expect) in [
        (
            "local id, err = compute('daemons.world_d', 'reset', nil) return tostring(id) .. '|' .. tostring(err)",
            "compute root",
        ),
        (
            "local id, err = compute('compute.probe', 'echo', {f = function() end}) return tostring(id) .. '|' .. tostring(err)",
            "cannot cross",
        ),
    ] {
        let out = vm.eval(probe).unwrap();
        assert!(out.starts_with("nil|"), "expected a refusal, got {out:?}");
        assert!(out.contains(expect), "unhelpful message: {out:?}");
    }

    // Give a hook the chance to fire, then confirm none did.
    for _ in 0..5 {
        vm.eval("return 1");
    }
    assert!(
        !vm.compute_result_ready(),
        "a call-site refusal must not also deliver a result"
    );
}

/// Every id gets exactly one answer, and the ids are distinct.
#[test]
fn every_submission_gets_exactly_one_result() {
    let mut vm = compute_on(enabled());
    vm.eval("_compute_session = this_session() return 'ok'").unwrap();

    let mut ids = Vec::new();
    for i in 0..5 {
        let id = vm
            .eval(&format!(
                "return compute('compute.probe', 'marker', {{marker = 'job{i}'}})"
            ))
            .unwrap();
        assert_ne!(id, "nil");
        ids.push(id);
    }

    let mut seen: Vec<ComputeReply> = Vec::new();
    for _ in 0..5 {
        seen.push(vm.next_compute_result());
    }

    let mut got: Vec<String> = seen.iter().map(|r| r.id.clone()).collect();
    got.sort();
    got.dedup();
    assert_eq!(got.len(), 5, "ids must be unique and each answered once");

    let mut markers: Vec<String> = seen.iter().map(|r| r.value.clone()).collect();
    markers.sort();
    assert_eq!(markers, vec!["job0", "job1", "job2", "job3", "job4"]);
}

/// The caller's tag comes back untouched, so a job can be correlated without
/// keeping an id-keyed side table.
#[test]
fn the_tag_is_echoed_back() {
    let mut vm = compute_on(enabled());
    vm.eval("_compute_session = this_session() return 'ok'").unwrap();

    vm.eval("return compute('compute.probe', 'marker', {marker='x'}, {tag = 'carry-me'})")
        .unwrap();
    let reply = vm.next_compute_result();
    assert_eq!(reply.kind, "ok", "{:?}", reply.error);
    assert_eq!(reply.tag.as_deref(), Some("carry-me"));
}

/// Compute is off unless asked for, and then the efun is absent rather than
/// present-and-useless — a missing global fails loudly at the call site.
#[test]
fn the_compute_efun_is_absent_when_the_feature_is_off() {
    let mut vm = RealVm::boot();
    assert!(!vm.reaches("compute"));
    assert!(!vm.reaches("compute_cancel"));
}

/// A full queue is answered, not dropped. The caller already has cleanup
/// written for a failed result; making it handle a separate "refused" return
/// path is how that cleanup gets forgotten.
#[test]
fn an_overloaded_pool_refuses_through_the_same_hook() {
    let mut vm = compute_on(ComputeConfig {
        enabled: true,
        workers: 1,
        queue_depth: 1,
        ..Default::default()
    });
    vm.eval("_compute_session = this_session() return 'ok'").unwrap();

    // Far more than one worker plus a one-deep queue can hold.
    for _ in 0..12 {
        vm.eval("return compute('compute.probe', 'spin', {ms = 120})").unwrap();
    }

    let mut kinds = Vec::new();
    for _ in 0..12 {
        kinds.push(vm.next_compute_result().kind);
    }
    assert!(
        kinds.iter().any(|k| k == "refused"),
        "expected some submissions to be refused, got {kinds:?}"
    );
    assert_eq!(kinds.len(), 12, "every submission must still be answered exactly once");
}
