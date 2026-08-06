//! Rendering. Frame layout, the tab bar, the journal strip and the status line
//! live here; each tab's body is its own module.

mod debug;
mod inspect;
mod play;
#[cfg(test)]
mod render_tests;
mod trace;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, Link, Tab};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let journal_height = if app.show_journal { 8 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),              // tab bar
            Constraint::Min(5),                 // body
            Constraint::Length(journal_height), // journal strip
            Constraint::Length(1),              // status
        ])
        .split(frame.area());

    draw_tabs(frame, chunks[0], app);

    match app.tab {
        Tab::Play => play::draw(frame, chunks[1], app),
        Tab::Debug => debug::draw(frame, chunks[1], app),
        Tab::Inspect => inspect::draw(frame, chunks[1], app),
        Tab::Trace => trace::draw(frame, chunks[1], app),
    }

    if app.show_journal {
        draw_journal(frame, chunks[2], app);
    }
    draw_status(frame, chunks[3], app);
}

fn draw_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!(" F{} {} ", i + 1, t.title())))
        .collect();
    let selected = Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .divider("");
    frame.render_widget(tabs, area);
}

fn draw_journal(frame: &mut Frame, area: Rect, app: &App) {
    let filter = app.journal_filter.as_deref();
    let rows: Vec<ListItem> = app
        .journal
        .iter()
        .filter(|e| filter.is_none_or(|f| e.matches(f)))
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|e| {
            let colour = match e.level.as_str() {
                "error" => Color::Red,
                "warn" => Color::Yellow,
                "info" => Color::Gray,
                "debug" | "trace" => Color::DarkGray,
                _ => Color::Magenta,
            };
            // A traceback arrives as one JSON line with embedded newlines; the
            // strip shows the first line and the Lua error is the first line.
            let head = e.msg.lines().next().unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", e.clock()), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<5} ", e.level), Style::default().fg(colour)),
                Span::styled(
                    format!("{:<22} ", crate::ui::shorten(&e.source, 22)),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(head.to_string()),
            ]))
        })
        .collect();

    let title = match filter {
        Some(f) => format!(" journal  /{}  ", f),
        None => " journal ".to_string(),
    };
    frame.render_widget(
        List::new(rows).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let dot = |link: &Link| match link {
        Link::Up => Style::default().fg(Color::Green),
        Link::Connecting => Style::default().fg(Color::Yellow),
        Link::Down(_) => Style::default().fg(Color::Red),
    };

    let mut spans = vec![
        Span::styled(" telnet ", Style::default().fg(Color::DarkGray)),
        Span::styled(app.telnet.label(), dot(&app.telnet)),
        Span::styled("  dap ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            if app.dbg.world_frozen {
                "frozen".to_string()
            } else if app.dbg.stopped {
                // One dispatch is held and the game is still running. Saying
                // "stopped" here is what made a live server look dead.
                "suspended".to_string()
            } else if app.dbg.attached {
                "attached".to_string()
            } else {
                app.dap.label()
            },
            if app.dbg.stopped {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                dot(&app.dap)
            },
        ),
    ];

    if app.dbg.attached {
        // Worth saying out loud: an attached client forces LuaJIT onto the
        // interpreter, so "everything is slow" is expected, not a bug.
        spans.push(Span::styled(
            "  JIT off while attached",
            Style::default().fg(Color::DarkGray),
        ));
    }

    spans.push(Span::styled(
        "   ^Q quit  ^J journal  F1-F4 tabs",
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Trim a path-like string from the left, so the filename stays visible.
pub fn shorten(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let tail: String = s
        .chars()
        .skip(s.chars().count() - width.saturating_sub(1))
        .collect();
    format!("…{}", tail)
}

/// A bordered block whose title brightens when the pane has focus.
pub fn pane(title: &str, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(Span::styled(format!(" {} ", title), style))
}
