//! Rendering. Pure: takes `&State`, paints one frame.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table,
    TableState, Wrap,
};
use ratatui::Frame;

use eeeemail_domain::{Message, Thread};

use crate::state::{Focus, FolderRow, ListMode, Overlay, State};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const WARN: Color = Color::Yellow;
const BAD: Color = Color::Red;

pub fn draw(f: &mut Frame, st: &mut State) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(f.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Min(20)])
        .split(chunks[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(body[1]);

    draw_folders(f, body[0], st);
    draw_list(f, right[0], st);
    draw_preview(f, right[1], st);
    draw_status(f, chunks[1], st);

    match &st.overlay {
        Overlay::None => {}
        Overlay::Help => draw_help(f),
        Overlay::Search(q) => draw_search(f, q),
        Overlay::Compose(form) => draw_compose(f, form, st.busy),
    }
}

// --- panes --------------------------------------------------------------

fn draw_folders(f: &mut Frame, area: Rect, st: &State) {
    let focused = st.focus == Focus::Folders;

    let items: Vec<ListItem> = st
        .rows
        .iter()
        .map(|row| {
            let line = match row {
                FolderRow::AllEmail => {
                    let name = "✉ All Email";
                    if st.all_unread > 0 {
                        // Same weight as an unread message row: bold, white.
                        Line::from(vec![
                            Span::styled(
                                name,
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                st.all_unread.to_string(),
                                Style::default().fg(WARN),
                            ),
                        ])
                    } else {
                        Line::from(Span::raw(name))
                    }
                }
                FolderRow::Account(ai) => {
                    let name = st
                        .app
                        .account_configs()
                        .get(*ai)
                        .map(|c| c.alias())
                        .unwrap_or("?");
                    Line::from(Span::styled(
                        name,
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ))
                }
                FolderRow::Folder(ai, fi) => {
                    let Some(folder) = st.folders_by_account.get(*ai).and_then(|v| v.get(*fi))
                    else {
                        return ListItem::new("");
                    };
                    let name = format!("  {}", folder.name);
                    if folder.unread_count > 0 {
                        Line::from(vec![
                            Span::styled(
                                name,
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                folder.unread_count.to_string(),
                                Style::default().fg(WARN),
                            ),
                        ])
                    } else {
                        Line::from(Span::raw(name))
                    }
                }
            };
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(pane_block(" Folders ", focused))
        .highlight_style(
            Style::default()
                .bg(if focused { ACCENT } else { Color::DarkGray })
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    if !st.rows.is_empty() {
        state.select(Some(st.row_idx));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_list(f: &mut Frame, area: Rect, st: &State) {
    let focused = st.focus == Focus::List;

    if st.list_mode == ListMode::Threads {
        if let Some(open) = &st.open_thread {
            draw_open_thread(f, area, st, open);
        } else {
            draw_thread_list(f, area, st);
        }
        return;
    }

    let title = match (&st.query, st.selected_row()) {
        (Some(q), _) => format!(" Search: {q} "),
        (None, Some(FolderRow::AllEmail)) => " All Email ".to_string(),
        (None, Some(FolderRow::Folder(ai, fi))) => st
            .folders_by_account
            .get(*ai)
            .and_then(|v| v.get(*fi))
            .map(|f| format!(" {} ", f.name))
            .unwrap_or_else(|| " Mail ".to_string()),
        _ => " Mail ".to_string(),
    };

    // The merged views mix accounts, so each row says which one it is from.
    let spans = st.spans_accounts();

    let rows: Vec<Row> = st
        .messages
        .iter()
        .map(|m| {
            let style = if m.is_read {
                Style::default().fg(DIM)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            let mut cells = vec![
                flags_cell(m),
                Span::styled(format_date(m.sort_ts()), style),
            ];
            if spans {
                cells.push(Span::styled(
                    st.account_label(&m.account_id).to_string(),
                    Style::default().fg(DIM),
                ));
            }
            cells.push(Span::styled(
                truncate(&m.sender().display(), 22),
                style,
            ));
            cells.push(Span::styled(
                if m.subject.is_empty() {
                    "(no subject)".to_string()
                } else {
                    m.subject.clone()
                },
                style,
            ));
            Row::new(cells)
        })
        .collect();

    let mut header_cells: Vec<&str> = vec!["", "DATE"];
    if spans {
        header_cells.push("ACCOUNT");
    }
    header_cells.extend_from_slice(&["FROM", "SUBJECT"]);
    let header = Row::new(header_cells).style(Style::default().fg(DIM));

    let mut constraints = vec![Constraint::Length(3), Constraint::Length(12)];
    if spans {
        constraints.push(Constraint::Length(10));
    }
    constraints.push(Constraint::Length(24));
    constraints.push(Constraint::Min(12));

    let table = Table::new(rows, constraints)
    .header(header)
    .block(pane_block(&title, focused))
    .row_highlight_style(
        Style::default()
            .bg(if focused { ACCENT } else { Color::DarkGray })
            .fg(Color::Black),
    )
    .highlight_symbol("▸ ");

    let mut state = TableState::default();
    if !st.messages.is_empty() {
        state.select(Some(st.msg_idx));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_thread_list(f: &mut Frame, area: Rect, st: &State) {
    let focused = st.focus == Focus::List;
    let spans = st.spans_accounts();

    let rows: Vec<Row> = st
        .threads
        .iter()
        .map(|t| {
            let style = if t.unread_count > 0 {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM)
            };
            let counts = if t.unread_count > 0 {
                Span::styled(
                    format!("{} ({})", t.message_count, t.unread_count),
                    Style::default().fg(WARN),
                )
            } else {
                Span::styled(t.message_count.to_string(), Style::default().fg(DIM))
            };
            let mut cells = vec![
                thread_flags_cell(t),
                Span::styled(format_date(t.last_message_at), style),
            ];
            if spans {
                cells.push(Span::styled(
                    st.account_label(&t.account_id).to_string(),
                    Style::default().fg(DIM),
                ));
            }
            cells.push(Span::styled(
                if t.subject.is_empty() {
                    "(no subject)".to_string()
                } else {
                    t.subject.clone()
                },
                style,
            ));
            cells.push(counts);
            Row::new(cells)
        })
        .collect();

    let mut header_cells: Vec<&str> = vec!["", "LAST"];
    if spans {
        header_cells.push("ACCOUNT");
    }
    header_cells.extend_from_slice(&["SUBJECT", "MSGS"]);
    let header = Row::new(header_cells).style(Style::default().fg(DIM));

    let mut constraints = vec![Constraint::Length(3), Constraint::Length(12)];
    if spans {
        constraints.push(Constraint::Length(10));
    }
    constraints.push(Constraint::Min(12));
    constraints.push(Constraint::Length(12));

    let table = Table::new(rows, constraints)
    .header(header)
    .block(pane_block(" Conversations — Enter to open ", focused))
    .row_highlight_style(
        Style::default()
            .bg(if focused { ACCENT } else { Color::DarkGray })
            .fg(Color::Black),
    )
    .highlight_symbol("▸ ");

    let mut state = TableState::default();
    if !st.threads.is_empty() {
        state.select(Some(st.thread_idx));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_open_thread(f: &mut Frame, area: Rect, st: &State, open: &crate::state::OpenThread) {
    let focused = st.focus == Focus::List;
    let title = format!(
        " {} — Esc back ",
        truncate(
            if open.thread.subject.is_empty() {
                "(no subject)"
            } else {
                &open.thread.subject
            },
            40
        )
    );

    let rows: Vec<Row> = open
        .messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let style = if m.is_read {
                Style::default().fg(DIM)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            let position = Span::styled(
                format!("#{:<2}", i + 1),
                if m.is_read {
                    Style::default().fg(DIM)
                } else {
                    Style::default().fg(WARN)
                },
            );
            Row::new(vec![
                position,
                Span::styled(format_date(m.sort_ts()), style),
                Span::styled(truncate(m.sender().display(), 22), style),
                Span::styled(
                    if m.subject.is_empty() {
                        "(no subject)".to_string()
                    } else {
                        m.subject.clone()
                    },
                    style,
                ),
            ])
        })
        .collect();

    let header = Row::new(vec!["", "DATE", "FROM", "SUBJECT"]).style(Style::default().fg(DIM));

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(12),
            Constraint::Length(24),
            Constraint::Min(12),
        ],
    )
    .header(header)
    .block(pane_block(&title, focused))
    .row_highlight_style(
        Style::default()
            .bg(if focused { ACCENT } else { Color::DarkGray })
            .fg(Color::Black),
    )
    .highlight_symbol("▸ ");

    let mut state = TableState::default();
    if !open.messages.is_empty() {
        state.select(Some(open.msg_idx));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_preview(f: &mut Frame, area: Rect, st: &State) {
    let focused = st.focus == Focus::Preview;
    let block = pane_block(" Message ", focused);

    let Some(message) = st.selected_message() else {
        f.render_widget(
            Paragraph::new("No message selected.")
                .style(Style::default().fg(DIM))
                .block(block),
            area,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(field(
        "From",
        &eeeemail_domain::join_addresses(&message.from),
    ));
    lines.push(field("To", &eeeemail_domain::join_addresses(&message.to)));
    if !message.cc.is_empty() {
        lines.push(field("Cc", &eeeemail_domain::join_addresses(&message.cc)));
    }
    lines.push(field("Date", &format_date_long(message.sort_ts())));
    lines.push(field("Subject", &message.subject));
    if !message.attachments.is_empty() {
        let names = message
            .attachments
            .iter()
            .map(|a| a.display_name().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(field("Attach", &names));
    }
    lines.push(Line::from(""));

    for line in message.body_preferred().lines() {
        lines.push(Line::from(line.to_string()));
    }

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((st.scroll, 0));
    f.render_widget(paragraph, area);
}

fn draw_status(f: &mut Frame, area: Rect, st: &State) {
    let spinner = if st.busy {
        ["|", "/", "-", "\\"][st.tick % 4]
    } else {
        " "
    };

    let mut spans = vec![
        Span::styled(format!(" {spinner} "), Style::default().fg(ACCENT)),
        Span::styled(
            format!("[{}]", st.focus.label()),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    if st.app.account_configs().len() > 1 {
        spans.push(Span::styled(
            format!("{}/{} ", st.account_idx + 1, st.app.account_configs().len()),
            Style::default().fg(DIM),
        ));
    }

    match &st.status {
        Some((text, true)) => spans.push(Span::styled(text.clone(), Style::default().fg(BAD))),
        Some((text, false)) => spans.push(Span::styled(text.clone(), Style::default().fg(WARN))),
        None => spans.push(Span::styled(
            "q quit · f fetch · a account · t threads · c compose · r reply · / search · ? help",
            Style::default().fg(DIM),
        )),
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// --- overlays -----------------------------------------------------------

fn draw_help(f: &mut Frame) {
    let lines = vec![
        Line::from(Span::styled(
            "eeeemail",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        key("Tab", "switch pane (folders → list → message)"),
        key("j / k, ↑ / ↓", "move"),
        key("g / G", "first / last"),
        key("Enter", "open message (marks it read)"),
        key("t", "toggle conversations / flat list"),
        key("Esc", "back to the conversation list"),
        key("a / A", "previous / next account (the folder pane lists them all; All Email merges them)"),
        key("f", "fetch mail from the server"),
        key("c", "compose"),
        key("r", "reply to the highlighted message"),
        key("*", "toggle star"),
        key("u", "toggle read"),
        key("d", "delete"),
        key("/", "search the local database"),
        key("?", "this help"),
        key("q", "quit"),
        Line::from(""),
        Line::from(Span::styled(
            "Compose: Tab moves between fields, Esc cancels, Enter on [ Send ] sends.",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "Press any key to close.",
            Style::default().fg(DIM),
        )),
    ];

    let area = centered(64, lines.len() as u16 + 2, f.area());
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Keys "),
        ),
        area,
    );
}

fn draw_search(f: &mut Frame, query: &str) {
    let area = Rect {
        height: 3,
        ..centered(60, 3, f.area())
    };
    let y = f.area().height.saturating_sub(3);
    let area = Rect { y, ..area };

    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(format!("/{query}")).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Search (Enter to run, Esc to cancel) ")
                .border_style(Style::default().fg(ACCENT)),
        ),
        area,
    );
    f.set_cursor_position((area.x + query.chars().count() as u16 + 2, area.y + 1));
}

fn draw_compose(f: &mut Frame, form: &crate::state::ComposeForm, busy: bool) {
    let area = centered(78, 16, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(if busy {
            " Compose — sending… "
        } else {
            " Compose "
        })
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // To
            Constraint::Length(3), // Subject
            Constraint::Min(4),    // Body
            Constraint::Length(1), // buttons
            Constraint::Length(1), // hint / error
        ])
        .split(inner);

    f.render_widget(input("To", &form.to, form.field == 0), rows[0]);
    f.render_widget(input("Subject", &form.subject, form.field == 1), rows[1]);
    f.render_widget(
        Paragraph::new(form.body.clone())
            .style(Style::default())
            .wrap(Wrap { trim: false })
            .block(input_block("Body", form.field == 2)),
        rows[2],
    );

    // Buttons: two side-by-side cells, the active one inverted.
    let send = Span::styled(
        " [ Send ] ",
        if form.field == 3 {
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        },
    );
    let cancel = Span::styled(
        " [ Cancel ] ",
        if form.field == 4 {
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        },
    );
    f.render_widget(Paragraph::new(Line::from(vec![send, cancel])), rows[3]);

    let footer: Line = match &form.error {
        Some(e) => Line::from(Span::styled(format!(" {e}"), Style::default().fg(BAD))),
        None => Line::from(Span::styled(
            " Tab next field · Enter newline in body · Esc cancel ",
            Style::default().fg(DIM),
        )),
    };
    f.render_widget(Paragraph::new(footer), rows[4]);

    // Caret for whichever text field is active.
    if let Some(len) = form.cursor_col() {
        let (x, y) = match form.field {
            0 => (
                inner.x + 1 + len.min(rows[0].width.saturating_sub(2)),
                rows[0].y + 1,
            ),
            1 => (
                inner.x + 1 + len.min(rows[1].width.saturating_sub(2)),
                rows[1].y + 1,
            ),
            _ => {
                // Body: the caret follows the last line of the text.
                let lines: Vec<&str> = form.body.split('\n').collect();
                let last = lines.last().copied().unwrap_or("");
                let row = lines.len().saturating_sub(1) as u16;
                let visible = rows[2].height.saturating_sub(2);
                let row = row.min(visible);
                (
                    inner.x + 1 + last.chars().count() as u16,
                    rows[2].y + 1 + row,
                )
            }
        };
        f.set_cursor_position((x, y));
    }
}

// --- helpers ------------------------------------------------------------

fn pane_block(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.to_string())
        .border_style(if focused {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(DIM)
        })
}

fn input_block(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.to_string())
        .border_style(if focused {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(DIM)
        })
}

fn input(title: &str, value: &str, focused: bool) -> Paragraph<'static> {
    Paragraph::new(value.to_string()).block(input_block(title, focused))
}

fn field(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<8}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        ),
        Span::raw(value.to_string()),
    ])
}

fn key(keys: &str, what: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{keys:<20}"), Style::default().fg(ACCENT)),
        Span::raw(what.to_string()),
    ])
}

fn flags_cell(m: &Message) -> Span<'static> {
    let mut s = String::new();
    if !m.is_read {
        s.push('●');
    } else {
        s.push(' ');
    }
    s.push(if m.is_starred { '*' } else { ' ' });
    let style = if !m.is_read {
        Style::default().fg(WARN)
    } else {
        Style::default().fg(DIM)
    };
    Span::styled(s, style)
}

fn thread_flags_cell(t: &Thread) -> Span<'static> {
    let style = if t.unread_count > 0 {
        Style::default().fg(WARN)
    } else {
        Style::default().fg(DIM)
    };
    Span::styled(if t.unread_count > 0 { "● " } else { "  " }, style)
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let cut: String = chars[..max.saturating_sub(1)].iter().collect();
    format!("{cut}…")
}

fn format_date(ts: i64) -> String {
    if ts == 0 {
        return " ".repeat(12);
    }
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| " ".repeat(12))
}

fn format_date_long(ts: i64) -> String {
    if ts == 0 {
        return "unknown".to_string();
    }
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M:%S %Z").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}
