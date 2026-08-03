//! End-to-end DAP: a real TCP client speaking raw protocol against a real Lua VM.
//!
//! Every wait has a timeout. `.cargo/config.toml` forces `--test-threads=1`, so
//! a single hang here would block the whole suite — and "wedges the server" is
//! precisely the failure mode this milestone risks.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use mlua::prelude::*;
use oxigeon::config::DebugServerConfig;
use oxigeon::core::scripting::debugger::{self, paths, DebugState, HookLocal, InstalledHook};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const STEP: Duration = Duration::from_secs(10);

// ─── raw DAP client ──────────────────────────────────────────────────────────

struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    seq: i64,
}

impl Client {
    async fn connect(addr: std::net::SocketAddr) -> Self {
        let stream = tokio::time::timeout(STEP, TcpStream::connect(addr))
            .await
            .expect("connect timed out")
            .expect("connect failed");
        Self { stream, buf: Vec::new(), seq: 0 }
    }

    async fn request(&mut self, command: &str, arguments: Value) {
        self.seq += 1;
        let msg = json!({
            "seq": self.seq, "type": "request",
            "command": command, "arguments": arguments,
        });
        let body = serde_json::to_vec(&msg).unwrap();
        let head = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stream.write_all(head.as_bytes()).await.unwrap();
        self.stream.write_all(&body).await.unwrap();
        self.stream.flush().await.unwrap();
    }

