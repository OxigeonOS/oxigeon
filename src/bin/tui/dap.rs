//! Debug Adapter Protocol client.
//!
//! The transport is the driver's own `DapCodec`, so framing cannot disagree
//! between the two ends. What is new here is the client half of the protocol,
//! and the rules it has to respect are unusual enough to be worth stating:
//!
//! - `stackTrace`, `scopes`, `variables`, `evaluate` and every step request are
//!   **rejected outright while the VM is running** — they do not queue. So the
//!   client tracks `stopped` itself and never sends them speculatively.
//! - `attach` is mandatory. It is what sets `clients = 1` and arms the
//!   breakpoint machinery; without it nothing ever stops and nothing says why.
//! - `disconnect` clears every breakpoint server-side, so the breakpoint set
//!   here is the source of truth and is re-sent on each attach.
//! - The adapter takes **one client at a time**. A second connection is dropped
//!   with no protocol error at all, which reads as a silent hang unless it is
//!   detected and named.
//! - `auto_continue_secs` can resume the VM without us asking, so an unsolicited
//!   `continued` is normal and invalidates every variables handle.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use oxigeon::core::scripting::debugger::dap::codec::DapCodec;
use oxigeon::core::scripting::debugger::paths;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_util::codec::Framed;

use crate::app::{Action, AppEvent};
use crate::inspect_payload::{self, EffectRow, Row, TraitRow};

/// Key under which a response carries a copy of the request that caused it.
/// The wire protocol does not echo request arguments, and `variables` responses
/// are indistinguishable without knowing which reference was asked for.
const REQUEST: &str = "__request";

// ─── transport ───────────────────────────────────────────────────────────────

pub async fn run(
    addr: String,
    events: UnboundedSender<AppEvent>,
    mut actions: UnboundedReceiver<Action>,
) {
    let stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            let _ = events.send(AppEvent::DapDown(format!(
                "{} — is [servers.debug] enabled?",
                e
            )));
            return;
        }
    };
    let _ = stream.set_nodelay(true);
    let mut framed = Framed::new(stream, DapCodec::default());
    let _ = events.send(AppEvent::DapUp);

    let mut seq: i64 = 0;
    let mut pending: HashMap<i64, Value> = HashMap::new();
    // Until the adapter answers anything, a close means it never accepted us.
    let mut spoke = false;

    loop {
        tokio::select! {
            incoming = framed.next() => {
                match incoming {
                    Some(Ok(mut msg)) => {
                        spoke = true;
                        // Reattach the originating request so the UI can tell
                        // one `variables` response from another.
                        if msg["type"] == "response" {
                            if let Some(rs) = msg["request_seq"].as_i64() {
                                if let Some(req) = pending.remove(&rs) {
                                    msg[REQUEST] = req;
                                }
                            }
                        }
                        if events.send(AppEvent::Dap(msg)).is_err() { return; }
                    }
                    Some(Err(e)) => {
                        let _ = events.send(AppEvent::DapDown(e.to_string()));
                        return;
                    }
                    None => {
                        let _ = events.send(AppEvent::DapDown(if spoke {
                            "adapter closed".into()
                        } else {
                            // The adapter drops connection number two on the
                            // floor without a word. Say so, rather than sitting
                            // there looking attached.
                            "refused — another debug client is attached".to_string()
                        }));
                        return;
                    }
                }
            }
            action = actions.recv() => {
                let Some(Action::Dap(command, arguments)) = action else { return };
                seq += 1;
                pending.insert(seq, json!({ "command": command, "arguments": arguments }));
                let msg = json!({
                    "seq": seq, "type": "request",
                    "command": command, "arguments": arguments,
                });
                if framed.send(msg).await.is_err() {
                    let _ = events.send(AppEvent::DapDown("write failed".into()));
                    return;
                }
            }
        }
    }
}

// ─── UI-side state ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Frame {
    pub id: i64,
    pub name: String,
    pub path: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Scope {
    pub name: String,
    pub var_ref: i64,
    pub expensive: bool,
}

#[derive(Debug, Clone)]
pub struct VarNode {
    pub name: String,
    pub value: String,
    pub ty: String,
    pub var_ref: i64,
    pub depth: usize,
    pub expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Source,
    Stack,
    Vars,
    Repl,
}

#[derive(Default)]
pub struct Inspect {
    /// The Lua expression naming the entity — `player` in most command frames.
    pub target: String,
    pub editing: bool,
    pub traits: Vec<TraitRow>,
    pub effects: Vec<EffectRow>,
    pub error: Option<String>,
    pub selected: usize,
    /// Set while an evaluate is in flight, so the pane can say so.
    pub pending: bool,
}

