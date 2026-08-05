//! Application state and the event/action vocabulary.
//!
//! Every background task talks to the UI through `AppEvent`, and the UI talks
//! back through `Action`. Nothing else is shared, so the draw loop owns all
//! state outright and never locks.

use std::collections::VecDeque;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use tokio::sync::mpsc::UnboundedSender;

/// How many rendered lines of game text to keep. The mudlib pages long output
/// itself, so this only has to cover scrollback a human would actually walk.
const SCROLLBACK: usize = 5_000;

/// Journal lines kept for the bottom strip.
const JOURNAL_LINES: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Play,
    Debug,
    Inspect,
    Trace,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Play, Tab::Debug, Tab::Inspect, Tab::Trace];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Play => "Play",
            Tab::Debug => "Debug",
            Tab::Inspect => "Inspect",
            Tab::Trace => "Trace",
        }
    }
}

/// Everything the background tasks can tell the UI.
#[derive(Debug)]
pub enum AppEvent {
    /// A decoded, ANSI-styled line of game output.
    GameLine(Line<'static>),
    /// A partial line — the prompt, which arrives without a newline.
    GamePrompt(Line<'static>),
    /// An inbound GMCP package and its raw JSON payload.
    Gmcp { package: String, json: String },
    /// The server asked to take over echoing, i.e. this is a password prompt.
    Echo(bool),
    TelnetUp,
    TelnetDown(String),

    /// A decoded DAP message (event or response).
    Dap(serde_json::Value),
    DapUp,
    DapDown(String),

    Journal(crate::journal::Entry),

    Key(KeyEvent),
    Resize(u16, u16),
    /// One-second heartbeat, for the auto-continue countdown.
    Tick,
}

/// Everything the UI can ask a background task to do.
#[derive(Debug, Clone)]
pub enum Action {
    /// A line of player input for the game.
    Send(String),
    /// Tell the server our pane size so the pager wraps correctly.
    Naws(u16, u16),
    /// A DAP request: command plus arguments.
    Dap(String, serde_json::Value),
}

/// Connection state, rendered in the status bar.
#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    Connecting,
    Up,
    Down(String),
}

impl Link {
    pub fn label(&self) -> String {
        match self {
            Link::Connecting => "connecting".into(),
            Link::Up => "up".into(),
            Link::Down(why) => format!("down: {}", why),
        }
    }
}

/// The live GMCP view of the character, fed by `mudlib/daemons/gmcp_d.lua`.
#[derive(Debug, Default, Clone)]
pub struct Vitals {
    pub hp: Option<i64>,
    pub maxhp: Option<i64>,
    pub mp: Option<i64>,
    pub maxmp: Option<i64>,
    pub level: Option<i64>,
    pub xp: Option<i64>,
    pub gold: Option<i64>,
}

#[derive(Debug, Default, Clone)]
pub struct RoomInfo {
    pub id: String,
    pub name: String,
    pub area: String,
    pub exits: Vec<String>,
}

/// One entry of `Char.Effects`. `remaining == -1` means no expiry.
#[derive(Debug, Clone)]
pub struct Effect {
    pub label: String,
    pub remaining: i64,
    pub stacks: i64,
}

pub struct App {
    pub tab: Tab,
    pub should_quit: bool,

    // ─── Play ────────────────────────────────────────────────────────────
    pub scrollback: VecDeque<Line<'static>>,
    /// Lines scrolled back from the bottom; 0 means pinned to the tail.
    pub scroll_offset: usize,
    pub prompt: Option<Line<'static>>,
    pub input: String,
    /// True while the server holds ECHO — mask what we render.
    pub masked: bool,
    pub history: Vec<String>,
    pub history_pos: Option<usize>,
    pub vitals: Vitals,
    pub room: RoomInfo,
    pub effects: Vec<Effect>,
    pub telnet: Link,

    // ─── Debug ───────────────────────────────────────────────────────────
    pub dap: Link,
    pub dbg: crate::dap::DebugView,

    // ─── Journal ─────────────────────────────────────────────────────────
    pub journal: VecDeque<crate::journal::Entry>,
    pub journal_filter: Option<String>,
    pub show_journal: bool,

    actions: UnboundedSender<Action>,
}

impl App {
    pub fn new(actions: UnboundedSender<Action>) -> Self {
        Self {
            tab: Tab::Play,
            should_quit: false,
            scrollback: VecDeque::new(),
            scroll_offset: 0,
            prompt: None,
            input: String::new(),
            masked: false,
            history: Vec::new(),
            history_pos: None,
            vitals: Vitals::default(),
            room: RoomInfo::default(),
            effects: Vec::new(),
            telnet: Link::Connecting,
            dap: Link::Connecting,
            dbg: crate::dap::DebugView::new(),
            journal: VecDeque::new(),
            journal_filter: None,
            show_journal: true,
            actions,
        }
    }

    pub fn act(&self, action: Action) {
        // A closed channel means the task died; the status bar already says so.
        let _ = self.actions.send(action);
    }

