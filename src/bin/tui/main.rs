//! `oxigeon-tui` — the mudlib development cockpit.
//!
//! Two connections to a running server: telnet for play, DAP for debug, in one
//! window. The point is the seam between them. Hitting a breakpoint stops the
//! entire Lua VM — every player on the server freezes — and from an editor that
//! is invisible. Here the game pane greys out under a banner counting the
//! adapter's own `auto_continue_secs` down, next to the stack that caused it.
//!
//! Nothing in the driver changes to support this. The DAP wire codec, the
//! telnet parser and the path normalisation are all reused from the library.

mod ansi;
mod app;
mod dap;
/// The client driven against a real adapter and a real VM. In the binary rather
/// than `tests/` because it needs `dap::run` and `DebugView` themselves, not a
/// re-implementation of them.
#[cfg(test)]
mod dap_live_tests;
mod inspect_payload;
mod journal;
mod lua_syntax;
mod telnet;
mod ui;

use std::io;
use std::time::Duration;

use app::{Action, App, AppEvent};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc;

/// Where to find things. Ports default to whatever `config/driver.toml` says,
/// so a server on non-standard ports needs no flags here either.
struct Args {
    config: String,
    host: String,
    telnet_port: u16,
    dap_port: u16,
    auto_continue_secs: u64,
    journal: String,
}

const USAGE: &str = "\
oxigeon-tui — play and debug a running Oxigeon server in one window

USAGE:
    oxigeon-tui [OPTIONS]

OPTIONS:
    --config <PATH>    driver config to read ports from [default: config/driver.toml]
    --host <HOST>      server host [default: 127.0.0.1]
    --telnet <PORT>    override the telnet port
    --dap <PORT>       override the debug adapter port
    --journal <PATH>   journal to tail [default: logs/journal.log]
    -h, --help         print this

The debug adapter must be enabled in the driver config:

    [servers.debug]
    enabled = true
";

fn parse_args() -> Result<Args, String> {
    let mut config = "config/driver.toml".to_string();
    let mut host = "127.0.0.1".to_string();
    let mut telnet: Option<u16> = None;
    let mut dap: Option<u16> = None;
    let mut journal = "logs/journal.log".to_string();

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        let mut value = |name: &str| {
            argv.next()
                .ok_or_else(|| format!("{} needs a value", name))
        };
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", USAGE);
                std::process::exit(0);
            }
            "--config" => config = value("--config")?,
            "--host" => host = value("--host")?,
            "--journal" => journal = value("--journal")?,
            "--telnet" => {
                telnet = Some(value("--telnet")?.parse().map_err(|_| "bad --telnet port")?)
            }
            "--dap" => dap = Some(value("--dap")?.parse().map_err(|_| "bad --dap port")?),
            other => return Err(format!("unknown argument '{}'", other)),
        }
    }

    // The config is a convenience, not a requirement — the defaults are the
    // documented ones, so a TUI run from outside the repo still works.
    let (cfg_telnet, cfg_dap, auto) = match oxigeon::config::load_driver_config(&config) {
        Ok(cfg) => {
            let t = cfg.servers.telnet.as_ref().map(|t| t.port);
            let d = cfg.servers.debug.as_ref();
            (t, d.map(|d| d.port), d.map(|d| d.auto_continue_secs))
        }
        Err(_) => (None, None, None),
    };

    Ok(Args {
        config,
        host,
        telnet_port: telnet.or(cfg_telnet).unwrap_or(4000),
        dap_port: dap.or(cfg_dap).unwrap_or(4711),
        auto_continue_secs: auto.unwrap_or(300),
        journal,
    })
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxigeon-tui: {}\n\n{}", e, USAGE);
            std::process::exit(2);
        }
    };
    let _ = &args.config; // kept for error messages

    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (actions_tx, actions_rx) = mpsc::unbounded_channel::<Action>();

    // Terminal input. A dedicated OS thread rather than a tokio task: crossterm's
    // read() blocks, and blocking a runtime worker starves the network tasks.
    {
        let tx = events_tx.clone();
        std::thread::spawn(move || loop {
            match event::poll(Duration::from_millis(200)) {
                Ok(true) => match event::read() {
                    // Windows reports both press and release; only act on press,
                    // or every keystroke registers twice.
                    Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                        if tx.send(AppEvent::Key(k)).is_err() {
                            return;
                        }
                    }
                    Ok(Event::Resize(w, h)) => {
                        if tx.send(AppEvent::Resize(w, h)).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => return,
                },
                Ok(false) => {
                    if tx.is_closed() {
                        return;
                    }
                }
                Err(_) => return,
            }
        });
    }

    // One-second heartbeat, so the auto-continue countdown ticks down even when
    // the server is frozen and nothing else is arriving.
    {
        let tx = events_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                if tx.send(AppEvent::Tick).is_err() {
                    return;
                }
            }
        });
    }

    let telnet_addr = format!("{}:{}", args.host, args.telnet_port);
    let dap_addr = format!("{}:{}", args.host, args.dap_port);

    // Actions fan out to whichever task owns them, so both connection tasks can
    // stay single-owner over their sockets.
    let (telnet_tx, telnet_rx) = mpsc::unbounded_channel::<Action>();
    let (dap_tx, dap_rx) = mpsc::unbounded_channel::<Action>();
    tokio::spawn(route_actions(actions_rx, telnet_tx, dap_tx));

    tokio::spawn(telnet::run(telnet_addr, events_tx.clone(), telnet_rx));
    tokio::spawn(dap::run(dap_addr, events_tx.clone(), dap_rx));
    tokio::spawn(journal::run(args.journal.clone(), events_tx.clone()));

    let mut app = App::new(actions_tx);
    app.dbg.auto_continue_secs = args.auto_continue_secs;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &mut events_rx).await;
    ratatui::restore();
    result
}

/// Split the action stream by destination. Keeping this out of `App` means the
/// UI never has to know which socket a request belongs to.
async fn route_actions(
    mut rx: mpsc::UnboundedReceiver<Action>,
    telnet: mpsc::UnboundedSender<Action>,
    dap: mpsc::UnboundedSender<Action>,
) {
    while let Some(action) = rx.recv().await {
        let sent = match action {
            Action::Dap(..) => dap.send(action),
            Action::Send(_) | Action::Naws(..) => telnet.send(action),
        };
        if sent.is_err() {
            // The owning task is gone. Its status is already on the status bar;
            // dropping the action is the whole of the correct response.
            continue;
        }
    }
}

async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    events: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> io::Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        let Some(first) = events.recv().await else {
            return Ok(());
        };
        app.handle(first);
        // Drain whatever else has queued before redrawing. A burst of game text
        // after a `continue` is one frame, not one frame per line.
        while let Ok(event) = events.try_recv() {
            app.handle(event);
        }
        if app.should_quit {
            return Ok(());
        }
    }
}
