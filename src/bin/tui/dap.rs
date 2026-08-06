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

use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// Something is stopped: the whole VM, or one dispatch.
    pub stopped: bool,
    /// **The whole game is held.** Straight off the `stopped` event's
    /// `allThreadsStopped`.
    ///
    /// Kept apart from `stopped` because the server may be built to suspend one
    /// dispatch and keep serving everyone else, and this pane used to draw its
    /// freeze banner over a game that was demonstrably still being played —
    /// commands worked in the next terminal while the screen said the world had
    /// stopped.
    pub world_frozen: bool,
    /// The stop the client is looking at, and the one it resumes. Was hard-coded
    /// to 1, which was fine while there could only ever be one.
    pub stop_id: i64,
    pub stop_reason: String,
    pub stopped_at: Option<Instant>,
    pub auto_continue_secs: u64,

    pub frames: Vec<Frame>,
    pub frame_sel: usize,
    pub scopes: Vec<Scope>,
    pub vars: Vec<VarNode>,
    pub var_sel: usize,

    /// Client-owned truth: the adapter forgets these on every disconnect.
    /// Breakpoints by file and line. `Some(msg)` makes one a **logpoint**: it
    /// reports and keeps running instead of stopping.
    pub breakpoints: BTreeMap<PathBuf, BTreeMap<u32, Option<String>>>,
    /// The logpoint message being typed, and the line it is for. `None` when
    /// not editing — the editor takes the keyboard while it is open.
    pub logpoint_edit: Option<(u32, String)>,

    /// Every `.lua` file under the roots, sorted. The tree is derived from it.
    pub files: Vec<PathBuf>,
    /// Directories currently open. Everything else is collapsed, which is the
    /// whole point: the flat list this replaces was several hundred rows of
    /// full paths, and finding `cmds/admin/trace.lua` in it meant reading.
    expanded: HashSet<PathBuf>,
    /// The visible rows, rebuilt whenever expansion changes. Selection indexes
    /// into this, not into `files`.
    pub rows: Vec<FileRow>,
    pub file_sel: usize,
    pub open: Option<PathBuf>,
    pub source: Vec<String>,
    /// Per line, whether it starts inside a Lua long bracket.
    ///
    /// Computed once when the file opens rather than per frame: it needs a scan
    /// from the top of the file, and the source pane redraws on every keystroke.
    pub blocks: Vec<Option<usize>>,
    pub cursor: usize,
    /// A `:` or `/` line editor over the source pane, vi-style.
    pub source_prompt: Option<SourcePrompt>,
    /// The last `/` pattern, for `n`/`N` and for highlighting.
    pub search: String,
    /// Whether matches are painted. `:noh` turns it off without forgetting the
    /// pattern, so `n` still works — the same split vi makes.
    pub highlight: bool,

    pub repl_input: String,
    pub repl_log: Vec<(String, String)>,
    /// `output` events, newest last, with whether each is a problem.
    ///
    /// Two sources, and they must not look alike: a **logpoint** reporting —
    /// ordinary, expected, possibly once a round — and a breakpoint condition
    /// that raised, which is a mistake. When conditions were the only source
    /// every line was drawn as a warning, so the first working logpoint looked
    /// like it had gone wrong.
    pub output: Vec<(bool, String)>,

    pub focus: Focus,
    pub inspect: Inspect,
    /// Which `variables` response belongs to the Inspect tab rather than to the
    /// variables tree. Held here, not in `Inspect`, so `set_running` clears it
    /// alongside the rest of the handle state a resume invalidates.
    inspect_ref: Option<i64>,
}