    /// Read one framed message, refilling from the socket as needed.
    async fn recv(&mut self) -> Value {
        loop {
            if let Some(v) = self.take_framed() {
                return v;
            }
            let mut chunk = [0u8; 4096];
            let n = tokio::time::timeout(STEP, self.stream.read(&mut chunk))
                .await
                .expect("timed out waiting for a DAP message")
                .expect("socket read failed");
            assert!(n > 0, "adapter closed the connection unexpectedly");
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    fn take_framed(&mut self) -> Option<Value> {
        let head_end = self.buf.windows(4).position(|w| w == b"\r\n\r\n")?;
        let header = std::str::from_utf8(&self.buf[..head_end]).ok()?;
        let len: usize = header
            .lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
            .and_then(|(_, v)| v.trim().parse().ok())?;
        let start = head_end + 4;
        if self.buf.len() < start + len {
            return None;
        }
        let v = serde_json::from_slice(&self.buf[start..start + len]).unwrap();
        self.buf.drain(..start + len);
        Some(v)
    }

    /// Read until the response for `command` arrives, returning it plus any
    /// events seen along the way.
    async fn response(&mut self, command: &str) -> (Value, Vec<Value>) {
        let mut events = Vec::new();
        loop {
            let m = self.recv().await;
            match m["type"].as_str() {
                Some("response") if m["command"] == command => return (m, events),
                Some("event") => events.push(m),
                _ => {}
            }
        }
    }

    async fn wait_event(&mut self, name: &str) -> Value {
        loop {
            let m = self.recv().await;
            if m["type"] == "event" && m["event"] == name {
                return m;
            }
        }
    }
}

// ─── the Lua side ────────────────────────────────────────────────────────────

/// Runs a Lua VM on its own thread, executing `path` each time it is nudged.
/// Mirrors `ScriptEngine::start`'s hook wiring.
fn spawn_vm(
    st: debugger::SharedDebugState,
    path: std::path::PathBuf,
) -> (mpsc::Sender<()>, mpsc::Receiver<()>, std::thread::JoinHandle<()>) {
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<()>();

    let handle = std::thread::spawn(move || {
        let lua = Lua::new();
        let hl = Rc::new(RefCell::new(HookLocal::new()));
        if let Some(rx) = st.take_vm_rx() {
            hl.borrow_mut().attach_channel(rx);
        }
        let mut installed = InstalledHook::default();
        let code = std::fs::read_to_string(&path).unwrap();
        let chunk_name = paths::chunk_name(&path);

        while go_rx.recv().is_ok() {
            debugger::sync_hook(&lua, &st, &mut installed, &hl);
            lua.load(code.as_str()).set_name(&chunk_name).exec().unwrap();
            let _ = done_tx.send(());
        }
    });

    (go_tx, done_rx, handle)
}

/// Same as [`spawn_vm`], but with the `debug` stdlib loaded and hidden, exactly
/// as `ScriptEngine::start` does when the adapter is enabled.
fn spawn_vm_with_debug_lib(
    st: debugger::SharedDebugState,
    path: std::path::PathBuf,
) -> (mpsc::Sender<()>, mpsc::Receiver<()>, std::thread::JoinHandle<()>) {
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<()>();

    let handle = std::thread::spawn(move || {
        let lua = unsafe {
            Lua::unsafe_new_with(
                mlua::StdLib::ALL_SAFE | mlua::StdLib::DEBUG,
                mlua::LuaOptions::default(),
            )
        };
        debugger::introspect::hide_debug_library(&lua).unwrap();
        debugger::introspect::load_helper(&lua).unwrap();

        let hl = Rc::new(RefCell::new(HookLocal::new()));
        if let Some(rx) = st.take_vm_rx() {
            hl.borrow_mut().attach_channel(rx);
        }
        let mut installed = InstalledHook::default();
        let code = std::fs::read_to_string(&path).unwrap();
        let chunk_name = paths::chunk_name(&path);

        while go_rx.recv().is_ok() {
            debugger::sync_hook(&lua, &st, &mut installed, &hl);
            lua.load(code.as_str()).set_name(&chunk_name).exec().unwrap();
            let _ = done_tx.send(());
        }
    });

    (go_tx, done_rx, handle)
}

/// A chunk whose line numbers are known: line 4 is inside `work`.
const CHUNK: &str = "local function work(n)\n\
                     \x20   local doubled = n * 2\n\
                     \x20   local label = 'v'\n\
                     \x20   return doubled\n\
                     end\n\
                     local total = 0\n\
                     for i = 1, 3 do total = total + work(i) end\n\
                     return total\n";

fn write_chunk(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let p = dir.path().join("probe.lua");
    std::fs::write(&p, CHUNK).unwrap();
    p
}

async fn handshake(c: &mut Client, path: &std::path::Path, lines: &[u32]) {
    c.request("initialize", json!({"adapterID": "oxigeon-lua"})).await;
    let (resp, events) = c.response("initialize").await;
    assert_eq!(resp["success"], true);
    assert_eq!(
        resp["body"]["supportsConfigurationDoneRequest"], true,
        "capabilities must advertise configurationDone"
    );
    // The response must arrive before the `initialized` event.
    assert!(
        events.is_empty(),
        "`initialized` must not precede the initialize response, got {events:?}"
    );
    let init = c.wait_event("initialized").await;
    assert_eq!(init["event"], "initialized");

    c.request("attach", json!({})).await;
    assert_eq!(c.response("attach").await.0["success"], true);

    let bps: Vec<Value> = lines.iter().map(|l| json!({"line": l})).collect();
    c.request("setBreakpoints", json!({
        "source": {"path": path.to_string_lossy()},
        "breakpoints": bps,
    })).await;
    let (resp, _) = c.response("setBreakpoints").await;
    assert_eq!(resp["body"]["breakpoints"].as_array().unwrap().len(), lines.len());

    // VS Code always sends this, and hangs if it goes unanswered.
    c.request("setExceptionBreakpoints", json!({"filters": []})).await;
    assert_eq!(c.response("setExceptionBreakpoints").await.0["success"], true);

    c.request("configurationDone", json!({})).await;
    assert_eq!(c.response("configurationDone").await.0["success"], true);
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn breakpoint_stops_the_vm_and_reports_a_stack() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig::default(), 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, done, _h) = spawn_vm(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[4]).await;

    c.request("threads", json!({})).await;
    let (threads, _) = c.response("threads").await;
    assert_eq!(threads["body"]["threads"][0]["id"], 1);

    go.send(()).unwrap();

    let stopped = c.wait_event("stopped").await;
    assert_eq!(stopped["body"]["reason"], "breakpoint");
    assert_eq!(stopped["body"]["threadId"], 1);
    assert!(st.stopped.load(Ordering::Acquire), "state should reflect the stop");

    c.request("stackTrace", json!({"threadId": 1})).await;
    let (trace, _) = c.response("stackTrace").await;
    let frames = trace["body"]["stackFrames"].as_array().unwrap();
    assert!(!frames.is_empty(), "expected at least one frame");
    assert_eq!(frames[0]["line"], 4, "should report the breakpoint line");
    assert!(
        frames[0]["source"]["path"].as_str().unwrap().ends_with("probe.lua"),
        "frame source should point at the file: {:?}", frames[0]
    );

    c.request("continue", json!({"threadId": 1})).await;
    assert_eq!(c.response("continue").await.0["success"], true);

    // The loop calls `work` three times, so it stops three times in all.
    for _ in 0..2 {
        c.wait_event("stopped").await;
        c.request("continue", json!({"threadId": 1})).await;
        let _ = c.response("continue").await;
    }

    done.recv_timeout(Duration::from_secs(10)).expect("VM never finished the chunk");
    assert!(!st.stopped.load(Ordering::Acquire));
}

/// `next` stays in the frame, `stepOut` returns to the caller. Depth comes from
/// the live VM stack, so this also guards the tail-call drift that would make
/// step-over stop firing.
#[tokio::test]
async fn stepping_moves_through_and_out_of_a_frame() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig::default(), 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, _done, _h) = spawn_vm(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[2]).await; // first line inside `work`
    go.send(()).unwrap();

    assert_eq!(c.wait_event("stopped").await["body"]["reason"], "breakpoint");

    async fn line_now(c: &mut Client) -> u64 {
        c.request("stackTrace", json!({"threadId": 1})).await;
        let (t, _) = c.response("stackTrace").await;
        t["body"]["stackFrames"][0]["line"].as_u64().unwrap()
    }

    assert_eq!(line_now(&mut c).await, 2);

    c.request("next", json!({"threadId": 1})).await;
    let _ = c.response("next").await;
    assert_eq!(c.wait_event("stopped").await["body"]["reason"], "step");
    assert_eq!(line_now(&mut c).await, 3, "next should advance within the frame");

    c.request("next", json!({"threadId": 1})).await;
    let _ = c.response("next").await;
    let _ = c.wait_event("stopped").await;
    assert_eq!(line_now(&mut c).await, 4);

    c.request("stepOut", json!({"threadId": 1})).await;
    let _ = c.response("stepOut").await;
    let _ = c.wait_event("stopped").await;
    assert_eq!(
        line_now(&mut c).await,
        7,
        "stepOut should land back in the calling loop"
    );

    c.request("disconnect", json!({})).await;
    let _ = c.response("disconnect").await;
}

