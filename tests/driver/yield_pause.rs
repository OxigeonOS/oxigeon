//! What a breakpoint costs the rest of the server.
//!
//! This is the whole reason the Lua 5.5 option exists. mlua's `VmState::Yield`
//! is "Lua 5.3+ and Luau", so under LuaJIT a stop has nowhere to suspend to and
//! is implemented by *blocking the Lua thread* — which is the only thread, so
//! every player stops, every timer queues, and nothing regenerates until the
//! client resumes. On 5.5 the hook yields, the engine parks that one coroutine
//! and goes back to its loop, and everyone else carries on.
//!
//! One test, split at the assertion, because the setup is identical and the
//! difference is the point. Neither branch needs a TCP client: the breakpoint
//! table and the request channel *are* what a DAP `setBreakpoints` and
//! `continue` write to, so a test can make the same requests the adapter makes
//! with none of the machinery.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use oxigeon::config::DebugServerConfig;
use oxigeon::core::scripting::debugger::paths;
use oxigeon::core::scripting::debugger::state::{
    BreakpointSpec, ResumeKind, StopId, VmRequest, WORLD_STOP,
};
use oxigeon::core::scripting::debugger::DebugState;

use crate::common::{RealVm, TestCtx};

/// How long a command sent during the stop is given to answer.
///
/// Generous for a dispatch that does almost no work, and the LuaJIT branch
/// asserts nothing arrives in it — so it also has to be long enough that
/// "nothing arrived" means frozen rather than merely slow.
const WINDOW: Duration = Duration::from_millis(1500);

/// Long enough that the auto-continue valve never fires during the test: both
/// branches resume explicitly, and a valve that fired first would make the
/// LuaJIT branch pass for the wrong reason.
const AUTO_CONTINUE_SECS: u64 = 30;

/// The one-shot timer is scheduled before the stop and set to fire after it, so
/// its flag is true only if the driver's timer path kept running *while a
/// dispatch was suspended*. Not a statistical claim about tick counts.
const LATE_TIMER_SECS: f64 = 1.0;

/// Where to stop: the body of `ticker_d.M.list`, reached by the first dispatch
/// and by nothing else in this test.
///
/// A breakpoint rather than a `pause` request, because `pause` stops at the next
/// line event *anywhere* — including the tail of the dispatch before it, which
/// runs on after its reply has been sent. That race made this test stop the
/// wrong dispatch roughly one run in seven.
/// `mudlib.default/` rather than `mudlib/`: the harness boots the mudlib this
/// repository ships, not the creator's own working copy. A breakpoint path that
/// names the wrong root simply never binds, and the test hangs to its deadline
/// rather than failing at the line that is wrong.
const BREAK_FILE: &str = "mudlib.default/daemons/ticker_d.lua";
const BREAK_LINE: u32 = 181;

/// A debug state with the freeze policy set explicitly.
///
/// `stop_the_world` defaults to true — a debugger that freezes is what everyone
/// expects — so every test of the *suspending* behaviour has to opt out, and
/// says so at the top rather than inheriting it.
fn debug_state(freeze: bool) -> oxigeon::core::scripting::debugger::SharedDebugState {
    DebugState::from_config(
        &DebugServerConfig {
            enabled: true,
            auto_continue_secs: AUTO_CONTINUE_SECS,
            stop_the_world: freeze,
            ..Default::default()
        },
        0,
    )
}

/// Whether anything is stopped — the world, or any one dispatch.
///
/// `DebugState::stopped` means "the world is frozen" and nothing else, so a test
/// that waited on it would wait for ever on the suspending path.
fn anything_stopped(dbg: &oxigeon::core::scripting::debugger::SharedDebugState) -> bool {
    dbg.stopped.load(Ordering::Acquire) || dbg.parked_count.load(Ordering::Acquire) > 0
}