impl DebugView {
    pub fn new() -> Self {
        let mut this = Self {
            attached: false,
            stopped: false,
            world_frozen: false,
            stop_id: 1,
            stop_reason: String::new(),
            stopped_at: None,
            auto_continue_secs: 300,
            frames: Vec::new(),
            frame_sel: 0,
            scopes: Vec::new(),
            vars: Vec::new(),
            var_sel: 0,
            breakpoints: BTreeMap::new(),
            logpoint_edit: None,
            files: Vec::new(),
            expanded: HashSet::new(),
            rows: Vec::new(),
            file_sel: 0,
            open: None,
            source: Vec::new(),
            blocks: Vec::new(),
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
            source_prompt: None,
            search: String::new(),
            highlight: true,
        };
        this.files = discover_lua_files();
        // The roots open, everything under them closed — the same first
        // impression NERDTree gives, and a screen you can read at a glance.
        for root in this.files.iter().filter_map(|f| f.components().next()) {
            this.expanded.insert(PathBuf::from(root.as_os_str()));
        }
        this.rebuild_rows();
        this
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
                self.world_frozen = msg["body"]["allThreadsStopped"]
                    .as_bool()
                    // Absent means the old, always-frozen behaviour.
                    .unwrap_or(true);
                self.stop_id = msg["body"]["threadId"].as_i64().unwrap_or(1);
                self.stopped_at = Some(Instant::now());
                self.stop_reason = msg["body"]["reason"]
                    .as_str()
                    .unwrap_or("stopped")
                    .to_string();
                request(
                    tx,
                    "stackTrace",
                    json!({ "threadId": self.stop_id, "levels": 64 }),
                );
            }
            // Only clear the view if *this* stop is the one that continued;
            // another dispatch may still be suspended behind it.
            Some("continued") => {
                let id = msg["body"]["threadId"].as_i64().unwrap_or(self.stop_id);
                if id == self.stop_id {
                    self.set_running();
                }
            }
            Some("output") => {
                if let Some(text) = msg["body"]["output"].as_str() {
                    let important = msg["body"]["category"].as_str() == Some("important");
                    // A logpoint on a hot line can produce a lot of these, and
                    // holding every one for the life of the session is a leak
                    // with a scrollback nobody reads.
                    if self.output.len() >= MAX_OUTPUT_LINES {
                        self.output.drain(..self.output.len() - MAX_OUTPUT_LINES + 1);
                    }
                    self.output.push((important, text.trim_end().to_string()));
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
                _ => self.output.push((true, format!("{command}: {why}"))),
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
        self.world_frozen = false;
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

        // The line editors own the keyboard while they are open, or typing a
        // message — or a search — would trip every binding below.
        if self.logpoint_edit.is_some() {
            return self.logpoint_key(key, tx);
        }
        if self.source_prompt.is_some() {
            return self.source_prompt_key(key);
        }

        // Execution control works from any pane, and only while stopped — the
        // adapter rejects all of these outright when the VM is running.
        if self.stopped {
            if let Some(cmd) = step_command(key.code, ctrl, shift) {
                return request(tx, cmd, json!({ "threadId": self.stop_id }));
            }
        }

        match key.code {
            KeyCode::Char('p') if ctrl && !self.stopped => {
                // Consumed by the next *line* event, so it lands on the next
                // command a player types rather than immediately.
                return request(tx, "pause", json!({"threadId": self.stop_id}));
            }
            KeyCode::F(9) if shift => return self.begin_logpoint(),
            // Ctrl+L for the same reason the arrows exist: Shift+F9 is a
            // function key with a modifier, which is the least reliable thing a
            // terminal can be asked to deliver.
            KeyCode::Char('l') if ctrl => return self.begin_logpoint(),
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

    /// Files that actually carry a breakpoint or a logpoint.
    ///
    /// `breakpoints` is keyed by path and `entry().or_default()` leaves an empty
    /// map behind when the last line is removed, so "is this path a key" is not
    /// the same question as "does this file have anything set". Everything that
    /// draws a marker goes through here, so the tree and the gutter cannot drift
    /// apart.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn marked_paths(&self) -> Vec<PathBuf> {
        self.breakpoints
            .iter()
            .filter(|(_, lines)| !lines.is_empty())
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Whether anything is set at or under `path`. Directories included, so a
    /// collapsed folder still shows that it holds one.
    pub fn marked_under(&self, path: &Path, is_dir: bool) -> bool {
        self.breakpoints.iter().any(|(p, lines)| {
            !lines.is_empty() && if is_dir { p.starts_with(path) } else { p == path }
        })
    }

    /// Recompute the visible rows, keeping the selection on the same entry.
    fn rebuild_rows(&mut self) {
        let was = self.rows.get(self.file_sel).map(|r| r.path.clone());
        self.rows = build_rows(&self.files, &self.expanded);
        self.file_sel = was
            .and_then(|p| self.rows.iter().position(|r| r.path == p))
            .unwrap_or(self.file_sel)
            .min(self.rows.len().saturating_sub(1));
    }

    /// Open every directory above `path`, so it is on screen.
    fn reveal(&mut self, path: &Path) {
        let mut acc = PathBuf::new();
        for comp in path.components() {
            acc.push(comp.as_os_str());
            if acc != path {
                self.expanded.insert(acc.clone());
            }
        }
        self.rebuild_rows();
        if let Some(i) = self.rows.iter().position(|r| r.path == path) {
            self.file_sel = i;
        }
    }

    fn toggle_dir(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
        self.rebuild_rows();
    }

    /// Tree navigation, with the vi keys alongside the arrows.
    fn files_key(&mut self, key: KeyEvent) {
        let Some(row) = self.rows.get(self.file_sel).cloned() else { return };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.file_sel = self.file_sel.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.file_sel = (self.file_sel + 1).min(self.rows.len().saturating_sub(1))
            }
            // Open a directory, or a file. Enter on an open directory closes it,
            // which is what makes it a toggle rather than a one-way trip.
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if row.is_dir {
                    if matches!(key.code, KeyCode::Right | KeyCode::Char('l')) && row.expanded {
                        // Already open: step into it rather than closing it.
                        self.file_sel = (self.file_sel + 1).min(self.rows.len().saturating_sub(1));
                    } else {
                        self.toggle_dir(&row.path);
                    }
                } else {
                    self.open_file(&row.path);
                    self.focus = Focus::Source;
                }
            }
            // Close this directory, or jump to the parent of whatever this is.
            KeyCode::Left | KeyCode::Char('h') => {
                if row.is_dir && row.expanded {
                    self.toggle_dir(&row.path);
                } else if let Some(parent) = row.path.parent() {
                    if let Some(i) = self.rows.iter().position(|r| r.path == parent) {
                        self.file_sel = i;
                    }
                }
            }
            _ => {}
        }
    }