    pub fn push_line(&mut self, line: Line<'static>) {
        if self.scrollback.len() >= SCROLLBACK {
            self.scrollback.pop_front();
        }
        self.scrollback.push_back(line);
        // Only follow the tail if the user has not scrolled away from it.
        if self.scroll_offset > 0 {
            self.scroll_offset += 1;
        }
    }

    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::GameLine(line) => self.push_line(line),
            AppEvent::GamePrompt(line) => self.prompt = Some(line),
            AppEvent::Gmcp { package, json } => self.on_gmcp(&package, &json),
            AppEvent::Echo(on) => self.masked = on,
            AppEvent::TelnetUp => self.telnet = Link::Up,
            AppEvent::TelnetDown(why) => self.telnet = Link::Down(why),
            AppEvent::Dap(msg) => self.dbg.on_message(&msg, &self.actions),
            AppEvent::DapUp => {
                self.dap = Link::Up;
                self.dbg.on_connected(&self.actions);
            }
            AppEvent::DapDown(why) => {
                self.dap = Link::Down(why);
                self.dbg.on_disconnected();
            }
            AppEvent::Journal(entry) => {
                if self.journal.len() >= JOURNAL_LINES {
                    self.journal.pop_front();
                }
                self.journal.push_back(entry);
            }
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Resize(w, h) => self.act(Action::Naws(w, h)),
            // Nothing to update — the countdown is derived from an Instant at
            // render time. The tick exists so the frame is drawn at all while
            // the VM is frozen and no other event is arriving.
            AppEvent::Tick => {}
        }
    }

    fn on_gmcp(&mut self, package: &str, json: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
            return;
        };
        let num = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_i64());

        // Package names are matched case-insensitively: gmcp_d lowercases on the
        // way in and clients disagree about capitalisation.
        match package.to_ascii_lowercase().as_str() {
            "char.vitals" => {
                self.vitals.hp = num(&v, "hp");
                self.vitals.maxhp = num(&v, "maxhp");
                self.vitals.mp = num(&v, "mp");
                self.vitals.maxmp = num(&v, "maxmp");
            }
            "char.status" => {
                self.vitals.level = num(&v, "level");
                self.vitals.xp = num(&v, "xp");
                self.vitals.gold = num(&v, "gold");
            }
            "char.effects" => {
                self.effects = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|e| Effect {
                                label: e
                                    .get("label")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("?")
                                    .to_string(),
                                remaining: num(e, "remaining").unwrap_or(-1),
                                stacks: num(e, "stacks").unwrap_or(1),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "room.info" => {
                let s = |k: &str| {
                    v.get(k)
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .to_string()
                };
                self.room = RoomInfo {
                    id: s("id"),
                    name: s("name"),
                    area: s("area"),
                    exits: v
                        .get("exits")
                        .and_then(|x| x.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|e| e.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                };
            }
            _ => {}
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Global bindings first, so they work from any tab.
        match key.code {
            KeyCode::Char('q') if ctrl => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('j') if ctrl => {
                self.show_journal = !self.show_journal;
                return;
            }
            KeyCode::F(n) if (1..=4).contains(&n) => {
                self.tab = Tab::ALL[(n - 1) as usize];
                return;
            }
            KeyCode::BackTab => {
                let i = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
                self.tab = Tab::ALL[(i + Tab::ALL.len() - 1) % Tab::ALL.len()];
                return;
            }
            _ => {}
        }

        match self.tab {
            Tab::Play => self.on_play_key(key),
            Tab::Debug => self.dbg.on_key(key, &self.actions),
            Tab::Inspect => self.dbg.on_inspect_key(key, &self.actions),
            Tab::Trace => self.on_trace_key(key),
        }
    }

    fn on_play_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.input);
                // Never put a password into the recallable history.
                if !self.masked && !line.is_empty() {
                    self.history.push(line.clone());
                }
                self.history_pos = None;
                self.act(Action::Send(line));
                self.scroll_offset = 0;
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Up => self.recall(-1),
            KeyCode::Down => self.recall(1),
            KeyCode::PageUp => self.scroll_offset = self.scroll_offset.saturating_add(10),
            KeyCode::PageDown => self.scroll_offset = self.scroll_offset.saturating_sub(10),
            KeyCode::Esc => {
                self.input.clear();
                self.scroll_offset = 0;
            }
            _ => {}
        }
    }

    fn recall(&mut self, delta: isize) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() - 1;
        self.history_pos = match (self.history_pos, delta) {
            (None, -1) => Some(last),
            (Some(0), -1) => Some(0),
            (Some(i), -1) => Some(i - 1),
            (Some(i), 1) if i >= last => None,
            (Some(i), 1) => Some(i + 1),
            (None, _) => None,
            (some, _) => some,
        };
        self.input = match self.history_pos {
            Some(i) => self.history[i].clone(),
            None => String::new(),
        };
    }

    fn on_trace_key(&mut self, key: KeyEvent) {
        // The trace rings are only reachable through the in-game command, so
        // this tab drives the player session rather than the adapter.
        match key.code {
            KeyCode::Char('t') => self.act(Action::Send("trace time".into())),
            KeyCode::Char('c') => self.act(Action::Send("trace calls".into())),
            KeyCode::Char('o') => self.act(Action::Send("trace off".into())),
            KeyCode::Char('r') => self.act(Action::Send("trace timings 20".into())),
            KeyCode::Char('s') => self.act(Action::Send("trace show 40".into())),
            KeyCode::Char('x') => self.act(Action::Send("trace clear".into())),
            _ => {}
        }
    }
}
