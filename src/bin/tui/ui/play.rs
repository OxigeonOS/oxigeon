//! The Play tab: the game itself, plus the GMCP-fed side panels — and the
//! freeze-the-world banner, which is the reason this pane and the debugger are
//! in the same window.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::ui::pane;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(40), Constraint::Length(28)])
        .split(area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(cols[0]);

    draw_game(frame, rows[0], app);
    draw_input(frame, rows[1], app);
    draw_side(frame, cols[1], app);

    if app.dbg.stopped {
        draw_pause_banner(frame, rows[0], app);
    }
}

fn draw_game(frame: &mut Frame, area: Rect, app: &App) {
    let inner_height = area.height.saturating_sub(2) as usize;
    // One row is given to the prompt, which is the unterminated tail of output.
    let visible = inner_height.saturating_sub(1);

    let len = app.scrollback.len();
    let end = len.saturating_sub(app.scroll_offset);
    let start = end.saturating_sub(visible);

    let mut lines: Vec<Line> = app
        .scrollback
        .iter()
        .skip(start)
        .take(end - start)
        .cloned()
        .collect();

    // Only show the live prompt when pinned to the tail; halfway up the
    // scrollback it would be a confusing artefact from the future.
    if app.scroll_offset == 0 {
        if let Some(prompt) = &app.prompt {
            lines.push(prompt.clone());
        }
    }

    let title = if app.scroll_offset > 0 {
        format!(" game  ↑{} lines back ", app.scroll_offset)
    } else {
        " game ".to_string()
    };

    let style = if app.dbg.stopped {
        // Dimmed, because nothing here is live: the VM is stopped and this text
        // is a photograph of the moment it froze.
        Style::default().add_modifier(Modifier::DIM)
    } else {
        Style::default()
    };

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(style)
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    // While the server holds ECHO this is a password. Never render it, and note
    // why, so a masked line does not look like a frozen input box.
    let (text, title) = if app.masked {
        ("*".repeat(app.input.chars().count()), " password ")
    } else {
        (app.input.clone(), " input ")
    };

    let style = if app.masked {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::DarkGray)),
            Span::styled(text.clone(), style),
            Span::styled("▎", Style::default().fg(Color::Cyan)),
        ]))
        .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn draw_side(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Min(3),
        ])
        .split(area);

    draw_room(frame, rows[0], app);
    draw_vitals(frame, rows[1], app);
    draw_effects(frame, rows[2], app);
}

fn draw_room(frame: &mut Frame, area: Rect, app: &App) {
    let room = &app.room;
    let body = if room.name.is_empty() {
        vec![Line::from(Span::styled(
            "waiting for Room.Info",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        vec![
            Line::from(Span::styled(
                room.name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            // The dotted room id, not just the area: it is what you type into
            // `goto`, and what a room file is named after.
            Line::from(Span::styled(
                if room.id.is_empty() {
                    room.area.clone()
                } else {
                    room.id.clone()
                },
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![
                Span::styled("exits ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    if room.exits.is_empty() {
                        "none".to_string()
                    } else {
                        room.exits.join(" ")
                    },
                    Style::default().fg(Color::Cyan),
                ),
            ]),
        ]
    };
    frame.render_widget(
        Paragraph::new(body).block(pane("room", false)),
        area,
    );
}

fn draw_vitals(frame: &mut Frame, area: Rect, app: &App) {
    let v = &app.vitals;
    let width = area.width.saturating_sub(4).min(14) as usize;

    let mut body = vec![
        bar("hp", v.hp, v.maxhp, Color::Red, width),
        bar("mp", v.mp, v.maxmp, Color::Blue, width),
    ];
    body.push(Line::from(vec![
        Span::styled("lvl ", Style::default().fg(Color::DarkGray)),
        Span::raw(v.level.map(|n| n.to_string()).unwrap_or_else(|| "-".into())),
        Span::styled("  xp ", Style::default().fg(Color::DarkGray)),
        Span::raw(v.xp.map(|n| n.to_string()).unwrap_or_else(|| "-".into())),
        Span::styled("  gp ", Style::default().fg(Color::DarkGray)),
        Span::raw(v.gold.map(|n| n.to_string()).unwrap_or_else(|| "-".into())),
    ]));

    frame.render_widget(Paragraph::new(body).block(pane("vitals", false)), area);
}

fn bar(label: &str, cur: Option<i64>, max: Option<i64>, colour: Color, width: usize) -> Line<'static> {
    let (cur, max) = match (cur, max) {
        (Some(c), Some(m)) if m > 0 => (c, m),
        _ => {
            return Line::from(vec![
                Span::styled(format!("{:<3}", label), Style::default().fg(Color::DarkGray)),
                Span::styled("—", Style::default().fg(Color::DarkGray)),
            ])
        }
    };
    let filled = ((cur.max(0) as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    Line::from(vec![
        Span::styled(format!("{:<3}", label), Style::default().fg(Color::DarkGray)),
        Span::styled("█".repeat(filled), Style::default().fg(colour)),
        Span::styled(
            "░".repeat(width - filled),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(format!(" {}/{}", cur, max)),
    ])
}

fn draw_effects(frame: &mut Frame, area: Rect, app: &App) {
    let body: Vec<Line> = if app.effects.is_empty() {
        vec![Line::from(Span::styled(
            "none",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.effects
            .iter()
            .map(|e| {
                // gmcp_d sends -1 for an effect with no expiry.
                let left = if e.remaining < 0 {
                    "∞".to_string()
                } else {
                    format!("{}s", e.remaining)
                };
                let stacks = if e.stacks > 1 {
                    format!(" x{}", e.stacks)
                } else {
                    String::new()
                };
                Line::from(vec![
                    Span::raw(format!("{}{}", e.label, stacks)),
                    Span::styled(
                        format!(" {}", left),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(body).block(pane("effects", false)), area);
}

/// The whole point of the cockpit: make freeze-the-world visible.
///
/// A breakpoint stops the entire Lua VM, so every player on the server — you
/// included — is frozen until the debugger resumes. From an editor that is
/// invisible; here it is a banner over the game you were just playing, counting
/// down the adapter's own `auto_continue_secs` safety valve.
fn draw_pause_banner(frame: &mut Frame, area: Rect, app: &App) {
    let width = area.width.saturating_sub(4).min(56);
    let height = 7u16.min(area.height);
    let box_area = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    let countdown = match app.dbg.auto_continue_in() {
        Some(secs) => format!("auto-continue in {}:{:02}", secs / 60, secs % 60),
        None => "auto-continue disabled".to_string(),
    };

    let where_ = app
        .dbg
        .frames
        .first()
        .map(|f| {
            format!(
                "{}:{}",
                f.path
                    .as_deref()
                    .map(oxigeon::core::scripting::debugger::paths::short)
                    .unwrap_or_else(|| f.name.clone()),
                f.line
            )
        })
        .unwrap_or_default();

    let body = vec![
        Line::from(Span::styled(
            "⏸  VM PAUSED",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(
            format!("{} at {}", app.dbg.stop_reason, where_),
            Style::default().fg(Color::Yellow),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            "every player on this server is frozen",
            Style::default().fg(Color::Gray),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            countdown,
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center),
    ];

    frame.render_widget(Clear, box_area);
    frame.render_widget(
        Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    " F5 continue · F2 debug ",
                    Style::default().fg(Color::Yellow),
                )),
        ),
        box_area,
    );
}