#[tokio::test]
async fn requests_are_rejected_while_the_vm_is_running() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig::default(), 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (_go, _done, _h) = spawn_vm(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[]).await;

    // Nothing is stopped, so this must fail fast rather than sit in the channel
    // until some future breakpoint — which is what would hang a real editor.
    c.request("stackTrace", json!({"threadId": 1})).await;
    let (resp, _) = c.response("stackTrace").await;
    assert_eq!(resp["success"], false);
    assert_eq!(resp["message"], "not stopped");

    c.request("continue", json!({"threadId": 1})).await;
    assert_eq!(c.response("continue").await.0["success"], false);
}

/// The safety valve. If the editor dies while the VM is stopped, the game must
/// recover on its own rather than staying frozen forever.
#[tokio::test]
async fn vm_auto_continues_when_the_client_goes_silent() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let cfg = DebugServerConfig { auto_continue_secs: 1, ..Default::default() };
    let st = DebugState::from_config(&cfg, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, done, _h) = spawn_vm(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[4]).await;

    go.send(()).unwrap();
    let stopped = c.wait_event("stopped").await;
    assert_eq!(stopped["body"]["reason"], "breakpoint");

    // Deliberately never respond.
    done.recv_timeout(Duration::from_secs(20))
        .expect("VM stayed frozen — the auto-continue valve did not fire");
    assert!(!st.stopped.load(Ordering::Acquire));
}

#[tokio::test]
async fn detaching_disarms_the_hook_and_clears_breakpoints() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig::default(), 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[4]).await;
    assert!(st.armed.load(Ordering::Relaxed), "attaching should arm the hook");
    assert_eq!(st.bp_count.load(Ordering::Relaxed), 1);

    c.request("disconnect", json!({})).await;
    let _ = c.response("disconnect").await;

    // Give the adapter a moment to tear the session down.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(st.clients.load(Ordering::Relaxed), 0);
    assert_eq!(st.bp_count.load(Ordering::Relaxed), 0, "breakpoints should not survive detach");
    assert!(
        !st.armed.load(Ordering::Relaxed),
        "hook must be disarmed after detach so the VM runs at full speed"
    );
}

