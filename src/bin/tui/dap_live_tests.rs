//! The TUI's DAP client, driven against a **real adapter and a real Lua VM**.
//!
//! The unit tests beside `DebugView` feed it hand-written JSON, which proves the
//! state machine but not that the wire works. The thing those cannot catch is
//! the one that fails silently: a breakpoint path that does not match the
//! `@`-chunk name `require` produced is accepted, answered `verified: true`,
//! and then never fires. Everything looks attached and nothing ever stops.
//!
//! So these run the actual `dap::run` transport and the actual `DebugView`
//! against `debugger::dap::serve`, and assert the VM really stops.
//!
//! Every wait is bounded. A hang here would wedge the whole suite, which is the
//! failure mode this milestone risks.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc as stdmpsc;
use std::time::Duration;

use mlua::Lua;
use oxigeon::config::DebugServerConfig;
use oxigeon::core::scripting::debugger::{
    self, paths, DebugState, HookLocal, InstalledHook, SharedDebugState,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::app::{Action, AppEvent};
use crate::dap::{DebugView, Focus};

const STEP: Duration = Duration::from_secs(10);

/// Line 4 is `return doubled`, inside `work`, reached three times by the loop.
const CHUNK: &str = "local function work(n)\n\
                     \x20   local doubled = n * 2\n\
                     \x20   local label = 'v'\n\
                     \x20   return doubled\n\
                     end\n\
                     local total = 0\n\
                     for i = 1, 3 do total = total + work(i) end\n\
                     return total\n";

/// A Lua thread with the `debug` stdlib loaded and hidden, exactly as
/// `ScriptEngine::start` sets it up when the adapter is enabled. It blocks
/// inside the hook while stopped, which is the whole freeze-the-world model.
fn spawn_vm(
    st: SharedDebugState,
    path: std::path::PathBuf,
) -> (stdmpsc::Sender<()>, stdmpsc::Receiver<()>) {
    let (go_tx, go_rx) = stdmpsc::channel::<()>();
    let (done_tx, done_rx) = stdmpsc::channel::<()>();

    std::thread::spawn(move || {
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
        // The same textual form `require` produces — which is exactly what the
        // client's breakpoint path has to line up with.
        let chunk_name = paths::chunk_name(&path);

        while go_rx.recv().is_ok() {
            debugger::sync_hook(&lua, &st, &mut installed, &hl);
            lua.load(code.as_str())
                .set_name(&chunk_name)
                .exec()
                .unwrap();
            let _ = done_tx.send(());
        }
    });

    (go_tx, done_rx)
}

struct Harness {
    view: DebugView,
    events: UnboundedReceiver<AppEvent>,
    actions: UnboundedSender<Action>,
    go: stdmpsc::Sender<()>,
    done: stdmpsc::Receiver<()>,
    path: std::path::PathBuf,
    /// Commands the adapter has answered. Needed because `attached` flips the
    /// moment `initialized` arrives, which is *before* the breakpoints queued
    /// behind it have reached the wire.
    answered: Vec<String>,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("probe.lua");
        std::fs::write(&path, CHUNK).unwrap();

        let st = DebugState::from_config(&DebugServerConfig::default(), 0);
        let addr = debugger::dap::serve("127.0.0.1", 0, st.clone())
            .await
            .unwrap();
        let (go, done) = spawn_vm(st, path.clone());

        let (events_tx, events) = mpsc::unbounded_channel::<AppEvent>();
        let (actions, actions_rx) = mpsc::unbounded_channel::<Action>();
        tokio::spawn(crate::dap::run(addr.to_string(), events_tx, actions_rx));

        Self {
            view: DebugView::new(),
            events,
            actions,
            go,
            done,
            path,
            answered: Vec::new(),
            _dir: dir,
        }
    }

    /// Feed events into the view until `ready` is satisfied, or time out.
    async fn pump(
        &mut self,
        what: &str,
        mut ready: impl FnMut(&DebugView, &[String]) -> bool,
    ) {
        if ready(&self.view, &self.answered) {
            return;
        }
        let deadline = tokio::time::Instant::now() + STEP;
        loop {
            let event = tokio::time::timeout_at(deadline, self.events.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
                .expect("event channel closed");
            match event {
                AppEvent::DapUp => self.view.on_connected(&self.actions),
                AppEvent::Dap(msg) => {
                    if msg["type"] == "response" {
                        if let Some(command) = msg["command"].as_str() {
                            self.answered.push(command.to_string());
                        }
                    }
                    self.view.on_message(&msg, &self.actions);
                }
                AppEvent::DapDown(why) => panic!("adapter went away while waiting for {what}: {why}"),
                _ => {}
            }
            if ready(&self.view, &self.answered) {
                return;
            }
        }
    }

    /// Run the handshake to completion. `configurationDone` being answered is
    /// the point after which a breakpoint is actually armed — releasing the VM
    /// before it is a race, and one that looks exactly like a wrong path.
    async fn attach(&mut self) {
        self.pump("the handshake", |_, answered| {
            answered.iter().any(|c| c == "configurationDone")
        })
        .await;
        assert!(self.view.attached);
    }

    fn press(&mut self, code: KeyCode) {
        self.view
            .on_key(KeyEvent::new(code, KeyModifiers::NONE), &self.actions);
    }
}

#[tokio::test]
async fn the_client_attaches_and_its_breakpoint_actually_stops_the_vm() {
    let mut h = Harness::start().await;

    // Set the breakpoint the way a user does: open the file, put the cursor on
    // the line, press F9. This is what exercises the path normalisation.
    let path = h.path.clone();
    h.view.open_file(&path);
    h.view.cursor = 3; // line 4
    h.press(KeyCode::F(9));

    h.attach().await;
    h.go.send(()).unwrap();

    h.pump("the breakpoint to fire", |v, _| v.stopped).await;
    assert_eq!(h.view.stop_reason, "breakpoint");

    // If the path had not matched the chunk name the adapter would still have
    // answered `verified: true` and the VM would have run to completion.
    h.pump("a stack", |v, _| !v.frames.is_empty()).await;
    assert_eq!(h.view.frames[0].line, 4);

    h.pump("locals", |v, _| v.vars.iter().any(|n| n.name == "doubled"))
        .await;
    let doubled = h.view.vars.iter().find(|n| n.name == "doubled").unwrap();
    assert_eq!(doubled.value, "2", "first pass through work(1)");

    // And the run really was suspended: it only finishes once we continue.
    assert!(
        h.done.try_recv().is_err(),
        "the VM must still be frozen while stopped"
    );
}

#[tokio::test]
async fn continuing_resumes_the_vm_and_drops_the_variable_handles() {
    let mut h = Harness::start().await;
    let path = h.path.clone();
    h.view.open_file(&path);
    h.view.cursor = 3;
    h.press(KeyCode::F(9));

    h.attach().await;
    h.go.send(()).unwrap();
    h.pump("the breakpoint to fire", |v, _| v.stopped).await;
    h.pump("locals", |v, _| !v.vars.is_empty()).await;

    // The loop calls `work` three times, so three stops, then the chunk ends.
    for _ in 0..3 {
        h.press(KeyCode::F(5));
        h.pump("the resume", |v, _| !v.stopped).await;
        assert!(v_cleared(&h.view), "handles must not survive a resume");
        if h.done.try_recv().is_ok() {
            return;
        }
        // Either it stopped again or it finished; both are fine.
        let _ = tokio::time::timeout(Duration::from_millis(500), h.pump("a stop", |v, _| v.stopped))
            .await;
    }

    fn v_cleared(view: &DebugView) -> bool {
        view.vars.is_empty() && view.frames.is_empty()
    }
}

#[tokio::test]
async fn the_repl_evaluates_in_the_paused_frame() {
    let mut h = Harness::start().await;
    let path = h.path.clone();
    h.view.open_file(&path);
    h.view.cursor = 3;
    h.press(KeyCode::F(9));

    h.attach().await;
    h.go.send(()).unwrap();
    h.pump("the breakpoint to fire", |v, _| v.stopped).await;
    h.pump("a stack", |v, _| !v.frames.is_empty()).await;

    h.view.focus = Focus::Repl;
    for c in "doubled + n".chars() {
        h.press(KeyCode::Char(c));
    }
    h.press(KeyCode::Enter);

    h.pump("the evaluate result", |v, _| {
        v.repl_log.last().is_some_and(|(_, r)| r != "…")
    })
    .await;

    let (expr, result) = h.view.repl_log.last().unwrap();
    assert_eq!(expr, "doubled + n");
    // work(1): doubled = 2, n = 1. Locals of the paused frame are in scope.
    assert_eq!(result, "3");
}

#[tokio::test]
async fn a_second_client_is_told_it_was_refused_rather_than_hanging() {
    // The adapter drops connection number two with no protocol error at all,
    // so a client that does not name this just sits there looking attached.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("probe.lua");
    std::fs::write(&path, CHUNK).unwrap();

    let st = DebugState::from_config(&DebugServerConfig::default(), 0);
    let addr = debugger::dap::serve("127.0.0.1", 0, st.clone())
        .await
        .unwrap();
    let _vm = spawn_vm(st, path);

    // First client: connect and complete the handshake so `clients` is 1.
    let (tx1, mut rx1) = mpsc::unbounded_channel::<AppEvent>();
    let (a1, a1_rx) = mpsc::unbounded_channel::<Action>();
    tokio::spawn(crate::dap::run(addr.to_string(), tx1, a1_rx));
    let mut first = DebugView::new();
    let deadline = tokio::time::Instant::now() + STEP;
    while !first.attached {
        match tokio::time::timeout_at(deadline, rx1.recv())
            .await
            .expect("first client timed out")
            .unwrap()
        {
            AppEvent::DapUp => first.on_connected(&a1),
            AppEvent::Dap(m) => first.on_message(&m, &a1),
            AppEvent::DapDown(w) => panic!("first client rejected: {w}"),
            _ => {}
        }
    }

    // Second client: the connect succeeds, then the socket closes unanswered.
    let (tx2, mut rx2) = mpsc::unbounded_channel::<AppEvent>();
    let (_a2, a2_rx) = mpsc::unbounded_channel::<Action>();
    tokio::spawn(crate::dap::run(addr.to_string(), tx2, a2_rx));

    let deadline = tokio::time::Instant::now() + STEP;
    loop {
        let event = tokio::time::timeout_at(deadline, rx2.recv())
            .await
            .expect("second client neither attached nor reported a refusal")
            .unwrap();
        if let AppEvent::DapDown(why) = event {
            assert!(
                why.contains("another debug client"),
                "the refusal must name its cause, got: {why}"
            );
            return;
        }
    }
}
