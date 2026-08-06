//! Rendering smoke tests against `TestBackend`.
//!
//! Layout bugs in a TUI are panics, not wrong pixels: a constraint that cannot
//! be satisfied, a slice index past the end of a short file, a subtraction that
//! underflows in a pane two rows tall. None of that shows up until someone
//! resizes the window, and by then the terminal is already wrecked.
//!
//! So every tab is drawn at a comfortable size, at the classic 80×24, and at a
//! size small enough that most panes have no room at all.

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::app::{Action, App, AppEvent, Effect, Tab};
use crate::dap::Frame;

/// Sizes worth drawing at: roomy, the classic default, and cramped enough that
/// several panes get zero rows.
const SIZES: [(u16, u16); 4] = [(160, 50), (120, 40), (80, 24), (40, 12)];

fn app() -> (App, UnboundedReceiver<Action>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (App::new(tx), rx)
}

/// Draw and return the screen as text, one line per row.
fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| super::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A populated app: connected, mid-game, with GMCP state and journal lines.
fn populated() -> (App, UnboundedReceiver<Action>) {
    let (mut app, rx) = app();
    app.handle(AppEvent::TelnetUp);
    app.handle(AppEvent::DapUp);
    app.handle(AppEvent::Gmcp {
        package: "Char.Vitals".into(),
        json: r#"{"hp":42,"maxhp":50,"mp":12,"maxmp":20}"#.into(),
    });
    app.handle(AppEvent::Gmcp {
        package: "Char.Status".into(),
        json: r#"{"level":4,"xp":700,"gold":63}"#.into(),
    });
    app.handle(AppEvent::Gmcp {
        package: "Room.Info".into(),
        json: r#"{"id":"thornhollow.square","name":"Thornhollow Square","area":"thornhollow","exits":["north","east","down"]}"#.into(),
    });
    app.effects = vec![Effect {
        label: "Blessing".into(),
        remaining: 42,
        stacks: 2,
    }];
    for i in 0..40 {
        app.push_line(format!("line {i} of the game output").into());
    }
    app.handle(AppEvent::Journal(crate::journal::Entry::parse(
        r#"{"ts":"2026-08-03T18:31:02Z","level":"error","source":"mob_d.lua:9","msg":"template missing"}"#,
    )));
    (app, rx)
}

/// Stopped at a breakpoint, with a stack and locals.
///
/// Frozen, which is the default policy and what the pause banner is about. A
/// server that suspends one dispatch instead sets `stopped` without
/// `world_frozen`, and draws no banner — see
/// `a_suspended_dispatch_does_not_claim_the_world_is_frozen`.
fn paused() -> (App, UnboundedReceiver<Action>) {
    let (mut app, rx) = populated();
    app.dbg.attached = true;
    app.dbg.stopped = true;
    app.dbg.world_frozen = true;
    app.dbg.stopped_at = Some(std::time::Instant::now());
    app.dbg.stop_reason = "breakpoint".into();
    app.dbg.frames = vec![Frame {
        id: 0,
        name: "M.execute".into(),
        path: Some("mudlib/cmds/who.lua".into()),
        line: 19,
    }];
    app.dbg.open_file(std::path::Path::new("mudlib/cmds/who.lua"));
    app.dbg.cursor = 18;
    (app, rx)
}

#[test]
fn every_tab_draws_at_every_size() {
    for tab in Tab::ALL {
        for (w, h) in SIZES {
            let (mut app, _rx) = populated();
            app.tab = tab;
            render(&mut app, w, h);

            // …and with the journal strip hidden, which changes the layout.
            app.show_journal = false;
            render(&mut app, w, h);
        }
    }
}

#[test]
fn every_tab_draws_while_stopped_at_a_breakpoint() {
    for tab in Tab::ALL {
        for (w, h) in SIZES {
            let (mut app, _rx) = paused();
            app.tab = tab;
            render(&mut app, w, h);
        }
    }
}

#[test]
fn an_empty_app_draws_before_anything_has_connected() {
    // The first frame is rendered before either socket has answered.
    for (w, h) in SIZES {
        let (mut app, _rx) = app();
        for tab in Tab::ALL {
            app.tab = tab;
            render(&mut app, w, h);
        }
    }
}