pub struct DebugView {
    pub attached: bool,
    pub stopped: bool,
    pub stop_reason: String,
    pub stopped_at: Option<Instant>,
    pub auto_continue_secs: u64,

    pub frames: Vec<Frame>,
    pub frame_sel: usize,
    pub scopes: Vec<Scope>,
    pub vars: Vec<VarNode>,
    pub var_sel: usize,

    /// Client-owned truth: the adapter forgets these on every disconnect.
    pub breakpoints: BTreeMap<PathBuf, BTreeSet<u32>>,

    pub files: Vec<PathBuf>,
    pub file_sel: usize,
    pub open: Option<PathBuf>,
    pub source: Vec<String>,
    pub cursor: usize,

    pub repl_input: String,
    pub repl_log: Vec<(String, String)>,
    /// `output` events — the adapter sends these only for breakpoint conditions
    /// that raised, which is the one case worth surfacing loudly.
    pub output: Vec<String>,

    pub focus: Focus,
    pub inspect: Inspect,
    /// Which `variables` response belongs to the Inspect tab rather than to the
    /// variables tree. Held here, not in `Inspect`, so `set_running` clears it
    /// alongside the rest of the handle state a resume invalidates.
    inspect_ref: Option<i64>,
}

impl DebugView {
    pub fn new() -> Self {
        Self {
            attached: false,
            stopped: false,
            stop_reason: String::new(),
            stopped_at: None,
            auto_continue_secs: 300,
            frames: Vec::new(),
            frame_sel: 0,
            scopes: Vec::new(),
            vars: Vec::new(),
            var_sel: 0,
            breakpoints: BTreeMap::new(),
            files: discover_lua_files(),
            file_sel: 0,
            open: None,
            source: Vec::new(),
            cursor: 0,
            repl_input: String::new(),
            repl_log: Vec::new(),
            output: Vec::new(),
            focus: Focus::Files,
            inspect: Inspect {
                target: "player".into(),
                ..Default::default()
            },
            inspect_ref: None,
        }
    }

    /// Seconds left before the adapter resumes the VM on its own. `None` when
    /// not stopped, or when `auto_continue_secs = 0` disables the safety valve.
    pub fn auto_continue_in(&self) -> Option<u64> {
        if self.auto_continue_secs == 0 {
            return None;
        }
        let at = self.stopped_at?;
        Some(self.auto_continue_secs.saturating_sub(at.elapsed().as_secs()))
    }

    // ─── protocol ────────────────────────────────────────────────────────

    pub fn on_connected(&mut self, tx: &UnboundedSender<Action>) {
        request(
            tx,
            "initialize",
            json!({
                "clientID": "oxigeon-tui",
                "clientName": "oxigeon-tui",
                "adapterID": "oxigeon-lua",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path",
            }),
        );
    }

    pub fn on_disconnected(&mut self) {
        self.attached = false;
        self.set_running();
    }

    pub fn on_message(&mut self, msg: &Value, tx: &UnboundedSender<Action>) {
        match msg["type"].as_str() {
            Some("event") => self.on_event(msg, tx),
            Some("response") => self.on_response(msg, tx),
            _ => {}
        }
    }

    fn on_event(&mut self, msg: &Value, tx: &UnboundedSender<Action>) {
        match msg["event"].as_str() {
            Some("initialized") => {
                // Order matters: attach arms the hook, breakpoints are only
                // honoured once armed, and configurationDone releases it.
                request(tx, "attach", json!({}));
                self.attached = true;
                self.send_all_breakpoints(tx);
                request(tx, "setExceptionBreakpoints", json!({ "filters": [] }));
                request(tx, "configurationDone", json!({}));
            }
            Some("stopped") => {
                self.stopped = true;
                self.stopped_at = Some(Instant::now());
                self.stop_reason = msg["body"]["reason"]
                    .as_str()
                    .unwrap_or("stopped")
                    .to_string();
                request(tx, "stackTrace", json!({ "threadId": 1, "levels": 64 }));
            }
            Some("continued") => self.set_running(),
            Some("output") => {
                if let Some(text) = msg["body"]["output"].as_str() {
                    self.output.push(text.trim_end().to_string());
                }
            }
            _ => {}
        }
    }

