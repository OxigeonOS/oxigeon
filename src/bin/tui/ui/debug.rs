//! The Debug tab: files, source with a breakpoint gutter, the call stack, the
//! variables tree, and a REPL over `evaluate`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::dap::Focus;
use crate::ui::{pane, shorten};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(7)])
        .split(area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(30),
            Constraint::Min(30),
            Constraint::Length(38),
        ])
        .split(rows[0]);

    draw_files(frame, cols[0], app);
    draw_source(frame, cols[1], app);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(cols[2]);
    draw_stack(frame, right[0], app);
    draw_vars(frame, right[1], app);

    draw_repl(frame, rows[1], app);
}

fn draw_files(frame: &mut Frame, area: Rect, app: &App) {
    let dbg = &app.dbg;
    let height = area.height.saturating_sub(2) as usize;
    let start = dbg.file_sel.saturating_sub(height / 2);

    let items: Vec<ListItem> = dbg
        .files
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, path)| {
            let marked = dbg
                .breakpoints
                .get(path)
                .is_some_and(|lines| !lines.is_empty());
            let name = path.to_string_lossy().replace('\\', "/");
            let style = if i == dbg.file_sel {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    if marked { "● " } else { "  " },
                    Style::default().fg(Color::Red),
                ),
                Span::styled(shorten(&name, 26), style),
            ]))
        })
        .collect();

    frame.render_widget(
        List::new(items).block(pane("files", dbg.focus == Focus::Files)),
        area,
    );
}

fn draw_source(frame: &mut Frame, area: Rect, app: &App) {
    let dbg = &app.dbg;
    let height = area.height.saturating_sub(2) as usize;
    let start = dbg.cursor.saturating_sub(height / 2);

    // The line the VM is actually stopped on, if it is in this file.
    let stopped_line = dbg
        .frames
        .get(dbg.frame_sel)
        .filter(|f| f.path.as_deref().map(std::path::Path::new) == dbg.open.as_deref())
        .map(|f| f.line as usize);

    let marks = dbg.open.as_ref().and_then(|p| dbg.breakpoints.get(p));

    let lines: Vec<Line> = dbg
        .source
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, text)| {
            let n = i + 1;
            let has_bp = marks.is_some_and(|m| m.contains(&(n as u32)));
            let is_stop = stopped_line == Some(n);
            let is_cursor = i == dbg.cursor;

            let row_style = if is_stop {
                Style::default().bg(Color::Rgb(60, 50, 0))
            } else if is_cursor {
                Style::default().bg(Color::Rgb(30, 30, 40))
            } else {
                Style::default()
            };

            Line::from(vec![
                Span::styled(
                    if has_bp { "●" } else { " " },
                    Style::default().fg(Color::Red),
                ),
                Span::styled(
                    if is_stop { "▶" } else { " " },
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(format!("{:>5} ", n), Style::default().fg(Color::DarkGray)),
                Span::styled(text.clone(), row_style),
            ])
            .style(row_style)
        })
        .collect();

    let title = match &dbg.open {
        Some(p) => shorten(&p.to_string_lossy().replace('\\', "/"), 40),
        None => "source — pick a file (Tab to focus, Enter to open)".into(),
    };

    frame.render_widget(
        Paragraph::new(lines).block(pane(&title, dbg.focus == Focus::Source)),
        area,
    );
}

fn draw_stack(frame: &mut Frame, area: Rect, app: &App) {
    let dbg = &app.dbg;
    let items: Vec<ListItem> = if dbg.frames.is_empty() {
        vec![ListItem::new(Span::styled(
            if dbg.attached {
                "running — F9 sets a breakpoint"
            } else {
                "not attached"
            },
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        dbg.frames
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let where_ = f
                    .path
                    .as_deref()
                    .map(oxigeon::core::scripting::debugger::paths::short)
                    // A frame with no path is a C function — an efun, or the
                    // pcall every command is dispatched through.
                    .unwrap_or_else(|| "[C]".to_string());
                let style = if i == dbg.frame_sel {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<16}", shorten(&f.name, 16)), style),
                    Span::styled(
                        format!(" {}:{}", where_, f.line),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items).block(pane("stack", dbg.focus == Focus::Stack)),
        area,
    );
}

fn draw_vars(frame: &mut Frame, area: Rect, app: &App) {
    let dbg = &app.dbg;
    let height = area.height.saturating_sub(2) as usize;
    let start = dbg.var_sel.saturating_sub(height / 2);

    let items: Vec<ListItem> = if dbg.vars.is_empty() {
        let hint = if !dbg.stopped {
            "variables need a paused frame"
        } else {
            "no locals in this frame"
        };
        vec![ListItem::new(Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        dbg.vars
            .iter()
            .enumerate()
            .skip(start)
            .take(height)
            .map(|(i, v)| {
                let marker = if v.var_ref > 0 {
                    if v.expanded {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                };
                let style = if i == dbg.var_sel {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![
                    Span::raw("  ".repeat(v.depth)),
                    Span::styled(marker, Style::default().fg(Color::DarkGray)),
                    Span::styled(v.name.clone(), style),
                    Span::styled(
                        format!(" {}", v.ty),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!(" {}", v.value),
                        Style::default().fg(Color::Green),
                    ),
                ]))
            })
            .collect()
    };

    frame.render_widget(
        List::new(items).block(pane("variables", dbg.focus == Focus::Vars)),
        area,
    );
}

fn draw_repl(frame: &mut Frame, area: Rect, app: &App) {
    let dbg = &app.dbg;
    let height = area.height.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // Breakpoint conditions that raised come through as `output` events. A
    // condition that silently never fires is indistinguishable from a broken
    // breakpoint, so the adapter stops anyway and says why — show it.
    for text in dbg.output.iter().rev().take(2).collect::<Vec<_>>().into_iter().rev() {
        lines.push(Line::from(Span::styled(
            format!("⚠ {}", text),
            Style::default().fg(Color::Yellow),
        )));
    }

    let room = height.saturating_sub(lines.len() + 1);
    for (expr, result) in dbg.repl_log.iter().rev().take(room).collect::<Vec<_>>().into_iter().rev() {
        lines.push(Line::from(vec![
            Span::styled("› ", Style::default().fg(Color::DarkGray)),
            Span::raw(expr.clone()),
            Span::styled("  ⇒ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                result.clone(),
                Style::default().fg(if result.starts_with('!') {
                    Color::Red
                } else {
                    Color::Green
                }),
            ),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled("› ", Style::default().fg(Color::Cyan)),
        Span::raw(dbg.repl_input.clone()),
        Span::styled("▎", Style::default().fg(Color::Cyan)),
    ]));

    let title = if dbg.stopped {
        "repl · F5 continue  F10 over  F11 into  ⇧F11 out  F9 breakpoint"
    } else {
        "repl · ^P pause  F9 breakpoint  (evaluate needs a paused frame)"
    };

    frame.render_widget(
        Paragraph::new(lines).block(pane(title, dbg.focus == Focus::Repl)),
        area,
    );
}