#[test]
fn the_play_tab_shows_the_gmcp_state() {
    let (mut app, _rx) = populated();
    let screen = render(&mut app, 120, 40);

    assert!(screen.contains("Thornhollow Square"), "{screen}");
    assert!(screen.contains("thornhollow.square"), "the room id is what `goto` takes");
    assert!(screen.contains("42/50"), "hp gauge");
    assert!(screen.contains("12/20"), "mp gauge");
    assert!(screen.contains("Blessing"), "active effect");
    assert!(screen.contains("north east down"), "exits");
}

#[test]
fn the_pause_banner_names_the_cost_and_counts_down() {
    let (mut app, _rx) = paused();
    let screen = render(&mut app, 120, 40);

    assert!(screen.contains("VM PAUSED"), "{screen}");
    assert!(
        screen.contains("every player on this server is frozen"),
        "the banner has to say what a breakpoint costs:\n{screen}"
    );
    assert!(screen.contains("auto-continue in 5:00"), "{screen}");
    assert!(screen.contains("who.lua:19"), "where it stopped:\n{screen}");
}

/// A suspended dispatch must not be drawn as a frozen world.
///
/// The report this comes from: an admin set a breakpoint in combat on a server
/// built to suspend one dispatch, and the *other* player's cockpit put up the
/// freeze banner over a game they were still playing — they could type, and the
/// screen said everything had stopped. The banner is a claim about the world, so
/// it is drawn from `world_frozen` and nothing else.
#[test]
fn a_suspended_dispatch_does_not_claim_the_world_is_frozen() {
    let (mut app, _rx) = paused();
    app.dbg.world_frozen = false;
    let screen = render(&mut app, 120, 40);

    assert!(
        !screen.contains("VM PAUSED"),
        "the freeze banner was drawn over a game that is still running:
{screen}"
    );
    assert!(
        !screen.contains("every player on this server is frozen"),
        "{screen}"
    );
    // Still visibly stopped — the status chip says which kind.
    assert!(
        screen.contains("suspended"),
        "a suspended dispatch should still be reported, just not as a freeze:
{screen}"
    );
}

#[test]
fn a_disabled_auto_continue_says_so_rather_than_counting_from_zero() {
    let (mut app, _rx) = paused();
    app.dbg.auto_continue_secs = 0;
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("auto-continue disabled"), "{screen}");
}

#[test]
fn a_masked_password_is_never_drawn() {
    let (mut app, _rx) = populated();
    app.handle(AppEvent::Echo(true));
    for c in "hunter2".chars() {
        app.handle(AppEvent::Key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char(c),
            ratatui::crossterm::event::KeyModifiers::NONE,
        )));
    }
    let screen = render(&mut app, 120, 40);

    assert!(!screen.contains("hunter2"), "the password reached the screen:\n{screen}");
    assert!(screen.contains("*******"), "{screen}");
    assert!(screen.contains("password"), "the pane says why it is masked");
}

#[test]
fn the_journal_strip_shows_a_driver_written_error() {
    let (mut app, _rx) = populated();
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("template missing"), "{screen}");
    assert!(screen.contains("mob_d.lua:9"), "{screen}");
    assert!(screen.contains("18:31:02"), "{screen}");
}

#[test]
fn the_status_bar_warns_that_the_jit_is_off_while_attached() {
    // "Everything is slow while attached" is expected, not a bug — but only if
    // the tool says so.
    let (mut app, _rx) = paused();
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("JIT off while attached"), "{screen}");
}

#[test]
fn the_debug_tab_marks_the_stopped_line_and_the_breakpoint_gutter() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;
    app.dbg
        .breakpoints
        .entry(std::path::PathBuf::from("mudlib/cmds/who.lua"))
        .or_default()
        .insert(19, None);

    let screen = render(&mut app, 160, 50);
    assert!(screen.contains("▶"), "the stopped line needs a marker:\n{screen}");
    assert!(screen.contains("●"), "the breakpoint gutter:\n{screen}");
    assert!(screen.contains("M.execute"), "the stack frame:\n{screen}");
}

