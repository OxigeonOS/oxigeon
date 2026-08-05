//! The Inspect tab: what a trait's number *actually* is, and what made it that.
//!
//! The variables pane next door can only show `entity.stats`, which for a
//! derived or buffed trait is the stored value and the wrong answer. This pane
//! reads through `DAEMON.trait.all` and `DAEMON.effect.active` instead, so
//! `max_hp` shows what the formula produces and a gauge shows its real ceiling.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::ui::pane;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(area);

    draw_target(frame, rows[0], app);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(40), Constraint::Length(36)])
        .split(rows[1]);

    draw_traits(frame, cols[0], app);
    draw_effects(frame, cols[1], app);
}

fn draw_target(frame: &mut Frame, area: Rect, app: &App) {
    let i = &app.dbg.inspect;
    let hint = if i.editing {
        "  (Enter to evaluate, Esc to cancel)"
    } else {
        "  e edit · r refresh"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("entity ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                i.target.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if i.editing { "▎" } else { "" },
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]))
        .block(pane("target — any Lua expression in the paused frame", i.editing)),
        area,
    );
}

fn draw_traits(frame: &mut Frame, area: Rect, app: &App) {
    let dbg = &app.dbg;
    let i = &dbg.inspect;

    // The states worth distinguishing: not attached, running, waiting, empty.
    if let Some(message) = unavailable(app) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                message,
                Style::default().fg(Color::DarkGray),
            )))
            .block(pane("traits", false)),
            area,
        );
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();
    let mut group = String::new();
    for (n, t) in i.traits.iter().enumerate() {
        if t.group != group {
            group = t.group.clone();
            items.push(ListItem::new(Line::from(Span::styled(
                format!("── {} ", if group.is_empty() { "other" } else { &group }),
                Style::default().fg(Color::DarkGray),
            ))));
        }

        let selected = n == i.selected;
        let name_style = if selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else if t.failed {
            // A broken definition answers with its default rather than taking
            // the server down; that is worth seeing, not hiding.
            Style::default().fg(Color::Red)
        } else {
            Style::default()
        };

        // The stored base and the effective value differing is the whole point:
        // it is the difference the raw table cannot show.
        let drifted = t.base != t.value && !t.base.is_empty();
        let mut spans = vec![
            Span::styled(format!("{:<16}", t.id), name_style),
            Span::styled(
                format!("{:>8}", t.value),
                Style::default()
                    .fg(if drifted { Color::Green } else { Color::Gray })
                    .add_modifier(if drifted {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
        ];
        if drifted {
            spans.push(Span::styled(
                format!("  base {}", t.base),
                Style::default().fg(Color::DarkGray),
            ));
        }
        if !t.max.is_empty() && t.max != "nil" {
            spans.push(Span::styled(
                format!("  /{}", t.max),
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(
            format!("  {}", t.kind),
            Style::default().fg(Color::DarkGray),
        ));
        // The human label, when it says more than the id already does.
        if !t.label.is_empty() && !t.label.eq_ignore_ascii_case(&t.id.replace('_', " ")) {
            spans.push(Span::styled(
                format!("  {}", t.label),
                Style::default().fg(Color::DarkGray),
            ));
        }
        items.push(ListItem::new(Line::from(spans)));
    }

    frame.render_widget(
        List::new(items).block(pane(
            &format!("traits ({}) — value, then stored base where they differ", i.traits.len()),
            false,
        )),
        area,
    );
}

fn draw_effects(frame: &mut Frame, area: Rect, app: &App) {
    let i = &app.dbg.inspect;
    let items: Vec<ListItem> = if i.effects.is_empty() {
        vec![ListItem::new(Span::styled(
            "no active effects",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        i.effects
            .iter()
            .map(|e| {
                let label = if e.label.is_empty() { &e.id } else { &e.label };
                let stacks = if e.stacks != "1" {
                    format!(" ×{}", e.stacks)
                } else {
                    String::new()
                };
                let expiry = if e.expires.is_empty() || e.expires == "nil" {
                    "permanent".to_string()
                } else {
                    format!("expires {}", e.expires)
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::raw(label.clone()),
                        Span::styled(stacks, Style::default().fg(Color::Cyan)),
                    ]),
                    Line::from(Span::styled(
                        format!("  {}  {}", e.id, expiry),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
            })
            .collect()
    };

    frame.render_widget(
        List::new(items).block(pane("effects — what is modifying the above", false)),
        area,
    );
}

/// Why the pane has nothing to show, in the user's terms.
fn unavailable(app: &App) -> Option<String> {
    let dbg = &app.dbg;
    if let Some(err) = &dbg.inspect.error {
        return Some(err.clone());
    }
    if !dbg.attached {
        return Some("not attached to the debug adapter".into());
    }
    if !dbg.stopped {
        // `evaluate` is rejected outright while the VM runs, so this pane
        // genuinely cannot work here. The Play tab's GMCP panels are the live
        // view of your own character.
        return Some(
            "set a breakpoint and trigger it — evaluate needs a paused frame".into(),
        );
    }
    if dbg.inspect.pending {
        return Some("evaluating…".into());
    }
    if dbg.inspect.traits.is_empty() {
        return Some("press r to read traits from the paused frame".into());
    }
    None
}