// ─── M4: variables and evaluate ──────────────────────────────────────────────

/// The load-bearing test for the `debug.getlocal` level offset. If the +1
/// measured by the M0 spike were wrong, this would silently return some other
/// frame's locals rather than failing loudly, so it asserts on actual values.
#[tokio::test]
async fn locals_are_readable_at_a_breakpoint() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, _done, _h) = spawn_vm_with_debug_lib(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[4]).await; // `return doubled`, after both locals exist
    go.send(()).unwrap();
    assert_eq!(c.wait_event("stopped").await["body"]["reason"], "breakpoint");

    c.request("scopes", json!({"frameId": 0})).await;
    let (scopes, _) = c.response("scopes").await;
    let scopes = scopes["body"]["scopes"].as_array().unwrap();
    let locals = scopes
        .iter()
        .find(|s| s["name"] == "Locals")
        .expect("a Locals scope should be offered");
    assert!(
        scopes.iter().any(|s| s["name"] == "Globals" && s["expensive"] == true),
        "Globals should be present and marked expensive: {scopes:?}"
    );

    c.request("variables", json!({"variablesReference": locals["variablesReference"]})).await;
    let (vars, _) = c.response("variables").await;
    let vars = vars["body"]["variables"].as_array().unwrap();

    let get = |name: &str| {
        vars.iter()
            .find(|v| v["name"] == name)
            .unwrap_or_else(|| panic!("no local named {name} in {vars:#?}"))
            .clone()
    };

    // The chunk was entered with n = 1, so doubled == 2.
    assert_eq!(get("n")["value"], "1");
    assert_eq!(get("doubled")["value"], "2");
    assert_eq!(get("doubled")["type"], "number");
    assert_eq!(get("label")["value"], "\"v\"", "strings should be quoted");

    // Internal slots must not leak into the pane.
    assert!(
        !vars.iter().any(|v| v["name"].as_str().unwrap_or("").starts_with('(')),
        "(*temporary) style slots should be filtered: {vars:#?}"
    );

    c.request("disconnect", json!({})).await;
    let _ = c.response("disconnect").await;
}

#[tokio::test]
async fn evaluate_runs_in_the_paused_frame() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, _done, _h) = spawn_vm_with_debug_lib(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[4]).await;
    go.send(()).unwrap();
    let _ = c.wait_event("stopped").await;

    async fn eval(c: &mut Client, expr: &str) -> Value {
        c.request("evaluate", json!({"frameId": 0, "expression": expr, "context": "repl"})).await;
        c.response("evaluate").await.0
    }

    // Locals of the paused frame are in scope...
    assert_eq!(eval(&mut c, "doubled").await["body"]["result"], "2");
    assert_eq!(eval(&mut c, "n * 10").await["body"]["result"], "10");
    // ...and so are globals, through the environment's __index.
    assert_eq!(eval(&mut c, "type(tostring)").await["body"]["result"], "\"function\"");

    // A bad expression is a normal REPL outcome: reported, not fatal.
    let bad = eval(&mut c, "no_such_thing.field").await;
    assert_eq!(bad["success"], false);
    assert!(
        bad["message"].as_str().unwrap().contains("nil"),
        "should surface the Lua error: {bad:?}"
    );

    // Assignment is refused rather than silently writing to the wrong frame.
    let assign = eval(&mut c, "doubled = 99").await;
    assert_eq!(assign["success"], false);
    assert!(
        assign["message"].as_str().unwrap().contains("assignment"),
        "should explain why: {assign:?}"
    );

    // The VM must still be usable after all that.
    assert_eq!(eval(&mut c, "doubled").await["body"]["result"], "2");

    c.request("disconnect", json!({})).await;
    let _ = c.response("disconnect").await;
}