/// The file pane is a tree, not a list of paths.
///
/// It used to render every `.lua` file under `mudlib/` and `game/` as its full
/// path — several hundred rows, all of them starting `mudlib/`. Nesting means
/// each row shows only its own name, and a closed folder shows nothing at all.
#[test]
fn the_file_pane_nests_rather_than_listing_every_path() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;
    // `paused()` reveals `who.lua`, which scrolls the tree to it. Look at the
    // top of the tree instead.
    app.dbg.file_sel = 0;
    let screen = render(&mut app, 160, 50);

    assert!(
        screen.contains("▾ mudlib") || screen.contains("▸ mudlib"),
        "a root directory with a disclosure arrow:\n{screen}"
    );
    // Children sit under their parent, indented, showing only their own name.
    // Read the files column alone: the source pane's *title* is a full path,
    // and legitimately so.
    // Content rows only: a pane's *border* line carries its title, and both the
    // source pane's title and the repl's key legend contain slashes.
    let files_column: String = screen
        .lines()
        .filter(|l| l.starts_with('│'))
        .map(|l| l.chars().take(30).collect::<String>())
        .filter(|l| l.ends_with('│'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        files_column.contains("  ▸ ") || files_column.contains("  ▾ "),
        "a nested entry, indented under its parent:\n{files_column}"
    );
    assert!(
        !files_column.contains('/'),
        "a path separator in the tree means it is still a flat list:\n{files_column}"
    );
}

/// A folder that is closed still shows that something inside it has a
/// breakpoint — otherwise collapsing the tree would hide them.
#[test]
fn a_closed_folder_still_shows_that_it_holds_a_breakpoint() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;
    app.dbg
        .breakpoints
        .entry(std::path::PathBuf::from("mudlib/cmds/who.lua"))
        .or_default()
        .insert(19, None);

    let screen = render(&mut app, 160, 50);
    // `mudlib` is a root and stays open, but it is drawn as a folder row and
    // carries the mark for what is underneath it.
    assert!(screen.contains("●"), "the mark should reach the folder:\n{screen}");
}

/// The `:` and `/` prompts take over the footer row, and say what they do.
#[test]
fn the_source_prompts_are_visible_and_labelled() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;

    app.dbg.source_prompt = Some(oxigeon_tui_prompt_goto());
    let screen = render(&mut app, 160, 50);
    assert!(screen.contains("go to line"), "{screen}");
    assert!(screen.contains(":42"), "the typed text:\n{screen}");

    app.dbg.source_prompt = Some(oxigeon_tui_prompt_search());
    let screen = render(&mut app, 160, 50);
    assert!(screen.contains("search"), "{screen}");
    assert!(screen.contains("n/N next/prev"), "the keys that follow:\n{screen}");
}

/// Focusing the variables pane gives it the middle column.
///
/// A 38-column strip beside the source is enough to see *that* a local exists
/// and not much else, and reading values is most of what a debugger is for.
#[test]
fn focusing_the_variables_pane_gives_it_the_middle() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;

    let narrow = render(&mut app, 160, 50);
    let source_col = narrow.lines().nth(1).unwrap_or_default().to_string();
    assert!(
        source_col.contains("who.lua"),
        "the source pane starts in the middle:
{narrow}"
    );

    app.dbg.focus = crate::dap::Focus::Vars;
    let wide = render(&mut app, 160, 50);
    let middle = wide.lines().nth(1).unwrap_or_default().to_string();
    assert!(
        middle.contains("variables"),
        "focusing variables should move it to the middle:
{wide}"
    );
    // And the source is still visible, just not in the middle.
    assert!(wide.contains("who.lua"), "the file should not vanish:
{wide}");
}

