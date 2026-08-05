//! The Trace tab.
//!
//! The trace rings live in a thread-local on the Lua thread and are exposed
//! only as pre-rendered strings through the `trace_*` efuns — there is no path
//! out of the process for them. So this tab drives the in-game `trace` command
//! over the player session and shows what came back, which means it needs a
//! character holding `admin` / `efun.trace`.
//!
//! That is text, not data, and it is labelled as such. Structured
//! `trace_*_data` efuns would make this a real pane; until then the output is
//! already well formatted and lifting it verbatim is honest.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::ui::pane;

/// Headers the in-game command prints, used to find the blocks in scrollback.
const HEADERS: [&str; 3] = ["── Trace ──", "── Command timings ──", "Trace status"];

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(area);

    draw_controls(frame, rows[0]);
    draw_capture(frame, rows[1], app);
}

fn draw_controls(frame: &mut Frame, area: Rect) {
    let key = |k: &'static str, what: &'static str| {
        vec![
            Span::styled(
                format!(" {} ", k),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {}   ", what), Style::default()),
        ]
    };

    let mut line = Vec::new();
    line.extend(key("t", "trace time"));
    line.extend(key("c", "trace calls"));
    line.extend(key("o", "trace off"));
    line.extend(key("r", "timings"));
    line.extend(key("s", "show"));
    line.extend(key("x", "clear"));

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(line),
            Line::from(Span::styled(
                "runs the in-game command on your session — needs admin / efun.trace. \
                 While tracing, the traced code runs interpreted.",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(pane("trace control", false)),
        area,
    );
}

/// Show the tail of the game scrollback from the last trace header onward.
/// The output is one block, printed in response to one command, so anchoring on
/// the header and running to the end of scrollback captures exactly it.
fn draw_capture(frame: &mut Frame, area: Rect, app: &App) {
    let start = app
        .scrollback
        .iter()
        .rposition(|line| {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            HEADERS.iter().any(|h| text.contains(h))
        });

    let Some(start) = start else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no trace output captured yet — press t then run a command, then r",
                Style::default().fg(Color::DarkGray),
            )))
            .block(pane("capture", false)),
            area,
        );
        return;
    };

    let height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = app
        .scrollback
        .iter()
        .skip(start)
        .take(height)
        .cloned()
        .collect();

    frame.render_widget(
        Paragraph::new(lines).block(pane("capture — verbatim from the game session", false)),
        area,
    );
}