/// Variable handles are scoped to one stop. A reference from a previous stop
/// must not resolve, or the pane would show values from a dead frame.
#[tokio::test]
async fn variable_handles_do_not_survive_a_resume() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, _done, _h) = spawn_vm_with_debug_lib(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[4]).await;
    go.send(()).unwrap();
    let _ = c.wait_event("stopped").await;

    c.request("scopes", json!({"frameId": 0})).await;
    let (scopes, _) = c.response("scopes").await;
    let stale = scopes["body"]["scopes"][0]["variablesReference"].clone();

    c.request("continue", json!({"threadId": 1})).await;
    let _ = c.response("continue").await;
    let _ = c.wait_event("stopped").await; // the loop calls `work` again

    c.request("variables", json!({"variablesReference": stale})).await;
    let (vars, _) = c.response("variables").await;
    assert!(
        vars["body"]["variables"].as_array().unwrap().is_empty(),
        "a handle from the previous stop must resolve to nothing, got {vars:?}"
    );

    c.request("disconnect", json!({})).await;
    let _ = c.response("disconnect").await;
}

// ─── conditional breakpoints ─────────────────────────────────────────────────

/// Send breakpoints carrying `condition` / `hitCondition`.
async fn set_conditional(c: &mut Client, path: &std::path::Path, bps: Value) {
    c.request("setBreakpoints", json!({
        "source": {"path": path.to_string_lossy()},
        "breakpoints": bps,
    })).await;
    let (r, _) = c.response("setBreakpoints").await;
    assert_eq!(r["success"], true);
}

/// `work` is called with 1, 2, 3 in turn, so a condition on `n` picks out
/// exactly which iteration stops.
#[tokio::test]
async fn a_condition_selects_which_iteration_stops() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, done, _h) = spawn_vm_with_debug_lib(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[]).await;
    set_conditional(&mut c, &path, json!([{"line": 4, "condition": "n == 2"}])).await;

    go.send(()).unwrap();
    assert_eq!(c.wait_event("stopped").await["body"]["reason"], "breakpoint");

    // Prove it is the n == 2 pass and not merely the first one.
    c.request("evaluate", json!({"frameId": 0, "expression": "n", "context": "repl"})).await;
    assert_eq!(c.response("evaluate").await.0["body"]["result"], "2");

    c.request("continue", json!({"threadId": 1})).await;
    let _ = c.response("continue").await;

    // n == 3 must not match, so the chunk should now run to completion.
    done.recv_timeout(Duration::from_secs(10))
        .expect("a false condition should not have stopped the VM again");
}

#[tokio::test]
async fn a_never_true_condition_never_stops() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, done, _h) = spawn_vm_with_debug_lib(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[]).await;
    set_conditional(&mut c, &path, json!([{"line": 4, "condition": "n > 99"}])).await;

    go.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(10))
        .expect("VM should have run straight through");
    assert!(!st.stopped.load(Ordering::Acquire));
}

/// A condition that raises must stop and say why. Never stopping would be
/// indistinguishable from a broken breakpoint.
#[tokio::test]
async fn a_broken_condition_stops_and_reports_the_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, _done, _h) = spawn_vm_with_debug_lib(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[]).await;
    set_conditional(&mut c, &path, json!([{"line": 4, "condition": "nope.field == 1"}])).await;

    go.send(()).unwrap();

    // The explanatory `output` event and the `stopped` event both arrive; order
    // between them is not guaranteed, so collect until we have seen both.
    let (mut saw_output, mut saw_stop) = (false, false);
    for _ in 0..6 {
        let m = c.recv().await;
        if m["type"] != "event" { continue }
        match m["event"].as_str() {
            Some("output") => {
                let text = m["body"]["output"].as_str().unwrap_or("");
                assert!(text.contains("condition"), "should name the culprit: {text}");
                assert!(text.contains("probe.lua:4"), "should locate it: {text}");
                saw_output = true;
            }
            Some("stopped") => saw_stop = true,
            _ => {}
        }
        if saw_output && saw_stop { break }
    }
    assert!(saw_stop, "a failing condition must still stop");
    assert!(saw_output, "a failing condition must explain itself");

    c.request("disconnect", json!({})).await;
    let _ = c.response("disconnect").await;
}

#[tokio::test]
async fn a_hit_condition_skips_earlier_hits() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, _done, _h) = spawn_vm_with_debug_lib(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[]).await;
    set_conditional(&mut c, &path, json!([{"line": 4, "hitCondition": "3"}])).await;

    go.send(()).unwrap();
    let _ = c.wait_event("stopped").await;

    // Ignoring the first two hits means we land on the n == 3 pass.
    c.request("evaluate", json!({"frameId": 0, "expression": "n", "context": "repl"})).await;
    assert_eq!(c.response("evaluate").await.0["body"]["result"], "3");

    c.request("disconnect", json!({})).await;
    let _ = c.response("disconnect").await;
}