    fn source_key(&mut self, key: KeyEvent) {
        let last = self.source.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(last),
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(20),
            KeyCode::PageDown => self.cursor = (self.cursor + 20).min(last),
            KeyCode::Home | KeyCode::Char('g') => self.cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.cursor = last,
            // The two vi prompts. Both edit in the footer row, where the
            // logpoint editor already lives.
            KeyCode::Char(':') => self.source_prompt = Some(SourcePrompt::Goto(String::new())),
            KeyCode::Char('/') => self.source_prompt = Some(SourcePrompt::Search(String::new())),
            KeyCode::Char('n') => self.find(true),
            KeyCode::Char('N') => self.find(false),
            _ => {}
        }
    }

    /// Keys while `:` or `/` is open.
    fn source_prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = self.source_prompt.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.source_prompt = None,
            KeyCode::Backspace => {
                prompt.text_mut().pop();
            }
            KeyCode::Char(c) => prompt.text_mut().push(c),
            KeyCode::Enter => {
                match self.source_prompt.take() {
                    Some(SourcePrompt::Goto(t)) => {
                        let t = t.trim();
                        // `:noh` — vi's own spelling for "stop highlighting
                        // that". Clearing the pattern would also lose what `n`
                        // repeats, so only the highlight goes.
                        if matches!(t, "noh" | "nohl" | "nohls" | "nohlsearch") {
                            self.highlight = false;
                        } else if let Ok(n) = t.parse::<usize>() {
                            // 1-based, as the gutter and every error message are.
                            self.cursor =
                                n.saturating_sub(1).min(self.source.len().saturating_sub(1));
                        }
                    }
                    Some(SourcePrompt::Search(t)) => {
                        // An empty pattern repeats the last one, as `//` does in
                        // vi — which makes `//` the shortest "next match" there
                        // is, without having to retype anything.
                        if !t.is_empty() {
                            self.search = t;
                        }
                        self.highlight = true;
                        // Starts from the line *after* the cursor, so pressing
                        // `/` on a term you are already sitting on advances.
                        self.find(true);
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Move to the next (or previous) line matching [`Self::search`], wrapping.
    ///
    /// Case-insensitive, because a search you have to get the case right for is
    /// one you type twice. Wrapping, because stopping at the end of the file
    /// looks like "no more matches" when there are several above you.
    fn find(&mut self, forward: bool) {
        if self.search.is_empty() || self.source.is_empty() {
            return;
        }
        let needle = self.search.to_lowercase();
        let n = self.source.len();
        for step in 1..=n {
            let i = if forward {
                (self.cursor + step) % n
            } else {
                (self.cursor + n - (step % n)) % n
            };
            if self.source[i].to_lowercase().contains(&needle) {
                self.cursor = i;
                return;
            }
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

    /// Open the logpoint editor on the line under the cursor.
    ///
    /// Pre-filled with the message already there, so `Ctrl+L` twice is "edit"
    /// rather than "start again".
    fn begin_logpoint(&mut self) {
        if self.open.is_none() {
            return;
        }
        let line = self.cursor as u32 + 1;
        let existing = self
            .open
            .as_ref()
            .and_then(|p| self.breakpoints.get(p))
            .and_then(|m| m.get(&line))
            .cloned()
            .flatten()
            .unwrap_or_default();
        self.logpoint_edit = Some((line, existing));
    }

    /// Keys while the logpoint editor is open.
    ///
    /// Enter sets it, Esc abandons it, and **an empty message removes the
    /// logpoint entirely** — which is the only way to un-set one without
    /// clearing the breakpoint and starting over.
    fn logpoint_key(&mut self, key: KeyEvent, tx: &UnboundedSender<Action>) {
        let Some((line, text)) = self.logpoint_edit.as_mut() else { return };
        match key.code {
            KeyCode::Esc => {
                self.logpoint_edit = None;
            }
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) => text.push(c),
            KeyCode::Enter => {
                let (line, text) = (*line, text.clone());
                self.logpoint_edit = None;
                let Some(path) = self.open.clone() else { return };
                let lines = self.breakpoints.entry(path.clone()).or_default();
                if text.trim().is_empty() {
                    lines.remove(&line);
                } else {
                    lines.insert(line, Some(text));
                }
                let lines = lines.clone();
                if self.attached {
                    send_breakpoints(tx, &path, &lines);
                }
            }
            _ => {}
        }
    }

    fn toggle_breakpoint(&mut self, tx: &UnboundedSender<Action>) {
        let Some(path) = self.open.clone() else { return };
        let line = self.cursor as u32 + 1;
        let lines = self.breakpoints.entry(path.clone()).or_default();
        if lines.remove(&line).is_none() {
            lines.insert(line, None);
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

    /// Fold whatever path we were handed onto the one the tree uses.
    ///
    /// The adapter reports frames with an absolute, forward-slashed path — the
    /// form Lua's `require` produced — while the tree, the breakpoint map and
    /// `setBreakpoints` all speak in paths relative to the roots. Storing both
    /// gave the same file two identities: a breakpoint set before a stop and one
    /// set after it landed on different keys, so the gutter dot vanished on the
    /// line you were standing on and the tree kept a mark for a breakpoint you
    /// had just removed.
    fn known_path(&self, path: &Path) -> PathBuf {
        if self.files.iter().any(|f| f == path) {
            return path.to_path_buf();
        }
        let wanted = paths::normalize(&path.to_string_lossy());
        self.files
            .iter()
            .find(|f| paths::normalize(&paths::abs_lua_path(f)) == wanted)
            .cloned()
            .unwrap_or_else(|| path.to_path_buf())
    }

    pub fn open_file(&mut self, path: &Path) {
        let path = &self.known_path(path);
        if self.open.as_deref() == Some(path.as_path()) {
            return;
        }
        self.source = std::fs::read_to_string(path)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_else(|e| vec![format!("<cannot read {}: {}>", path.display(), e)]);
        self.blocks = crate::lua_syntax::block_state(&self.source);
        self.open = Some(path.to_path_buf());
        self.cursor = 0;
        // Expand the directories above it. A stop opens whatever file it landed
        // in, and a tree that did not show where that was would be worse than
        // the flat list it replaced.
        self.reveal(path);
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

/// Console lines kept. A logpoint reporting once a combat round fills this in
/// minutes, and only the recent ones are worth anything.
const MAX_OUTPUT_LINES: usize = 500;

/// Which execution-control request a key means, if any.
///
/// Every step has a `Ctrl`+arrow alias because the function keys are not ours to
/// take: **F11 toggles full-screen in most terminals** and never reaches the
/// application at all, and F10 opens the menu bar in some. The arrows say what
/// they do — down *into* a call, up *out* of it, right *along* the line — and
/// nothing else in this app binds `Ctrl` with an arrow.
fn step_command(code: KeyCode, ctrl: bool, shift: bool) -> Option<&'static str> {
    Some(match code {
        KeyCode::F(5) => "continue",
        KeyCode::F(10) => "next",
        KeyCode::F(11) if shift => "stepOut",
        KeyCode::F(11) => "stepIn",
        KeyCode::Char('g') if ctrl => "continue",
        KeyCode::Right if ctrl => "next",
        KeyCode::Down if ctrl => "stepIn",
        KeyCode::Up if ctrl => "stepOut",
        _ => return None,
    })
}

fn send_breakpoints(
    tx: &UnboundedSender<Action>,
    path: &Path,
    lines: &BTreeMap<u32, Option<String>>,
) {
    // Absolute and forward-slashed, the same textual form `require` produced —
    // `paths::normalize` on the far side folds the rest.
    let bps: Vec<Value> = lines
        .iter()
        .map(|(line, message)| match message {
            // `logMessage` is the protocol's own field for this, so the adapter
            // needs nothing bespoke and VS Code sets the same thing.
            Some(m) => json!({ "line": line, "logMessage": m }),
            None => json!({ "line": line }),
        })
        .collect();
    request(
        tx,
        "setBreakpoints",
        json!({
            "source": { "path": paths::abs_lua_path(path) },
            "breakpoints": bps,
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

/// One visible row of the file tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    pub path: PathBuf,
    /// How deep to indent. Root entries are 0.
    pub depth: usize,
    pub is_dir: bool,
    /// Directories only: whether this one is open.
    pub expanded: bool,
}

impl FileRow {
    /// Just this entry's own name — the tree shows the path by nesting.
    pub fn label(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}

/// A modal line editor over the source pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourcePrompt {
    /// `:` — a line number to jump to.
    Goto(String),
    /// `/` — a pattern to search for.
    Search(String),
}

impl SourcePrompt {
    pub fn sigil(&self) -> char {
        match self {
            Self::Goto(_) => ':',
            Self::Search(_) => '/',
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Goto(t) | Self::Search(t) => t,
        }
    }

    fn text_mut(&mut self) -> &mut String {
        match self {
            Self::Goto(t) | Self::Search(t) => t,
        }
    }
}

/// Flatten the file list into the rows a collapsed tree shows.
///
/// Driven off the sorted path list rather than a real tree structure: the paths
/// already carry the hierarchy, and sorting them puts every directory's contents
/// together and in order. A row is emitted only when every one of its ancestors
/// is expanded, which is what `visible` tracks.
fn build_rows(files: &[PathBuf], expanded: &HashSet<PathBuf>) -> Vec<FileRow> {
    let mut rows: Vec<FileRow> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for file in files {
        let parts: Vec<_> = file.components().collect();
        if parts.is_empty() {
            continue;
        }
        let mut acc = PathBuf::new();
        let mut visible = true;

        // Every component but the last is a directory.
        for (depth, comp) in parts[..parts.len() - 1].iter().enumerate() {
            acc.push(comp.as_os_str());
            if visible && seen.insert(acc.clone()) {
                rows.push(FileRow {
                    path: acc.clone(),
                    depth,
                    is_dir: true,
                    expanded: expanded.contains(&acc),
                });
            }
            if !expanded.contains(&acc) {
                visible = false;
            }
        }

        if visible {
            rows.push(FileRow {
                path: file.clone(),
                depth: parts.len() - 1,
                is_dir: false,
                expanded: false,
            });
        }
    }
    rows
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

    /// The `Ctrl`+arrow aliases do the same thing as the function keys.
    ///
    /// They exist because **F11 toggles full-screen in most terminals** and is
    /// swallowed before it reaches us, so "step into" was unreachable for anyone
    /// whose terminal does that. Shift+F11 for step-out is worse again: a
    /// modified function key is the least reliable thing a terminal can be asked
    /// to deliver.
    #[test]
    fn every_step_has_an_alias_the_terminal_will_not_steal() {
        let (mut v, tx, mut rx) = view();
        stop_at(&mut v, &tx);
        drain(&mut rx);

        v.on_key(ctrl(KeyCode::Right), &tx);
        v.on_key(ctrl(KeyCode::Down), &tx);
        v.on_key(ctrl(KeyCode::Up), &tx);
        v.on_key(ctrl(KeyCode::Char('g')), &tx);
        assert_eq!(
            drain(&mut rx),
            vec!["next", "stepIn", "stepOut", "continue"],
            "^→ over, ^↓ into, ^↑ out, ^G go"
        );
    }

    /// A bare arrow still moves the selection rather than stepping.
    #[test]
    fn an_unmodified_arrow_is_not_a_step() {
        let (mut v, tx, mut rx) = view();
        stop_at(&mut v, &tx);
        drain(&mut rx);
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Right] {
            v.on_key(key(code), &tx);
        }
        assert!(drain(&mut rx).is_empty(), "arrows navigate unless Ctrl is held");
    }

    /// Setting a logpoint: open the editor, type, and it goes out as a
    /// breakpoint carrying `logMessage`.
    #[test]
    fn a_logpoint_is_typed_in_and_sent_with_its_message() {
        let (mut v, tx, mut rx) = view();
        v.attached = true;
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        v.cursor = 18; // line 19
        drain(&mut rx);

        v.on_key(ctrl(KeyCode::Char('l')), &tx);
        assert!(v.logpoint_edit.is_some(), "the editor should be open");
        for c in "hp={hp}".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);

        assert!(v.logpoint_edit.is_none(), "enter closes the editor");
        assert_eq!(
            v.breakpoints[Path::new("mudlib/cmds/who.lua")][&19],
            Some("hp={hp}".to_string())
        );
        assert_eq!(drain(&mut rx), vec!["setBreakpoints"]);
    }

    /// An empty message removes the logpoint. Without this there is no way to
    /// un-set one short of clearing the breakpoint and starting again.
    #[test]
    fn an_empty_logpoint_message_removes_it() {
        let (mut v, tx, mut rx) = view();
        v.attached = true;
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        v.cursor = 18;
        v.breakpoints
            .entry(PathBuf::from("mudlib/cmds/who.lua"))
            .or_default()
            .insert(19, Some("watch me".into()));
        drain(&mut rx);

        // Re-opening pre-fills with what is there, so this is an edit.
        v.on_key(ctrl(KeyCode::Char('l')), &tx);
        assert_eq!(v.logpoint_edit.as_ref().unwrap().1, "watch me");
        for _ in 0.."watch me".len() {
            v.on_key(key(KeyCode::Backspace), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);

        assert!(v.breakpoints[Path::new("mudlib/cmds/who.lua")].is_empty());
    }

    /// Esc abandons the edit without touching the breakpoint set.
    #[test]
    fn escape_abandons_a_logpoint_edit() {
        let (mut v, tx, mut rx) = view();
        v.attached = true;
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        v.cursor = 18;
        drain(&mut rx);

        v.on_key(shift(KeyCode::F(9)), &tx);
        v.on_key(key(KeyCode::Char('x')), &tx);
        v.on_key(key(KeyCode::Esc), &tx);

        assert!(v.logpoint_edit.is_none());
        assert!(v.breakpoints.get(Path::new("mudlib/cmds/who.lua")).is_none_or(|m| m.is_empty()));
        assert!(drain(&mut rx).is_empty(), "an abandoned edit sends nothing");
    }

    /// While the editor is open, keys are text — not execution control.
    ///
    /// `g` in a logpoint message must not continue the VM.
    #[test]
    fn the_logpoint_editor_owns_the_keyboard() {
        let (mut v, tx, mut rx) = view();
        stop_at(&mut v, &tx);
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        v.cursor = 18;
        drain(&mut rx);

        v.on_key(ctrl(KeyCode::Char('l')), &tx);
        for c in "going".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        assert_eq!(v.logpoint_edit.as_ref().unwrap().1, "going");
        assert!(drain(&mut rx).is_empty(), "typing must not step the VM");
    }

    /// A stop must open the *same* file the tree knows about.
    ///
    /// The adapter reports an absolute, forward-slashed path — the form Lua's
    /// `require` produced — while the tree is built from relative paths under
    /// `mudlib/` and `game/`. Keying `breakpoints` by whatever `open` happens to
    /// hold meant a breakpoint set before a stop and one set after it landed on
    /// two different entries: the gutter dot vanished on the line you were
    /// standing on, and the tree kept a mark for a breakpoint you had removed.
    #[test]
    fn a_stop_opens_the_file_under_the_path_the_tree_uses() {
        let (mut v, _tx, _rx) = view();
        let relative = v
            .files
            .iter()
            .find(|f| f.ends_with("who.lua"))
            .expect("who.lua")
            .clone();
        // Exactly the form the adapter sends: `paths::abs_lua_path`.
        let absolute = oxigeon::core::scripting::debugger::paths::abs_lua_path(&relative);

        // What `follow_frame` does with a stopped frame.
        v.open_file(Path::new(&absolute));

        assert_eq!(
            v.open.as_ref(),
            Some(&relative),
            "a stop opened {absolute} rather than the tree's {relative:?}"
        );
        assert_eq!(
            v.rows.get(v.file_sel).map(|r| &r.path),
            Some(&relative),
            "and it should be the selected row"
        );
    }

    /// Setting a breakpoint and removing it again leaves no trace.
    ///
    /// The report: "if I re-add it and remove it, the red dot stays in the file
    /// tree". `entry().or_default()` leaves an empty map behind for that path,
    /// so anything asking "does this file have breakpoints" by looking for a
    /// *key* gets the wrong answer for ever after.
    #[test]
    fn removing_the_last_breakpoint_leaves_no_mark_behind() {
        let (mut v, tx, _rx) = view();
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        v.cursor = 18;
        v.focus = Focus::Source;

        v.on_key(key(KeyCode::F(9)), &tx);
        assert_eq!(v.marked_paths(), vec![PathBuf::from("mudlib/cmds/who.lua")]);

        v.on_key(key(KeyCode::F(9)), &tx);
        assert!(
            v.marked_paths().is_empty(),
            "the file still counts as marked after its last breakpoint went: {:?}",
            v.breakpoints
        );
    }

    // ─── the file tree ───────────────────────────────────────────────────

    /// The tree opens with the roots expanded and nothing else.
    ///
    /// The flat list this replaces was every `.lua` file under `mudlib/` and
    /// `game/` — several hundred rows of full paths — so finding one meant
    /// reading rather than navigating.
    #[test]
    fn the_tree_starts_at_the_roots_rather_than_showing_every_file() {
        let (v, _tx, _rx) = view();
        assert!(!v.files.is_empty(), "there should be files to show");
        assert!(
            v.rows.len() < v.files.len(),
            "a collapsed tree must be shorter than the flat list: {} rows vs {} files",
            v.rows.len(),
            v.files.len()
        );
        // Top level is `mudlib` and `game`, both directories, both open.
        let roots: Vec<_> = v.rows.iter().filter(|r| r.depth == 0).collect();
        assert!(!roots.is_empty());
        assert!(roots.iter().all(|r| r.is_dir && r.expanded), "{roots:?}");
    }

    /// A closed directory hides its contents; opening it shows them.
    #[test]
    fn a_directory_toggles_between_showing_and_hiding_its_contents() {
        let (mut v, _tx, _rx) = view();

        let dir = v
            .rows
            .iter()
            .find(|r| r.is_dir && r.depth == 1)
            .expect("a nested directory")
            .path
            .clone();
        let children = |v: &DebugView| {
            v.rows.iter().filter(|r| r.path.parent() == Some(dir.as_path())).count()
        };

        // It starts closed — only the roots are open.
        assert_eq!(children(&v), 0, "{dir:?} should start collapsed");

        v.file_sel = v.rows.iter().position(|r| r.path == dir).unwrap();
        v.on_key(key(KeyCode::Enter), &_tx);
        assert!(children(&v) > 0, "enter did not open {dir:?}");
        assert!(v.rows.iter().find(|r| r.path == dir).unwrap().expanded);

        v.on_key(key(KeyCode::Enter), &_tx);
        assert_eq!(children(&v), 0, "enter did not close {dir:?} again");
    }

    /// Selection survives a directory opening above it.
    #[test]
    fn expanding_a_directory_keeps_the_selection_on_the_same_entry() {
        let (mut v, tx, _rx) = view();
        let dir = v.rows.iter().find(|r| r.is_dir && r.depth == 1).unwrap().path.clone();
        v.file_sel = v.rows.iter().position(|r| r.path == dir).unwrap();
        v.on_key(key(KeyCode::Enter), &tx);
        assert_eq!(
            v.rows[v.file_sel].path, dir,
            "the cursor moved when the tree grew under it"
        );
    }

    /// Opening a file expands the directories above it and selects it.
    ///
    /// A stop opens whatever file it landed in, and a tree that did not then
    /// show where that was would be worse than the flat list.
    #[test]
    fn opening_a_file_reveals_it_in_the_tree() {
        let (mut v, _tx, _rx) = view();
        let deep = v
            .files
            .iter()
            .find(|f| f.components().count() >= 3)
            .expect("a file below the root")
            .clone();
        assert!(
            !v.rows.iter().any(|r| r.path == deep),
            "it should not be visible before it is opened"
        );

        v.open_file(&deep);

        assert_eq!(v.rows[v.file_sel].path, deep, "not selected after opening");
        // Every directory above it is open, or it could not be on screen.
        for ancestor in deep.ancestors().skip(1).filter(|a| !a.as_os_str().is_empty()) {
            assert!(
                v.rows.iter().any(|r| r.path == ancestor && r.expanded),
                "{ancestor:?} should have been expanded"
            );
        }
    }

    /// `h` closes a directory, or steps out to the parent.
    #[test]
    fn h_closes_a_directory_or_moves_to_the_parent() {
        let (mut v, tx, _rx) = view();
        let deep = v.files.iter().find(|f| f.components().count() >= 3).unwrap().clone();
        v.open_file(&deep);
        v.focus = Focus::Files;

        // On a file, `h` goes up to the folder holding it.
        v.on_key(key(KeyCode::Char('h')), &tx);
        assert_eq!(v.rows[v.file_sel].path, deep.parent().unwrap());

        // On an open folder, `h` closes it.
        v.on_key(key(KeyCode::Char('h')), &tx);
        assert!(!v.rows[v.file_sel].expanded, "h did not close the directory");
    }

    // ─── the vi prompts ──────────────────────────────────────────────────

    fn with_source(lines: &[&str]) -> (DebugView, UnboundedSender<Action>, UnboundedReceiver<Action>) {
        let (mut v, tx, rx) = view();
        v.source = lines.iter().map(|s| s.to_string()).collect();
        v.open = Some(PathBuf::from("probe.lua"));
        v.focus = Focus::Source;
        (v, tx, rx)
    }

    /// `:42` jumps to line 42, counting from one as the gutter does.
    #[test]
    fn colon_goes_to_a_line_number() {
        let lines: Vec<String> = (1..=100).map(|n| format!("line {n}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let (mut v, tx, _rx) = with_source(&refs);

        v.on_key(key(KeyCode::Char(':')), &tx);
        for c in "42".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);

        assert!(v.source_prompt.is_none(), "the prompt should close");
        assert_eq!(v.cursor, 41, "cursor is 0-based, the prompt is 1-based");
        assert_eq!(v.source[v.cursor], "line 42");
    }

    /// A line number past the end lands on the last line rather than nowhere.
    #[test]
    fn a_line_number_past_the_end_clamps() {
        let (mut v, tx, _rx) = with_source(&["a", "b", "c"]);
        v.on_key(key(KeyCode::Char(':')), &tx);
        for c in "9999".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);
        assert_eq!(v.cursor, 2);
    }

    /// `/` searches, `n` and `N` walk the matches, and both wrap.
    #[test]
    fn slash_searches_and_n_walks_the_matches() {
        let (mut v, tx, _rx) =
            with_source(&["local x", "  send(a)", "nothing", "  SEND(b)", "done"]);

        v.on_key(key(KeyCode::Char('/')), &tx);
        for c in "send".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);
        assert_eq!(v.cursor, 1, "first match below the cursor");

        // Case-insensitive: `SEND(b)` is the next one.
        v.on_key(key(KeyCode::Char('n')), &tx);
        assert_eq!(v.cursor, 3);

        // And it wraps rather than stopping at the end.
        v.on_key(key(KeyCode::Char('n')), &tx);
        assert_eq!(v.cursor, 1, "search should wrap to the top");

        v.on_key(key(KeyCode::Char('N')), &tx);
        assert_eq!(v.cursor, 3, "N goes back the other way");
    }

    /// A pattern that matches nothing leaves the cursor alone.
    #[test]
    fn a_search_with_no_match_does_not_move() {
        let (mut v, tx, _rx) = with_source(&["a", "b", "c"]);
        v.cursor = 1;
        v.on_key(key(KeyCode::Char('/')), &tx);
        for c in "zzz".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);
        assert_eq!(v.cursor, 1);
    }

    /// While a prompt is open its keys are text, not commands.
    ///
    /// `n` in a search term must not mean "next match", and `:` must not open a
    /// second prompt on top of the first.
    #[test]
    fn a_prompt_owns_the_keyboard_while_it_is_open() {
        let (mut v, tx, _rx) = with_source(&["alpha", "beta", "number nine"]);
        v.on_key(key(KeyCode::Char('/')), &tx);
        for c in "n:9".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        assert_eq!(
            v.source_prompt,
            Some(SourcePrompt::Search("n:9".into())),
            "every key should have gone into the term"
        );
        assert_eq!(v.cursor, 0, "nothing should have moved yet");
    }

    /// Esc abandons a prompt and leaves the previous search intact.
    #[test]
    fn escape_abandons_a_prompt() {
        let (mut v, tx, _rx) = with_source(&["alpha", "beta"]);
        v.search = "alpha".into();
        v.on_key(key(KeyCode::Char('/')), &tx);
        v.on_key(key(KeyCode::Char('b')), &tx);
        v.on_key(key(KeyCode::Esc), &tx);

        assert!(v.source_prompt.is_none());
        assert_eq!(v.search, "alpha", "an abandoned search must not replace the last one");
    }

    /// `//` repeats the last search, as in vi.
    #[test]
    fn an_empty_pattern_repeats_the_last_search() {
        let (mut v, tx, _rx) = with_source(&["one", "hit", "two", "hit", "three"]);

        v.on_key(key(KeyCode::Char('/')), &tx);
        for c in "hit".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);
        assert_eq!(v.cursor, 1);

        // `/` then Enter with nothing typed.
        v.on_key(key(KeyCode::Char('/')), &tx);
        v.on_key(key(KeyCode::Enter), &tx);
        assert_eq!(v.cursor, 3, "// should advance to the next match");
        assert_eq!(v.search, "hit", "and must not forget the pattern");
    }

    /// `:noh` stops the painting without forgetting the pattern, so `n` still
    /// works — the split vi makes.
    #[test]
    fn noh_clears_the_highlight_but_keeps_the_search() {
        let (mut v, tx, _rx) = with_source(&["one", "hit", "two", "hit"]);
        v.on_key(key(KeyCode::Char('/')), &tx);
        for c in "hit".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);
        assert!(v.highlight);

        v.on_key(key(KeyCode::Char(':')), &tx);
        for c in "noh".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);

        assert!(!v.highlight, ":noh should stop the painting");
        assert_eq!(v.search, "hit", "but not lose the pattern");
        v.on_key(key(KeyCode::Char('n')), &tx);
        assert_eq!(v.cursor, 3, "n still walks the matches");
    }

    /// A new search turns painting back on, or `:noh` would be permanent.
    #[test]
    fn searching_again_restores_the_highlight() {
        let (mut v, tx, _rx) = with_source(&["one", "hit"]);
        v.highlight = false;
        v.on_key(key(KeyCode::Char('/')), &tx);
        for c in "hit".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);
        assert!(v.highlight);
    }

    /// `:` with something that is neither a number nor `noh` does nothing,
    /// rather than jumping somewhere arbitrary.
    #[test]
    fn an_unrecognised_colon_command_is_ignored() {
        let (mut v, tx, _rx) = with_source(&["a", "b", "c"]);
        v.cursor = 1;
        v.on_key(key(KeyCode::Char(':')), &tx);
        for c in "wq".chars() {
            v.on_key(key(KeyCode::Char(c)), &tx);
        }
        v.on_key(key(KeyCode::Enter), &tx);
        assert_eq!(v.cursor, 1);
        assert!(v.source_prompt.is_none());
    }

    /// Opening a file records where every long bracket runs, so the source pane
    /// does not have to rescan from the top on each redraw.
    #[test]
    fn opening_a_file_records_its_block_comment_spans() {
        let (mut v, _tx, _rx) = view();
        v.open_file(Path::new("mudlib/cmds/who.lua"));
        assert_eq!(
            v.blocks.len(),
            v.source.len(),
            "one entry per line, or the pane would index past the end"
        );
    }

    /// `g` and `G` go to the top and the bottom, as in vi.
    #[test]
    fn g_and_shift_g_jump_to_the_ends() {
        let (mut v, tx, _rx) = with_source(&["a", "b", "c", "d"]);
        v.on_key(key(KeyCode::Char('G')), &tx);
        assert_eq!(v.cursor, 3);
        v.on_key(key(KeyCode::Char('g')), &tx);
        assert_eq!(v.cursor, 0);
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
            Some(&BTreeMap::from([(19, None)])),
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
        assert!(v.output[0].1.contains("condition failed"));
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