/// Block until something stops, or fail saying it never did.
fn wait_for_stop(dbg: &oxigeon::core::scripting::debugger::SharedDebugState, what: &str) {
    let t0 = Instant::now();
    while !anything_stopped(dbg) {
        assert!(t0.elapsed() < Duration::from_secs(5), "{what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Block until nothing is stopped.
fn wait_for_running(dbg: &oxigeon::core::scripting::debugger::SharedDebugState, what: &str) {
    let t0 = Instant::now();
    while anything_stopped(dbg) {
        assert!(t0.elapsed() < Duration::from_secs(5), "{what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The 1-based line holding `needle`, or fail naming what was not found.
///
/// A breakpoint test pins a *statement*; the number it happens to sit on is a
/// property of everything written above it, and editing an unrelated function
/// in the same file must not turn into "the debugger cannot stop".
fn line_of(file: &str, needle: &str) -> u32 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let mut hits = src
        .lines()
        .enumerate()
        .filter(|(_, l)| l.trim() == needle)
        .map(|(i, _)| i as u32 + 1);

    let line = hits.next().unwrap_or_else(|| panic!("no line {needle:?} in {file}"));
    assert!(hits.next().is_none(), "{needle:?} is not unique in {file}");
    line
}

/// Set one breakpoint and attach, the way a client's `setBreakpoints` does.
fn set_breakpoint(
    dbg: &oxigeon::core::scripting::debugger::SharedDebugState,
    file: &str,
    line: u32,
    spec: BreakpointSpec,
) {
    let key = paths::normalize(&paths::abs_lua_path(&std::path::PathBuf::from(file)));
    dbg.breakpoints.lock().unwrap().by_file.entry(key).or_default().insert(line, spec);
    dbg.bp_count.store(1, Ordering::Relaxed);
    dbg.clients.store(1, Ordering::Relaxed);
    dbg.republish();
}

#[test]
fn a_stop_holds_one_dispatch_and_on_the_yielding_path_holds_only_that_one() {
    // The whole subject of this test, so it is turned off explicitly.
    let dbg = debug_state(false);
    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    // A wounded entity whose regeneration anchor is far enough back to have
    // earned points, and a timer that fires once, after the stop has started.
    vm.eval(
        "_e = { char_id = 9401, stats = {} } \
         DAEMON.trait.seed(_e, 'character') \
         DAEMON.trait.set_cur(_e, 'hp', 5) \
         _e.stats._at.hp = _e.stats._at.hp - 60 \
         _late = false \
         DAEMON.ticker.after(1, 'yp_late', function() _late = true end) return 'ok'",
    )
    .unwrap();

    // The same two things a DAP client's attach and `setBreakpoints` do.
    set_breakpoint(&dbg, BREAK_FILE, BREAK_LINE, BreakpointSpec::default());
    // A quiet dispatch, so `sync_hook` widens the trigger mask — which only
    // happens between commands — before anything is expected to stop.
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    vm.send_eval("DAEMON.ticker.list() return 'first'");

    wait_for_stop(&dbg, "the breakpoint never stopped anything");

    // Let the one-shot come due while the first dispatch is suspended.
    std::thread::sleep(Duration::from_secs_f64(LATE_TIMER_SECS + 0.25));

    // A second player's command: settle a gauge (which runs the `regen_rate`
    // pipeline) and report both the regenerated value and whether the timer
    // got through.
    vm.send_eval(
        "DAEMON.trait.touch(_e) \
         return tostring(DAEMON.trait.value(_e, 'hp')) .. '/' .. tostring(_late)",
    );
    let during = vm.probe_within(WINDOW);

    #[cfg(not(feature = "luajit"))]
    {
        let reply = during
            .expect("the second dispatch never answered while one was suspended")
            .unwrap();
        let (hp, late) = reply
            .split_once('/')
            .unwrap_or_else(|| panic!("expected `hp/late`, got {reply:?}"));
        assert_ne!(
            hp, "5",
            "the second entity did not regenerate while another dispatch was \
             suspended — traits are still stopping the world ({reply})"
        );
        assert_eq!(
            late, "true",
            "the timer scheduled before the stop had not fired by the time the \
             second dispatch ran — the driver's timer path is still frozen ({reply})"
        );

        // And the suspended one resumes correctly afterwards.
        resume(&dbg);
        assert_eq!(
            vm.probe_within(Duration::from_secs(5))
                .expect("the suspended dispatch never completed after the resume")
                .unwrap(),
            "first"
        );
    }

    #[cfg(feature = "luajit")]
    {
        assert!(
            during.is_none(),
            "a command answered while the VM was stopped — LuaJIT has no yield, \
             so a stop must block the Lua thread (got {during:?})"
        );

        // Frozen, not broken: the resume releases the thread and both dispatches
        // come back, the suspended one first because it was already in flight.
        resume(&dbg);
        assert_eq!(
            vm.probe_within(Duration::from_secs(5))
                .expect("the suspended dispatch never completed after the resume")
                .unwrap(),
            "first"
        );
        let reply = vm
            .probe_within(Duration::from_secs(5))
            .expect("the queued second dispatch never ran after the resume")
            .unwrap();
        // Everything the yielding path got *during* the stop, this path gets
        // only after it — which is the entire difference.
        assert!(reply.ends_with("/true"), "{reply}");
    }
}

/// A breakpoint in a tick suspends that tick, not the server.
///
/// Ticks were a direct call at first, on the reasoning that a stop in one would
/// be rare. It is the opposite: combat rounds, regeneration, effect ticks and
/// every daemon heartbeat arrive as `on_timer`, so a tick is the *likeliest*
/// place to want a breakpoint — and breaking in combat froze every player on the
/// server, which is exactly what the yielding runtime is meant to prevent.
///
/// Split at the assertion like the test above, because LuaJIT still has nowhere
/// to suspend to. The remaining direct calls — connects, GMCP, hot reloads —
/// block on both, and so does anything behind a C frame; see the test below.
#[test]
fn a_stop_in_a_tick_suspends_only_that_tick() {
    let dbg = debug_state(false);
    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    vm.eval(
        "_tm = 'no'          DAEMON.ticker.after(0.1, 'yp_tm', function() DAEMON.ticker.list() _tm = 'ran' end)          return 'ok'",
    )
    .unwrap();

    set_breakpoint(&dbg, BREAK_FILE, BREAK_LINE, BreakpointSpec::default());
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    wait_for_stop(&dbg, "the breakpoint in the tick never stopped anything");

    // A player's command, sent while the tick is stopped.
    vm.send_eval("return 'alive'");
    let during = vm.probe_within(WINDOW);

    #[cfg(not(feature = "luajit"))]
    {
        // Suspended, not blocked: the engine is holding a coroutine for this
        // tick, and everyone else is still being served.
        assert!(
            !dbg.parked.lock().unwrap().is_empty(),
            "a breakpoint in a tick blocked the world instead of parking the tick"
        );
        assert_eq!(
            during.map(|p| p.unwrap()).as_deref(),
            Some("alive"),
            "no command was answered while a tick was suspended — a breakpoint \
             in combat still freezes every player"
        );
    }

    #[cfg(feature = "luajit")]
    assert!(
        during.is_none(),
        "a command answered while a tick was stopped — LuaJIT has no yield, so \
         this must block (got {during:?})"
    );

    resume(&dbg);
    wait_for_running(&dbg, "the VM still reports itself stopped after the resume");

    // On the blocking path the `alive` probe was still in flight; collect it
    // now, or the next `eval` would be handed that reply instead of its own.
    #[cfg(feature = "luajit")]
    assert_eq!(
        vm.probe_within(Duration::from_secs(5)).map(|p| p.unwrap()).as_deref(),
        Some("alive"),
        "the command queued behind the stopped tick never ran"
    );

    // Clear the breakpoint before asking, or the probe's own call would stop
    // again — `DAEMON.ticker` is on the mudlib's normal paths.
    dbg.bp_count.store(0, Ordering::Relaxed);
    dbg.breakpoints.lock().unwrap().by_file.clear();
    dbg.republish();

    assert_eq!(
        vm.eval("return _tm").unwrap(),
        "ran",
        "the tick never finished after the resume"
    );
}

/// A stop behind a C frame blocks too, even on a coroutine.
///
/// This is why the hook asks `lua_isyieldable` rather than "am I on a
/// coroutine". A `table.sort` comparator — like a `gsub` replacement function or
/// an `__index` metamethod — is called *by C*, from a player command running on
/// the dispatch coroutine, and cannot yield past that frame. Answering the
/// easier question here parks a coroutine that Lua never actually suspended,
/// and the debugger believes the VM is stopped from then on.
#[test]
fn a_stop_behind_a_c_frame_blocks_even_on_a_coroutine() {
    // Asks to suspend; the C frame is what makes it block anyway.
    let dbg = debug_state(false);
    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    set_breakpoint(&dbg, BREAK_FILE, BREAK_LINE, BreakpointSpec::default());
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    // `table.sort` is the C frame; the comparator is where the breakpoint is.
    vm.send_eval(
        "local t = { 3, 1, 2 } \
         table.sort(t, function(a, b) DAEMON.ticker.list() return a < b end) \
         return 'sorted'",
    );

    wait_for_stop(&dbg, "the breakpoint in the comparator never stopped anything");

    assert!(
        dbg.parked.lock().unwrap().is_empty(),
        "a stop behind a C frame was parked — Lua cannot have suspended it, so \
         nothing will ever resume it"
    );

    // A comparator runs several times, so keep continuing until it is through.
    let deadline = Instant::now() + Duration::from_secs(10);
    let reply = loop {
        if anything_stopped(&dbg) {
            resume(&dbg);
        }
        if let Some(p) = vm.probe_within(Duration::from_millis(100)) {
            break p.unwrap();
        }
        assert!(deadline > Instant::now(), "the sort never finished");
    };
    assert_eq!(reply, "sorted");
}

/// Ask the VM to continue, the way the adapter's `continue` request does.
///
/// Resumes the newest suspended dispatch, or — when nothing is parked, which is
/// every stop on the freezing path — the world stop.
fn resume(dbg: &oxigeon::core::scripting::debugger::SharedDebugState) {
    let stop = dbg
        .parked_list()
        .last()
        .map(|(id, _, _)| *id)
        .unwrap_or(WORLD_STOP);
    resume_stop(dbg, stop);
}

/// Resume one named stop.
fn resume_stop(dbg: &oxigeon::core::scripting::debugger::SharedDebugState, stop: StopId) {
    let tx = dbg
        .vm_tx
        .lock()
        .unwrap()
        .clone()
        .expect("the request channel is gone");
    tx.send(VmRequest::Resume { stop, kind: ResumeKind::Continue })
        .expect("nothing is listening for debug requests");
}

/// A resumed dispatch must find its frame exactly as it left it.
///
/// Parking a coroutine means the hook yields out of the middle of a Lua call and
/// something else runs before it is resumed. If that lost the frame's slots — a
/// parameter in particular — the symptom would be an error blaming a local that
/// obviously is not nil, at a line long after it was last touched, and only ever
/// after a resume. This is the cheapest possible statement of the property:
/// stop inside a function *with arguments*, resume, and check the value it
/// computes from them.
///
/// `strings.M.number` is the subject because it is small, pure, takes a
/// parameter, and reads it on both sides of the stop.
#[test]
fn a_resumed_dispatch_still_has_its_arguments() {
    let dbg = debug_state(false);
    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    // `if n % 1 == 0 then` in `strings.number`: past the guards, before the
    // return, with `n` read on both sides.
    //
    // Looked up rather than written down. This was line 114 until a function
    // was added above it, and then the breakpoint armed on a line inside a
    // different function that the probe never reaches — so the test failed
    // saying the debugger could not stop, which is not what had happened. What
    // it is really pinning is a *statement*, so it asks for one.
    let line = line_of("mudlib.default/lib/strings.lua", "if n % 1 == 0 then");
    set_breakpoint(&dbg, "mudlib.default/lib/strings.lua", line, BreakpointSpec::default());
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    vm.send_eval("return require('lib.strings').number(1234)");

    wait_for_stop(&dbg, "the breakpoint never stopped anything");

    // Let the world turn while it is suspended, so the resume is not simply the
    // next thing that happens.
    std::thread::sleep(Duration::from_millis(250));

    // Keep continuing: the same line is reached again on later calls.
    let deadline = Instant::now() + Duration::from_secs(10);
    let reply = loop {
        if anything_stopped(&dbg) {
            resume(&dbg);
        }
        if let Some(p) = vm.probe_within(Duration::from_millis(100)) {
            break p.unwrap();
        }
        assert!(deadline > Instant::now(), "the suspended dispatch never finished");
    };

    assert_eq!(
        reply, "1234",
        "the argument was not intact after the resume — a parked frame is losing \
         its slots, which would surface as `attempt to index a nil value (local \
         'self')` in any method that stops"
    );
}

/// `stop_the_world` on 5.5 behaves exactly as LuaJIT does.
///
/// The flag has to be real rather than decorative, and the only way to say that
/// precisely is to assert the *other* runtime's behaviour on this one: nothing
/// parks, `stopped` is true, and no other command is answered until the resume.
/// Runs on both, because on LuaJIT it is simply what already happens.
#[test]
fn freezing_holds_the_whole_vm_on_either_runtime() {
    let dbg = debug_state(true);
    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    set_breakpoint(&dbg, BREAK_FILE, BREAK_LINE, BreakpointSpec::default());
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    vm.send_eval("DAEMON.ticker.list() return 'first'");
    wait_for_stop(&dbg, "the breakpoint never stopped anything");

    assert!(
        dbg.stopped.load(Ordering::Acquire),
        "with stop_the_world on, a stop must freeze the VM"
    );
    assert_eq!(
        dbg.parked_count.load(Ordering::Acquire),
        0,
        "nothing may be parked when the world is frozen — the stop is the VM itself"
    );

    vm.send_eval("return 'alive'");
    assert!(
        vm.probe_within(WINDOW).is_none(),
        "a command was answered while the world was supposed to be frozen"
    );

    resume(&dbg);
    assert_eq!(
        vm.probe_within(Duration::from_secs(5))
            .expect("the frozen dispatch never completed")
            .unwrap(),
        "first"
    );
    assert_eq!(
        vm.probe_within(Duration::from_secs(5))
            .expect("the queued command never ran after the resume")
            .unwrap(),
        "alive"
    );
}

/// Two dispatches stopped at once are separately inspectable, and resuming one
/// leaves the other where it was.
///
/// This is the "kept resetting itself" report. A breakpoint on a line a ticker
/// reaches every round parks a *new* dispatch every round; with one slot for the
/// captured frames, each stop silently replaced the last — the older one's
/// capture leaked and every question about it came back empty.
#[cfg(not(feature = "luajit"))]
#[test]
fn two_stops_at_once_are_both_inspectable() {
    let dbg = debug_state(false);
    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    set_breakpoint(&dbg, BREAK_FILE, BREAK_LINE, BreakpointSpec::default());
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    // Two commands that both reach the breakpoint. The second is sent only once
    // the first has stopped, so their order is not a race.
    vm.send_eval("DAEMON.ticker.list() return 'one'");
    wait_for_stop(&dbg, "the first dispatch never stopped");
    let first = dbg.parked_list();
    assert_eq!(first.len(), 1, "expected one stop, got {first:?}");

    vm.send_eval("DAEMON.ticker.list() return 'two'");
    let t0 = Instant::now();
    while dbg.parked_count.load(Ordering::Acquire) < 2 {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "the second dispatch never stopped — the first is still holding the \
             only slot"
        );
        std::thread::sleep(Duration::from_millis(2));
    }

    let both = dbg.parked_list();
    assert_eq!(both.len(), 2, "{both:?}");
    let (older, newer) = (both[0].0, both[1].0);
    assert_ne!(older, newer, "two stops must have distinct ids");

    // Both still describe a real frame. Under the single slot the older one's
    // frames were gone entirely.
    for (id, _, _) in &both {
        assert!(
            dbg.parked_stop(*id).is_some_and(|p| !p.frames.is_empty()),
            "stop {id} has no frames — it was overwritten rather than kept"
        );
    }

    // Resuming the newer one must not disturb the older.
    resume_stop(&dbg, newer);
    assert_eq!(
        vm.probe_within(Duration::from_secs(5))
            .expect("the resumed dispatch never completed")
            .unwrap(),
        "two"
    );
    assert_eq!(
        dbg.parked_count.load(Ordering::Acquire),
        1,
        "resuming one stop resumed the other too"
    );
    assert!(
        dbg.parked_stop(older).is_some(),
        "the stop that was not resumed disappeared"
    );

    resume_stop(&dbg, older);
    assert_eq!(
        vm.probe_within(Duration::from_secs(5))
            .expect("the second suspended dispatch never completed")
            .unwrap(),
        "one"
    );
    wait_for_running(&dbg, "something is still parked after both resumes");
}

/// A logpoint reports and keeps running.
///
/// The answer to "a breakpoint on a line combat reaches every round is a stop
/// every round": make it a logpoint and it is a running commentary instead.
/// Nothing stops, so it works the same on both runtimes.
#[test]
fn a_logpoint_reports_without_stopping() {
    let dbg = debug_state(true);
    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    // Console output is delivered to a DAP client, which these tests do not
    // stand up — so watch the VM's behaviour instead: it must not stop, and the
    // command must complete normally.
    set_breakpoint(
        &dbg,
        BREAK_FILE,
        BREAK_LINE,
        BreakpointSpec {
            log_message: Some("listing {#ids} timers".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    // Would stop three times if this were an ordinary breakpoint.
    for _ in 0..3 {
        assert_eq!(
            vm.eval("DAEMON.ticker.list() return 'ran'").unwrap(),
            "ran",
            "a logpoint stopped the dispatch instead of reporting"
        );
    }
    assert!(!dbg.stopped.load(Ordering::Acquire), "a logpoint froze the VM");
    assert_eq!(
        dbg.parked_count.load(Ordering::Acquire),
        0,
        "a logpoint suspended a dispatch"
    );
}

/// A logpoint's condition still gates it, and a failing `{expr}` does not stop
/// the line from being reported.
#[test]
fn a_logpoint_that_cannot_evaluate_still_reports() {
    let dbg = debug_state(true);
    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    set_breakpoint(
        &dbg,
        BREAK_FILE,
        BREAK_LINE,
        BreakpointSpec {
            condition: Some("true".to_string()),
            log_message: Some("{no_such_local} and {#ids}".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    assert_eq!(
        vm.eval("DAEMON.ticker.list() return 'ran'").unwrap(),
        "ran",
        "an unresolvable {{expr}} must not stop the dispatch — a logpoint that \
         gave up the moment one field went nil would be useless on the line it \
         is watching"
    );
    assert!(!dbg.stopped.load(Ordering::Acquire));
}

/// A logpoint's `{expr}` must see the locals of the line it is on.
///
/// The report: a logpoint of `alive={player.is_alive()}` on a line where
/// `player` is plainly a local came back `player.is_alive()=<eval:1: attempt to
/// index a nil value (global 'player')>` — the evaluator resolved against no
/// frame at all, so every name fell through to globals.
#[test]
fn a_logpoint_sees_the_locals_of_the_line_it_is_on() {
    let dbg = debug_state(true);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    *dbg.evt_tx.lock().unwrap() = Some(tx);

    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    // `ticker_d.lua:185` is `return ids`, with `ids` a local built just above.
    set_breakpoint(
        &dbg,
        BREAK_FILE,
        185,
        BreakpointSpec {
            log_message: Some("count={#ids} type={type(ids)}".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");

    vm.eval("DAEMON.ticker.after(60, 'lp_a', function() end) return 'ok'").unwrap();
    assert_eq!(vm.eval("DAEMON.ticker.list() return 'ran'").unwrap(), "ran");

    let mut lines = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let oxigeon::core::scripting::debugger::state::DebugEventMsg::Output { text, .. } = msg {
            lines.push(text);
        }
    }
    let joined = lines.join(" | ");
    assert!(!joined.is_empty(), "the logpoint emitted nothing at all");
    // `describe` renders a string result quoted, as the variables pane does.
    assert!(
        joined.contains("type=\"table\""),
        "`ids` was not in scope for the logpoint: {joined}"
    );
    // That `#ids` evaluated at all, not what it came to. The count is however
    // many tickers the mudlib happens to register, so pinning it made this
    // logpoint test fail whenever a daemon grew a heartbeat — which says
    // nothing about whether the evaluator could see the frame.
    let count = joined
        .split("count=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse::<usize>().ok());
    assert!(
        count.is_some_and(|n| n >= 2),
        "the length of the local list should have been evaluated: {joined}"
    );
    assert!(
        !joined.contains("nil value"),
        "the evaluator could not see the frame: {joined}"
    );
}

/// Calling a method with `.` instead of `:` is reported where it can be acted on.
///
/// `player.is_alive()` passes no `self`, and Lua reports it from inside the
/// callee — "mobile.lua:114: attempt to index a nil value (local 'self')", a
/// file and a line that have nothing to do with what was typed. In a debug
/// console, where expressions are typed constantly, that is a long way to walk
/// for a missing colon.
#[test]
fn a_dot_call_on_a_method_says_so() {
    let dbg = debug_state(true);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    *dbg.evt_tx.lock().unwrap() = Some(tx);

    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    // A method declared the usual way, so calling it with `.` leaves `self` nil.
    vm.eval("_obj = { hp = 5 } function _obj:alive() return self.hp > 0 end return 'ok'")
        .unwrap();

    set_breakpoint(
        &dbg,
        BREAK_FILE,
        185,
        BreakpointSpec {
            log_message: Some("alive={_obj.alive()} n={#ids}".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");
    assert_eq!(vm.eval("DAEMON.ticker.list() return 'ran'").unwrap(), "ran");

    let joined = drain_output(&mut rx);
    assert!(!joined.is_empty(), "the logpoint emitted nothing");
    assert!(
        joined.contains("did you mean :alive()"),
        "the missing colon should be named: {joined}"
    );
    // The rest of the message still renders — one bad hole does not kill the
    // line, which is the point of rendering errors in place.
    assert!(joined.contains("n="), "{joined}");
}

/// The hint has to be specific, or it fires on every nil that happens to be
/// called `self`.
#[test]
fn an_unrelated_failure_gets_no_colon_hint() {
    let dbg = debug_state(true);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    *dbg.evt_tx.lock().unwrap() = Some(tx);

    let mut vm = RealVm::boot_fixture_with_probe_opts(TestCtx {
        debug_state: Some(dbg.clone()),
        ..Default::default()
    });

    set_breakpoint(
        &dbg,
        BREAK_FILE,
        185,
        BreakpointSpec {
            log_message: Some("{no_such_global.field}".to_string()),
            ..Default::default()
        },
    );
    assert_eq!(vm.eval("return 'armed'").unwrap(), "armed");
    assert_eq!(vm.eval("DAEMON.ticker.list() return 'ran'").unwrap(), "ran");

    let joined = drain_output(&mut rx);
    assert!(!joined.is_empty(), "the logpoint emitted nothing");
    assert!(
        !joined.contains("did you mean"),
        "the hint fired on an unrelated failure: {joined}"
    );
}

/// Collect every console line the VM emitted.
fn drain_output(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<
        oxigeon::core::scripting::debugger::state::DebugEventMsg,
    >,
) -> String {
    use oxigeon::core::scripting::debugger::state::DebugEventMsg;
    let mut lines = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let DebugEventMsg::Output { text, .. } = msg {
            lines.push(text);
        }
    }
    lines.join(" | ")
}