#[tokio::test]
async fn conditional_support_is_advertised() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = write_chunk(&dir);
    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();

    let mut c = Client::connect(addr).await;
    c.request("initialize", json!({"adapterID": "oxigeon-lua"})).await;
    let (r, _) = c.response("initialize").await;
    // Without these, VS Code greys out the condition fields in its UI.
    assert_eq!(r["body"]["supportsConditionalBreakpoints"], true);
    assert_eq!(r["body"]["supportsHitConditionalBreakpoints"], true);
    let _ = path;
}

// ─── table previews ──────────────────────────────────────────────────────────

/// `table: 0x025d651ea7b0` tells you nothing. A collapsed table should say what
/// it holds, so the pane is scannable without expanding every row.
#[tokio::test]
async fn tables_preview_their_contents_instead_of_an_address() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("shapes.lua");
    std::fs::write(
        &path,
        "local seq = {10, 20, 30}\n\
         local map = {name = 'varuser', hp = 100}\n\
         local empty = {}\n\
         local nested = {inner = {a = 1}}\n\
         local big = {}\n\
         for i = 1, 30 do big['k' .. i] = i end\n\
         local named = setmetatable({}, {__tostring = function() return 'Player<varuser>' end})\n\
         local done = true\n\
         return done\n",
    )
    .unwrap();

    let st = DebugState::from_config(&DebugServerConfig { enabled: true, ..Default::default() }, 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone()).await.unwrap();
    let (go, _done, _h) = spawn_vm_with_debug_lib(st.clone(), path.clone());

    let mut c = Client::connect(addr).await;
    handshake(&mut c, &path, &[9]).await; // `return done`, with everything in scope
    go.send(()).unwrap();
    let _ = c.wait_event("stopped").await;

    c.request("scopes", json!({"frameId": 0})).await;
    let (scopes, _) = c.response("scopes").await;
    let locals_ref = scopes["body"]["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Locals")
        .unwrap()["variablesReference"]
        .clone();

    c.request("variables", json!({"variablesReference": locals_ref})).await;
    let (vars, _) = c.response("variables").await;
    let vars = vars["body"]["variables"].as_array().unwrap();
    let val = |name: &str| {
        vars.iter()
            .find(|v| v["name"] == name)
            .unwrap_or_else(|| panic!("no local {name} in {vars:#?}"))["value"]
            .as_str()
            .unwrap()
            .to_string()
    };

    assert!(!val("map").contains("0x"), "a raw address is not a preview: {}", val("map"));
    assert!(val("map").contains("varuser"), "map preview: {}", val("map"));
    assert!(val("map").contains("(2)"), "should carry a count: {}", val("map"));

    // A pure sequence reads as a list, not a record.
    assert!(val("seq").starts_with('['), "seq preview: {}", val("seq"));
    assert!(val("seq").contains("10, 20, 30"), "seq preview: {}", val("seq"));

    assert_eq!(val("empty"), "{}");

    // Nesting collapses rather than recursing — children are one click away.
    assert!(val("nested").contains("{...}"), "nested preview: {}", val("nested"));

    // Large tables elide but still report the true size.
    assert!(val("big").contains("..."), "big preview should elide: {}", val("big"));
    assert!(val("big").contains("(30)"), "big preview should count: {}", val("big"));

    // A mudlib object that defines __tostring knows best.
    assert_eq!(val("named"), "Player<varuser>");

    // Previews are summaries, not replacements: the row is still expandable.
    let map_ref = vars.iter().find(|v| v["name"] == "map").unwrap()["variablesReference"]
        .as_i64()
        .unwrap();
    assert!(map_ref > 0, "a previewed table must still be expandable");
    c.request("variables", json!({"variablesReference": map_ref})).await;
    let (children, _) = c.response("variables").await;
    assert_eq!(children["body"]["variables"].as_array().unwrap().len(), 2);

    c.request("disconnect", json!({})).await;
    let _ = c.response("disconnect").await;
}



