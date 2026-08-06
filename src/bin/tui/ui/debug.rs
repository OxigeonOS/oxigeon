//! The Debug tab: files, source with a breakpoint gutter, the call stack, the
//! variables tree, and a REPL over `evaluate`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::dap::{Focus, SourcePrompt};
use crate::lua_syntax::{self, Tok};
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

    // Focusing the variables pane gives it the middle column.
    //
    // A 38-column strip beside the source is enough to see *that* a local
    // exists and not much else — and reading values is most of what a debugger
    // is for. Tab moves focus on and the source comes back, so it costs one
    // keystroke each way and there is no mode to get stuck in.
    let zoomed = app.dbg.focus == Focus::Vars;
    if zoomed {
        draw_vars(frame, cols[1], app);
    } else {
        draw_source(frame, cols[1], app);
    }

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(cols[2]);
    draw_stack(frame, right[0], app);
    // The side pane shows whichever of the two is not in the middle, so the
    // file you were reading never disappears entirely.
    if zoomed {
        draw_source(frame, right[1], app);
    } else {
        draw_vars(frame, right[1], app);
    }

    draw_repl(frame, rows[1], app);
}

fn draw_files(frame: &mut Frame, area: Rect, app: &App) {
    let dbg = &app.dbg;
    let height = area.height.saturating_sub(2) as usize;
    let start = dbg.file_sel.saturating_sub(height / 2);

    let items: Vec<ListItem> = dbg
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, row)| {
            // A directory is marked if anything under it is. Otherwise closing a
            // folder would hide the fact that it holds a breakpoint, which is
            // exactly when you want to know. One predicate, shared with the
            // gutter, so the two cannot disagree about the same file.
            let marked = dbg.marked_under(&row.path, row.is_dir);
            let style = if i == dbg.file_sel {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else if row.is_dir {
                Style::default().fg(Color::Blue)
            } else {
                Style::default()
            };
            // Two spaces per level, and a disclosure arrow only where there is
            // something to disclose — a file aligned under its folder's arrow
            // reads as a child without needing a box-drawing glyph for it.
            let indent = "  ".repeat(row.depth);
            let arrow = if row.is_dir {
                if row.expanded { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            let width = 26usize.saturating_sub(indent.len() + 2);
            ListItem::new(Line::from(vec![
                Span::styled(
                    if marked { "●" } else { " " },
                    Style::default().fg(Color::Red),
                ),
                Span::styled(indent, Style::default()),
                Span::styled(arrow, Style::default().fg(Color::DarkGray)),
                Span::styled(shorten(&row.label(), width.max(6)), style),
            ]))
        })
        .collect();

    frame.render_widget(
        List::new(items).block(pane("files", dbg.focus == Focus::Files)),
        area,
    );
}

/// Colour for each kind of Lua token.
///
/// Deliberately restrained: this pane already spends colour on the breakpoint
/// gutter, the stopped line and search hits, and syntax that shouts drowns all
/// three. Comments recede, strings and structure separate, and everything else
/// is left alone.
fn token_style(tok: Tok, base: Style) -> Style {
    match tok {
        Tok::Plain => base,
        Tok::Keyword => base.fg(Color::Magenta),
        Tok::Literal => base.fg(Color::Rgb(200, 140, 60)),
        Tok::Str => base.fg(Color::Green),
        Tok::Comment => base.fg(Color::DarkGray),
        Tok::Ident => base.fg(Color::Cyan),
    }
}

/// Render one source line: Lua syntax underneath, search hits painted over it.
///
/// Search wins where they overlap. Highlighting the *term* rather than the line
/// is what makes a search feel like one — with only the cursor moved you cannot
/// see why it landed there, or how many hits share the line — and that has to
/// survive being drawn on top of a comment.
fn render_line(text: &str, block: Option<usize>, needle: &str, base: Style) -> Vec<Span<'static>> {
    let hit = base.bg(Color::Rgb(90, 80, 0)).fg(Color::White);

    // Offsets below come from the lowercased copy, so both must agree
    // byte-for-byte. `İ` lowercases to two chars and would slice mid-character.
    let (hay, need) = (text.to_lowercase(), needle.to_lowercase());
    let searching = !needle.is_empty() && hay.len() == text.len() && hay.contains(&need);

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut at = 0usize;
    for (run, tok) in lua_syntax::tokenize(text, block) {
        let style = token_style(tok, base);
        let end = at + run.len();
        if !searching {
            spans.push(Span::styled(run, style));
            at = end;
            continue;
        }
        // Split this run wherever a match falls inside it.
        let mut i = at;
        while i < end {
            match hay[i..end].find(&need) {
                Some(off) => {
                    let start = i + off;
                    if start > i {
                        spans.push(Span::styled(text[i..start].to_string(), style));
                    }
                    let stop = (start + need.len()).min(end);
                    spans.push(Span::styled(text[start..stop].to_string(), hit));
                    i = stop;
                }
                None => {
                    spans.push(Span::styled(text[i..end].to_string(), style));
                    break;
                }
            }
        }
        at = end;
    }
    spans
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
            // A logpoint is marked differently: it never stops, so a gutter
            // that showed it as a breakpoint would be promising something the
            // line will not do.
            let mark = marks.and_then(|m| m.get(&(n as u32)));
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
                    match mark {
                        Some(Some(_)) => "◆",
                        Some(None) => "●",
                        None => " ",
                    },
                    Style::default().fg(match mark {
                        Some(Some(_)) => Color::Cyan,
                        _ => Color::Red,
                    }),
                ),
                Span::styled(
                    if is_stop { "▶" } else { " " },
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(format!("{:>5} ", n), Style::default().fg(Color::DarkGray)),
            ]
            .into_iter()
            .chain(render_line(
                text,
                dbg.blocks.get(i).copied().flatten(),
                if dbg.highlight { &dbg.search } else { "" },
                row_style,
            ))
            .collect::<Vec<_>>())
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
                "running — F9 sets a breakpoint, ⇧F9 a logpoint"
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

    // Console output: logpoints reporting, and conditions that raised. The
    // warning glyph is for the second kind only — a logpoint doing exactly what
    // it was asked to do is not a warning, and dressing it as one is what made
    // the first working logpoint look broken.
    //
    // Most of the pane goes to these now. Two lines was right when the only
    // source was a condition that failed; a logpoint is a *stream*, and two
    // lines of it tells you nothing.
    let shown = height.saturating_sub(2).max(1);
    let skip = dbg.output.len().saturating_sub(shown);
    for (important, text) in dbg.output.iter().skip(skip) {
        lines.push(if *important {
            Line::from(Span::styled(
                format!("⚠ {text}"),
                Style::default().fg(Color::Yellow),
            ))
        } else {
            Line::from(vec![
                Span::styled("· ", Style::default().fg(Color::DarkGray)),
                Span::styled(text.clone(), Style::default().fg(Color::Green)),
            ])
        });
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

    // The editor takes over this row while it is open: it is the only place with
    // a cursor already, and a modal box over the source would hide the line the
    // message is about.
    let editing = dbg.logpoint_edit.as_ref();
    lines.push(match (editing, dbg.source_prompt.as_ref()) {
        (Some((line, text)), _) => Line::from(vec![
            Span::styled(format!("logpoint {line} › "), Style::default().fg(Color::Cyan)),
            Span::raw(text.clone()),
            Span::styled("▎", Style::default().fg(Color::Cyan)),
        ]),
        (None, Some(prompt)) => Line::from(vec![
            Span::styled(prompt.sigil().to_string(), Style::default().fg(Color::Yellow)),
            Span::raw(prompt.text().to_string()),
            Span::styled("▎", Style::default().fg(Color::Yellow)),
        ]),
        (None, None) => Line::from(vec![
            Span::styled("› ", Style::default().fg(Color::Cyan)),
            Span::raw(dbg.repl_input.clone()),
            Span::styled("▎", Style::default().fg(Color::Cyan)),
        ]),
    });

    // The steps advertise their Ctrl+arrow aliases, because F11 is full-screen
    // in most terminals and never arrives — telling someone to press a key that
    // resizes their window is worse than not telling them.
    let title = if editing.is_some() {
        "logpoint · {expr} is evaluated in the frame · enter set  esc cancel  empty removes"
    } else if let Some(prompt) = dbg.source_prompt.as_ref() {
        match prompt {
            SourcePrompt::Goto(_) => "go to line · a number, or :noh to clear the highlight",
            SourcePrompt::Search(_) => {
                "search · enter find  // repeat  n/N next/prev  esc cancel"
            }
        }
    } else if dbg.stopped {
        "repl · F5/^G go  F10/^→ over  F11/^↓ into  ⇧F11/^↑ out  F9 break  ⇧F9/^L logpoint"
    } else {
        "repl · ^P pause  F9 break  ⇧F9/^L logpoint  (evaluate needs a paused frame)"
    };

    frame.render_widget(
        Paragraph::new(lines).block(pane(title, dbg.focus == Focus::Repl)),
        area,
    );
}