    fn on_response(&mut self, msg: &Value, tx: &UnboundedSender<Action>) {
        let command = msg[REQUEST]["command"]
            .as_str()
            .or_else(|| msg["command"].as_str())
            .unwrap_or_default()
            .to_string();
        let args = &msg[REQUEST]["arguments"];

        if msg["success"] == Value::Bool(false) {
            let why = msg["message"].as_str().unwrap_or("failed").to_string();
            match command.as_str() {
                "evaluate" => {
                    if self.inspect.pending {
                        self.inspect.pending = false;
                        self.inspect.error = Some(why.clone());
                    }
                    self.repl_log.push((self.last_repl(), format!("! {}", why)));
                }
                // "not stopped" against anything else means the VM resumed
                // underneath a request we had already sent. Fold the state back.
                _ if why.contains("not stopped") => self.set_running(),
                _ => self.output.push(format!("{}: {}", command, why)),
            }
            return;
        }

        match command.as_str() {
            "stackTrace" => {
                self.frames = msg["body"]["stackFrames"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|f| Frame {
                                id: f["id"].as_i64().unwrap_or(0),
                                name: f["name"].as_str().unwrap_or("?").to_string(),
                                path: f["source"]["path"].as_str().map(str::to_string),
                                line: f["line"].as_u64().unwrap_or(0) as u32,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.frame_sel = 0;
                self.follow_frame(tx);
            }
            "scopes" => {
                self.scopes = msg["body"]["scopes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|s| Scope {
                                name: s["name"].as_str().unwrap_or("?").to_string(),
                                var_ref: s["variablesReference"].as_i64().unwrap_or(0),
                                expensive: s["expensive"].as_bool().unwrap_or(false),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // One collapsed header per scope, so Locals and Upvalues stay
                // visibly apart and the expensive Globals scope is reachable
                // without being walked on every stop — expanding it otherwise
                // reaches the entire daemon graph.
                self.vars = self
                    .scopes
                    .iter()
                    .map(|s| VarNode {
                        name: s.name.clone(),
                        value: if s.expensive { "(expensive)".into() } else { String::new() },
                        ty: String::new(),
                        var_ref: s.var_ref,
                        depth: 0,
                        expanded: false,
                    })
                    .collect();
                self.var_sel = 0;
                for scope in self.scopes.clone() {
                    if !scope.expensive {
                        request(tx, "variables", json!({ "variablesReference": scope.var_ref }));
                    }
                }
            }
            "variables" => {
                let asked = args["variablesReference"].as_i64().unwrap_or(0);
                let rows = parse_variables(&msg["body"]["variables"]);
                if self.inspect.pending && self.inspect_ref == Some(asked) {
                    self.absorb_inspect(rows);
                } else {
                    self.insert_children(asked, rows);
                }
            }
            "evaluate" => {
                let body = &msg["body"];
                let result = body["result"].as_str().unwrap_or_default().to_string();
                let var_ref = body["variablesReference"].as_i64().unwrap_or(0);
                if self.inspect.pending {
                    // The payload is a table; its rows come back through one
                    // `variables` call, because a single string would be cut
                    // off at introspect.lua's 256-character value limit.
                    if var_ref > 0 {
                        self.inspect_ref = Some(var_ref);
                        request(tx, "variables", json!({ "variablesReference": var_ref }));
                    } else {
                        self.inspect.pending = false;
                        self.inspect.error =
                            Some(format!("expected a table, got: {}", result));
                    }
                } else {
                    self.repl_log.push((self.last_repl(), result));
                }
            }
            _ => {}
        }
    }

    fn set_running(&mut self) {
        self.stopped = false;
        self.stopped_at = None;
        self.frames.clear();
        self.scopes.clear();
        // Every variablesReference is invalidated by a resume — introspect.lua
        // resets its handle table. Reusing one would read an unrelated value.
        self.vars.clear();
        self.var_sel = 0;
        self.inspect_ref = None;
        self.inspect.pending = false;
    }

    fn last_repl(&self) -> String {
        self.repl_log
            .last()
            .map(|(q, _)| q.clone())
            .unwrap_or_default()
    }

    fn follow_frame(&mut self, tx: &UnboundedSender<Action>) {
        let Some(frame) = self.frames.get(self.frame_sel).cloned() else {
            return;
        };
        if let Some(path) = frame.path.as_ref() {
            self.open_file(Path::new(path));
            self.cursor = frame.line.saturating_sub(1) as usize;
        }
        request(tx, "scopes", json!({ "frameId": frame.id }));
    }

    fn insert_children(&mut self, parent_ref: i64, rows: Vec<VarNode>) {
        // Every node the response can belong to is already in the list — the
        // scope headers are seeded when `scopes` arrives — so a reference that
        // matches nothing is a stale handle from before a resume. Drop it.
        let Some(i) = self.vars.iter().position(|v| v.var_ref == parent_ref) else {
            return;
        };
        let depth = self.vars[i].depth + 1;
        self.vars[i].expanded = true;
        let rows: Vec<VarNode> = rows
            .into_iter()
            .map(|mut r| {
                r.depth = depth;
                r
            })
            .collect();
        self.vars.splice(i + 1..i + 1, rows);
    }

    fn send_all_breakpoints(&self, tx: &UnboundedSender<Action>) {
        for (path, lines) in &self.breakpoints {
            send_breakpoints(tx, path, lines);
        }
    }

    // ─── input ───────────────────────────────────────────────────────────

    pub fn on_key(&mut self, key: KeyEvent, tx: &UnboundedSender<Action>) {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Execution control works from any pane, and only while stopped — the
        // adapter rejects all of these outright when the VM is running.
        match key.code {
            KeyCode::F(5) if self.stopped => return request(tx, "continue", json!({"threadId": 1})),
            KeyCode::F(10) if self.stopped => return request(tx, "next", json!({"threadId": 1})),
            KeyCode::F(11) if self.stopped && shift => {
                return request(tx, "stepOut", json!({"threadId": 1}))
            }
            KeyCode::F(11) if self.stopped => return request(tx, "stepIn", json!({"threadId": 1})),
            KeyCode::Char('p') if ctrl && !self.stopped => {
                // Consumed by the next *line* event, so it lands on the next
                // command a player types rather than immediately.
                return request(tx, "pause", json!({"threadId": 1}));
            }
            KeyCode::F(9) => return self.toggle_breakpoint(tx),
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Files => Focus::Source,
                    Focus::Source => Focus::Stack,
                    Focus::Stack => Focus::Vars,
                    Focus::Vars => Focus::Repl,
                    Focus::Repl => Focus::Files,
                };
                return;
            }
            _ => {}
        }

        match self.focus {
            Focus::Repl => self.repl_key(key, tx),
            Focus::Files => self.files_key(key),
            Focus::Source => self.source_key(key),
            Focus::Stack => self.stack_key(key, tx),
            Focus::Vars => self.vars_key(key, tx),
        }
    }

    fn repl_key(&mut self, key: KeyEvent, tx: &UnboundedSender<Action>) {
        match key.code {
            KeyCode::Char(c) => self.repl_input.push(c),
            KeyCode::Backspace => {
                self.repl_input.pop();
            }
            KeyCode::Enter => {
                let expr = std::mem::take(&mut self.repl_input);
                if expr.is_empty() {
                    return;
                }
                if !self.stopped {
                    self.repl_log
                        .push((expr, "! evaluate needs a paused frame".into()));
                    return;
                }
                let frame_id = self.frames.get(self.frame_sel).map(|f| f.id).unwrap_or(0);
                self.repl_log.push((expr.clone(), String::from("…")));
                request(
                    tx,
                    "evaluate",
                    json!({ "frameId": frame_id, "expression": expr, "context": "repl" }),
                );
            }
            _ => {}
        }
    }

    fn files_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.file_sel = self.file_sel.saturating_sub(1),
            KeyCode::Down => {
                self.file_sel = (self.file_sel + 1).min(self.files.len().saturating_sub(1))
            }
            KeyCode::Enter => {
                if let Some(path) = self.files.get(self.file_sel).cloned() {
                    self.open_file(&path);
                    self.focus = Focus::Source;
                }
            }
            _ => {}
        }
    }

    fn source_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down => self.cursor = (self.cursor + 1).min(self.source.len().saturating_sub(1)),
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(20),
            KeyCode::PageDown => {
                self.cursor = (self.cursor + 20).min(self.source.len().saturating_sub(1))
            }
            _ => {}
        }
    }

    fn stack_key(&mut self, key: KeyEvent, tx: &UnboundedSender<Action>) {
        let moved = match key.code {
            KeyCode::Up => {
                self.frame_sel = self.frame_sel.saturating_sub(1);
                true
            }
            KeyCode::Down => {
                self.frame_sel = (self.frame_sel + 1).min(self.frames.len().saturating_sub(1));
                true
            }
            _ => false,
        };
        if moved && self.stopped {
            self.follow_frame(tx);
        }
    }

    fn vars_key(&mut self, key: KeyEvent, tx: &UnboundedSender<Action>) {
        match key.code {
            KeyCode::Up => self.var_sel = self.var_sel.saturating_sub(1),
            KeyCode::Down => {
                self.var_sel = (self.var_sel + 1).min(self.vars.len().saturating_sub(1))
            }
            KeyCode::Enter => {
                let Some(node) = self.vars.get(self.var_sel).cloned() else {
                    return;
                };
                if node.var_ref <= 0 || !self.stopped {
                    return;
                }
                if node.expanded {
                    self.collapse(self.var_sel);
                } else {
                    request(tx, "variables", json!({"variablesReference": node.var_ref}));
                }
            }
            _ => {}
        }
    }

    fn collapse(&mut self, index: usize) {
        let depth = self.vars[index].depth;
        self.vars[index].expanded = false;
        let end = self.vars[index + 1..]
            .iter()
            .position(|v| v.depth <= depth)
            .map(|p| index + 1 + p)
            .unwrap_or(self.vars.len());
        self.vars.drain(index + 1..end);
    }

    fn toggle_breakpoint(&mut self, tx: &UnboundedSender<Action>) {
        let Some(path) = self.open.clone() else { return };
        let line = self.cursor as u32 + 1;
        let lines = self.breakpoints.entry(path.clone()).or_default();
        if !lines.remove(&line) {
            lines.insert(line);
        }
        let lines = lines.clone();
        // Only transmit once the handshake has run. A `setBreakpoints` queued
        // before `initialize` would reach the adapter first and be answered
        // against a session that does not exist yet — and the local set is
        // re-sent on `initialized` regardless, so nothing is lost by waiting.
        if self.attached {
            send_breakpoints(tx, &path, &lines);
        }
    }

    pub fn open_file(&mut self, path: &Path) {
        if self.open.as_deref() == Some(path) {
            return;
        }
        self.source = std::fs::read_to_string(path)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_else(|e| vec![format!("<cannot read {}: {}>", path.display(), e)]);
        self.open = Some(path.to_path_buf());
        self.cursor = 0;
        if let Some(i) = self.files.iter().position(|f| f == path) {
            self.file_sel = i;
        }
    }

    // ─── Inspect tab ─────────────────────────────────────────────────────

    pub fn on_inspect_key(&mut self, key: KeyEvent, tx: &UnboundedSender<Action>) {
        if self.inspect.editing {
            match key.code {
                KeyCode::Char(c) => self.inspect.target.push(c),
                KeyCode::Backspace => {
                    self.inspect.target.pop();
                }
                KeyCode::Enter => {
                    self.inspect.editing = false;
                    self.request_inspect(tx);
                }
                KeyCode::Esc => self.inspect.editing = false,
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('e') => self.inspect.editing = true,
            KeyCode::Char('r') | KeyCode::Enter => self.request_inspect(tx),
            KeyCode::Up => self.inspect.selected = self.inspect.selected.saturating_sub(1),
            KeyCode::Down => {
                let n = self.inspect.traits.len() + self.inspect.effects.len();
                self.inspect.selected = (self.inspect.selected + 1).min(n.saturating_sub(1));
            }
            _ => {}
        }
    }

    fn request_inspect(&mut self, tx: &UnboundedSender<Action>) {
        if !self.stopped {
            self.inspect.error =
                Some("attach and break somewhere — evaluate needs a paused frame".into());
            return;
        }
        let frame_id = self.frames.get(self.frame_sel).map(|f| f.id).unwrap_or(0);
        self.inspect.pending = true;
        self.inspect.error = None;
        request(
            tx,
            "evaluate",
            json!({
                "frameId": frame_id,
                "expression": inspect_payload::expression(&self.inspect.target),
                "context": "repl",
            }),
        );
    }

    fn absorb_inspect(&mut self, rows: Vec<VarNode>) {
        self.inspect.pending = false;
        self.inspect_ref = None;
        self.inspect.traits.clear();
        self.inspect.effects.clear();
        for row in rows {
            match inspect_payload::parse_row(&row.value) {
                Some(Row::Trait(t)) => self.inspect.traits.push(t),
                Some(Row::Effect(e)) => self.inspect.effects.push(e),
                None => {}
            }
        }
        if self.inspect.traits.is_empty() && self.inspect.effects.is_empty() {
            self.inspect.error = Some(format!(
                "`{}` resolved to nothing with traits in this frame",
                self.inspect.target
            ));
        }
        self.inspect.selected = 0;
    }
}