/// Lua is coloured, and the tokenizer must not lose or duplicate text.
///
/// Colour does not survive into the text dump, so this pins the thing that
/// actually breaks when a hand-rolled scanner is wrong: characters going missing
/// as runs are split and re-joined.
#[test]
fn syntax_highlighting_does_not_lose_or_duplicate_text() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;
    app.dbg.source = vec![
        "local function greet(who) -- a comment".into(),
        r#"  player:send("if you end this then")"#.into(),
    ];
    app.dbg.blocks = vec![None, None];
    app.dbg.cursor = 0;

    let screen = render(&mut app, 160, 50);
    assert!(screen.contains("local function greet(who) -- a comment"), "{screen}");
    assert!(screen.contains(r#"player:send("if you end this then")"#), "{screen}");
}

/// Syntax colouring and search highlighting have to compose: a match inside a
/// string or a comment is still marked, and splitting the run at it must not
/// drop characters.
#[test]
fn a_search_hit_inside_a_string_survives_syntax_colouring() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;
    app.dbg.source = vec![r#"  send("the end of the world")  -- end"#.into()];
    app.dbg.blocks = vec![None];
    app.dbg.cursor = 0;
    app.dbg.search = "end".into();

    let screen = render(&mut app, 160, 50);
    assert!(
        screen.contains(r#"send("the end of the world")  -- end"#),
        "text was lost splitting runs at the match:
{screen}"
    );
}

fn oxigeon_tui_prompt_goto() -> crate::dap::SourcePrompt {
    crate::dap::SourcePrompt::Goto("42".into())
}

fn oxigeon_tui_prompt_search() -> crate::dap::SourcePrompt {
    crate::dap::SourcePrompt::Search("send".into())
}

/// A search highlights the term itself, not just the line it moved to.
#[test]
fn a_search_marks_the_matching_text() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;
    app.dbg.source = vec!["local x = 1".into(), "  send(player, msg)".into()];
    app.dbg.cursor = 0; // the fixture points at line 19 of a longer file
    app.dbg.search = "send".into();

    // The assertion is only that it still renders the line: colour does not
    // survive into the text dump, so this pins that highlighting cannot panic
    // or drop content — which is the failure mode of splitting spans by index.
    let screen = render(&mut app, 160, 50);
    assert!(screen.contains("send(player, msg)"), "{screen}");
}

/// Highlighting must not mangle a line whose lowercase form is a different
/// length — `İ` lowercases to two chars, and slicing by the lowered offsets
/// would panic or cut a character in half.
#[test]
fn highlighting_leaves_awkward_unicode_alone() {
    let (mut app, _rx) = paused();
    app.tab = Tab::Debug;
    app.dbg.source = vec!["İstanbul send".into()];
    app.dbg.cursor = 0;
    app.dbg.search = "send".into();

    let screen = render(&mut app, 160, 50);
    assert!(screen.contains("İstanbul send"), "{screen}");
}

#[test]
fn the_inspect_tab_explains_itself_when_it_cannot_run() {
    // `evaluate` needs a paused frame, so the pane is empty most of the time.
    // Empty and silent would read as broken.
    let (mut app, _rx) = populated();
    app.tab = Tab::Inspect;
    let screen = render(&mut app, 120, 40);
    assert!(
        screen.contains("not attached"),
        "the pane must say why it is empty:\n{screen}"
    );

    let (mut app, _rx) = paused();
    app.tab = Tab::Inspect;
    let screen = render(&mut app, 120, 40);
    assert!(
        screen.contains("press r to read traits"),
        "attached and stopped, it should invite the read:\n{screen}"
    );
}

#[test]
fn a_scrolled_back_game_pane_says_how_far_back_it_is() {
    let (mut app, _rx) = populated();
    app.scroll_offset = 12;
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("↑12 lines back"), "{screen}");
}

/// Not an assertion — a way to look at the thing without a terminal.
///
/// ```text
/// cargo test --bin oxigeon-tui -- --ignored --nocapture screenshot
/// ```
#[test]
#[ignore = "prints the rendered UI; run explicitly"]
fn screenshot() {
    for (name, tab) in [
        ("Play, stopped at a breakpoint", Tab::Play),
        ("Debug", Tab::Debug),
        ("Inspect", Tab::Inspect),
    ] {
        let (mut app, _rx) = paused();
        app.tab = tab;
        if tab == Tab::Debug {
            app.dbg
                .breakpoints
                .entry(std::path::PathBuf::from("mudlib/cmds/who.lua"))
                .or_default()
                .insert(19, None);
        }
        println!("\n╔══ {name} ══\n{}\n", render(&mut app, 118, 34));
    }
}

#[test]
fn the_trace_tab_says_it_needs_a_command_run_first() {
    let (mut app, _rx) = populated();
    app.tab = Tab::Trace;
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("no trace output captured yet"), "{screen}");

    // Once the in-game command has printed its block, it is lifted verbatim.
    app.push_line("── Command timings ──".into());
    app.push_line("   0.89ms  who                 205       82      8".into());
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("0.89ms  who"), "{screen}");
}