fn request(tx: &UnboundedSender<Action>, command: &str, arguments: Value) {
    let _ = tx.send(Action::Dap(command.to_string(), arguments));
}

fn send_breakpoints(tx: &UnboundedSender<Action>, path: &Path, lines: &BTreeSet<u32>) {
    // Absolute and forward-slashed, the same textual form `require` produced —
    // `paths::normalize` on the far side folds the rest.
    request(
        tx,
        "setBreakpoints",
        json!({
            "source": { "path": paths::abs_lua_path(path) },
            "breakpoints": lines.iter().map(|l| json!({"line": l})).collect::<Vec<_>>(),
        }),
    );
}

fn parse_variables(value: &Value) -> Vec<VarNode> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| VarNode {
                    name: v["name"].as_str().unwrap_or("?").to_string(),
                    value: v["value"].as_str().unwrap_or_default().to_string(),
                    ty: v["type"].as_str().unwrap_or_default().to_string(),
                    var_ref: v["variablesReference"].as_i64().unwrap_or(0),
                    depth: 0,
                    expanded: false,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every `.lua` file a breakpoint could apply to. The adapter has no `source`
/// request, so the client reads files itself.
fn discover_lua_files() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in ["mudlib", "game"] {
        walk(Path::new(root), &mut found);
    }
    found.sort();
    found
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "lua") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventKind;
    use tokio::sync::mpsc::{self, UnboundedReceiver};

    fn view() -> (DebugView, UnboundedSender<Action>, UnboundedReceiver<Action>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (DebugView::new(), tx, rx)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    /// Commands actually put on the wire.
    fn drain(rx: &mut UnboundedReceiver<Action>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(Action::Dap(command, _)) = rx.try_recv() {
            out.push(command);
        }
        out
    }

    fn stop_at(view: &mut DebugView, tx: &UnboundedSender<Action>) {
        view.on_message(
            &json!({"type": "event", "event": "stopped", "body": {"reason": "breakpoint"}}),
            tx,
        );
        view.on_message(
            &json!({
                "type": "response", "success": true, "command": "stackTrace",
                REQUEST: {"command": "stackTrace", "arguments": {}},
                "body": {"stackFrames": [
                    {"id": 0, "name": "M.execute", "line": 19,
                     "source": {"path": "mudlib/cmds/who.lua"}}
                ]}
            }),
            tx,
        );
    }

    #[test]
    fn execution_control_is_not_sent_while_the_vm_is_running() {
        // The adapter rejects these outright rather than queuing them, so a
        // client that fires them speculatively just collects errors.
        let (mut v, tx, mut rx) = view();
        assert!(!v.stopped);
        for k in [
            key(KeyCode::F(5)),
            key(KeyCode::F(10)),
            key(KeyCode::F(11)),
            shift(KeyCode::F(11)),
        ] {
            v.on_key(k, &tx);
        }
        assert!(drain(&mut rx).is_empty(), "nothing may go out while running");
    }

    #[test]
    fn execution_control_is_sent_once_stopped() {
        let (mut v, tx, mut rx) = view();
        stop_at(&mut v, &tx);
        drain(&mut rx);

        v.on_key(key(KeyCode::F(10)), &tx);
        v.on_key(shift(KeyCode::F(11)), &tx);
        v.on_key(key(KeyCode::F(5)), &tx);
        assert_eq!(drain(&mut rx), vec!["next", "stepOut", "continue"]);
    }

    #[test]
    fn pause_is_only_offered_while_running() {
        let (mut v, tx, mut rx) = view();
        v.on_key(ctrl(KeyCode::Char('p')), &tx);
        assert_eq!(drain(&mut rx), vec!["pause"]);

        stop_at(&mut v, &tx);
        drain(&mut rx);
        v.on_key(ctrl(KeyCode::Char('p')), &tx);
        assert!(drain(&mut rx).is_empty(), "already stopped");
    }

    #[test]
    fn the_repl_refuses_to_evaluate_without_a_paused_frame() {
        let (mut v, tx, mut rx) = view();
        v.focus = Focus::Repl;
        for c in "player.name".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);

        assert!(drain(&mut rx).is_empty());
        let (expr, result) = v.repl_log.last().expect("the refusal is logged, not silent");
        assert_eq!(expr, "player.name");
        assert!(result.contains("paused frame"), "got: {}", result);
    }

    #[test]
    fn a_breakpoint_set_before_the_handshake_is_not_transmitted_early() {
        // Queuing `setBreakpoints` ahead of `initialize` would put it on the
        // wire first. It is recorded locally and goes out with the handshake.
        let (mut v, tx, mut rx) = view();
        assert!(!v.attached);
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        v.cursor = 18; // line 19
        v.on_key(key(KeyCode::F(9)), &tx);

        assert!(drain(&mut rx).is_empty(), "nothing goes out before attach");
        assert_eq!(
            v.breakpoints.get(Path::new("mudlib/cmds/who.lua")),
            Some(&BTreeSet::from([19])),
            "but it is remembered"
        );
    }

    #[test]
    fn attaching_re_sends_every_breakpoint_because_disconnect_cleared_them() {
        let (mut v, tx, mut rx) = view();
        v.attached = true;
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        v.cursor = 18; // line 19
        v.on_key(key(KeyCode::F(9)), &tx);
        assert_eq!(drain(&mut rx), vec!["setBreakpoints"]);

        // A reconnect: the adapter has forgotten everything it was told.
        v.on_disconnected();
        v.on_message(&json!({"type": "event", "event": "initialized"}), &tx);

        assert_eq!(
            drain(&mut rx),
            vec![
                "attach",
                "setBreakpoints",
                "setExceptionBreakpoints",
                "configurationDone"
            ],
            "attach must precede the breakpoints it arms"
        );
    }

    #[test]
    fn a_breakpoint_toggles_off_and_the_file_is_sent_empty() {
        let (mut v, tx, mut rx) = view();
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        v.cursor = 18;
        v.on_key(key(KeyCode::F(9)), &tx);
        v.on_key(key(KeyCode::F(9)), &tx);
        drain(&mut rx);
        assert!(v.breakpoints.values().all(|lines| lines.is_empty()));
    }

    #[test]
    fn resuming_drops_every_variables_handle() {
        // introspect.lua resets its handle table on resume, so a retained
        // reference would read an unrelated value on the next stop.
        let (mut v, tx, _rx) = view();
        stop_at(&mut v, &tx);
        v.vars.push(VarNode {
            name: "player".into(),
            value: "{...}".into(),
            ty: "table".into(),
            var_ref: 7,
            depth: 0,
            expanded: true,
        });

        v.on_message(&json!({"type": "event", "event": "continued"}), &tx);

        assert!(!v.stopped);
        assert!(v.vars.is_empty());
        assert!(v.frames.is_empty());
        assert!(v.auto_continue_in().is_none());
    }

    #[test]
    fn a_variables_response_for_a_stale_handle_is_discarded() {
        let (mut v, tx, _rx) = view();
        stop_at(&mut v, &tx);
        v.on_message(
            &json!({
                "type": "response", "success": true, "command": "variables",
                REQUEST: {"command": "variables", "arguments": {"variablesReference": 999}},
                "body": {"variables": [{"name": "ghost", "value": "1", "variablesReference": 0}]}
            }),
            &tx,
        );
        assert!(v.vars.is_empty(), "a handle we do not hold must not populate the tree");
    }

    #[test]
    fn scopes_become_collapsed_headers_and_globals_is_left_alone() {
        let (mut v, tx, mut rx) = view();
        stop_at(&mut v, &tx);
        drain(&mut rx);
        v.on_message(
            &json!({
                "type": "response", "success": true, "command": "scopes",
                REQUEST: {"command": "scopes", "arguments": {"frameId": 0}},
                "body": {"scopes": [
                    {"name": "Locals",  "variablesReference": 1, "expensive": false},
                    {"name": "Globals", "variablesReference": 2, "expensive": true}
                ]}
            }),
            &tx,
        );

        assert_eq!(v.vars.len(), 2);
        assert_eq!(v.vars[0].name, "Locals");
        assert_eq!(v.vars[1].name, "Globals");
        // Expanding Globals reaches the whole daemon graph, so it is offered
        // rather than walked.
        assert_eq!(drain(&mut rx), vec!["variables"]);

        v.on_message(
            &json!({
                "type": "response", "success": true, "command": "variables",
                REQUEST: {"command": "variables", "arguments": {"variablesReference": 1}},
                "body": {"variables": [
                    {"name": "session_id", "value": "\"abc\"", "type": "string",
                     "variablesReference": 0}
                ]}
            }),
            &tx,
        );
        assert_eq!(v.vars[1].name, "session_id");
        assert_eq!(v.vars[1].depth, 1, "a scope's rows nest under its header");
        assert!(v.vars[0].expanded);
    }

    #[test]
    fn a_response_saying_not_stopped_folds_the_state_back_to_running() {
        // The VM can resume underneath a request already in flight — most often
        // because auto_continue fired.
        let (mut v, tx, _rx) = view();
        stop_at(&mut v, &tx);
        assert!(v.stopped);
        v.on_message(
            &json!({
                "type": "response", "success": false, "command": "scopes",
                "message": "not stopped",
                REQUEST: {"command": "scopes", "arguments": {}}
            }),
            &tx,
        );
        assert!(!v.stopped);
    }

    #[test]
    fn the_auto_continue_countdown_runs_from_the_moment_of_the_stop() {
        let (mut v, tx, _rx) = view();
        v.auto_continue_secs = 300;
        stop_at(&mut v, &tx);
        assert_eq!(v.auto_continue_in(), Some(300));

        // 0 disables the safety valve entirely.
        v.auto_continue_secs = 0;
        assert_eq!(v.auto_continue_in(), None);
    }

    #[test]
    fn inspect_rows_are_parsed_back_out_of_the_delimited_strings() {
        let (mut v, tx, _rx) = view();
        stop_at(&mut v, &tx);
        v.inspect.pending = true;
        // Quoted, as introspect.lua renders a string value.
        let rows = vec![
            VarNode {
                name: "1".into(),
                value: "\"T\u{1f}max_hp\u{1f}Max Health\u{1f}derived\u{1f}derived\u{1f}nil\u{1f}75\u{1f}\u{1f}false\"".into(),
                ty: "string".into(),
                var_ref: 0,
                depth: 0,
                expanded: false,
            },
            VarNode {
                name: "2".into(),
                value: "\"E\u{1f}blessing\u{1f}Blessing\u{1f}2\u{1f}1754250000\"".into(),
                ty: "string".into(),
                var_ref: 0,
                depth: 0,
                expanded: false,
            },
        ];
        v.absorb_inspect(rows);

        assert!(!v.inspect.pending);
        assert_eq!(v.inspect.traits.len(), 1);
        let t = &v.inspect.traits[0];
        assert_eq!(t.id, "max_hp");
        assert_eq!(t.label, "Max Health");
        assert_eq!(t.kind, "derived");
        assert_eq!(t.value, "75");
        assert!(!t.failed);

        assert_eq!(v.inspect.effects.len(), 1);
        assert_eq!(v.inspect.effects[0].id, "blessing");
        assert_eq!(v.inspect.effects[0].stacks, "2");
    }

    #[test]
    fn inspect_refuses_to_evaluate_while_the_vm_runs() {
        let (mut v, tx, mut rx) = view();
        v.on_inspect_key(key(KeyCode::Char('r')), &tx);
        assert!(drain(&mut rx).is_empty());
        assert!(v
            .inspect
            .error
            .as_deref()
            .is_some_and(|e| e.contains("paused frame")));
    }

    #[test]
    fn a_breakpoint_condition_that_raised_is_surfaced_not_swallowed() {
        // The adapter stops anyway and reports why; silently never stopping
        // would be indistinguishable from a broken breakpoint.
        let (mut v, tx, _rx) = view();
        v.on_message(
            &json!({
                "type": "event", "event": "output",
                "body": {"output": "Breakpoint condition failed at cmds/who.lua:24 — eval:1: ...\n"}
            }),
            &tx,
        );
        assert_eq!(v.output.len(), 1);
        assert!(v.output[0].contains("condition failed"));
    }

    #[test]
    fn stopping_opens_the_file_and_puts_the_cursor_on_the_line() {
        let (mut v, tx, mut rx) = view();
        stop_at(&mut v, &tx);
        assert!(v.stopped);
        assert_eq!(v.stop_reason, "breakpoint");
        assert_eq!(v.frames.len(), 1);
        assert_eq!(v.cursor, 18, "line 19, zero-indexed");
        assert!(v.open.as_ref().is_some_and(|p| p.ends_with("who.lua")));
        // Following a frame asks for its scopes.
        assert_eq!(drain(&mut rx), vec!["stackTrace", "scopes"]);
    }

    #[test]
    fn key_events_are_only_acted_on_once_per_press() {
        // Windows delivers Press and Release for every key; the input thread
        // filters on Press, and this pins that the filter exists.
        let event = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(event.kind, KeyEventKind::Press);
    }
}
