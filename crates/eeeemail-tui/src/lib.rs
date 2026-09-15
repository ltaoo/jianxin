//! # eeeemail-tui
//!
//! The interactive client: folders on the left, messages on the right, the
//! message itself underneath. Two keys matter most — `f` fetches mail and `c`
//! composes it.
//!
//! Everything is offline-first: panes are painted from the local SQLite copy
//! and network work happens on a worker thread, so a slow IMAP server shows a
//! spinner instead of a frozen screen.

mod state;
mod ui;

use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

pub use state::{ComposeForm, Focus, ListMode, OpenThread, Overlay, State};

/// Take over the terminal until the user quits.
pub fn run(app: eeeemail_core::App) -> Result<()> {
    let app = Arc::new(app);
    let mut state = State::new(Arc::clone(&app))?;

    // First run: nothing is in the database yet, so fetch immediately rather
    // than showing an empty window.
    if !state.has_any_folders() {
        state.start_sync();
    }

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut state);

    // Always put the terminal back, even on the error path — a half-restored
    // terminal is the one bug users remember.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, st: &mut State) -> Result<()> {
    while !st.quit {
        st.poll();
        st.tick = st.tick.wrapping_add(1);

        terminal.draw(|f| ui::draw(f, st))?;

        // Redraw at ~6 fps while idle so a worker finishing is visible quickly.
        if !event::poll(Duration::from_millis(160))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                handle_key(st, key)?;
            }
        }
    }
    Ok(())
}

fn handle_key(st: &mut State, key: KeyEvent) -> Result<()> {
    // Ctrl-C means quit everywhere, including inside an overlay.
    if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
        st.quit = true;
        return Ok(());
    }

    match st.overlay {
        Overlay::Help => st.overlay = Overlay::None,
        Overlay::Search(_) => handle_search_key(st, key),
        Overlay::Compose(_) => handle_compose_key(st, key),
        Overlay::None => handle_normal_key(st, key),
    }
    Ok(())
}

fn handle_normal_key(st: &mut State, key: KeyEvent) {
    let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));

    match key.code {
        KeyCode::Char('q') => st.quit = true,

        KeyCode::Esc => {
            if st.open_thread.is_some() {
                st.close_thread();
            } else {
                st.clear_query();
                st.status = None;
            }
        }

        KeyCode::Tab => st.focus = st.focus.next(),
        KeyCode::BackTab => st.focus = st.focus.next().next(),

        KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
            let delta = if down { 1 } else { -1 };
            match st.focus {
                Focus::Folders => st.move_folder(delta),
                Focus::List => st.move_message(delta),
                Focus::Preview => scroll(st, delta),
            }
        }

        KeyCode::Char('g') => st.jump(false),
        KeyCode::Char('G') => st.jump(true),
        KeyCode::PageDown => scroll(st, 10),
        KeyCode::PageUp => scroll(st, -10),

        KeyCode::Enter => match st.focus {
            Focus::Folders => {
                st.reload_messages();
                st.focus = Focus::List;
            }
            Focus::List => {
                // In the conversation view, Enter first drills into the
                // highlighted conversation; inside one it opens the message.
                if st.list_mode == ListMode::Threads && st.open_thread.is_none() {
                    st.open_selected_thread();
                } else {
                    st.open_message();
                }
            }
            Focus::Preview => {}
        },

        // Fetch mail: the whole point of having a client.
        KeyCode::Char('f') | KeyCode::F(5) => st.start_sync(),

        // Toggle between the flat message list and conversations.
        KeyCode::Char('t') => st.toggle_list_mode(),

        KeyCode::Char('a') => {
            st.cycle_account(-1);
        }
        KeyCode::Char('A') => {
            st.cycle_account(1);
        }

        KeyCode::Char('c') | KeyCode::Char('m') => {
            st.overlay = Overlay::Compose(ComposeForm::default());
        }
        KeyCode::Char('r') => {
            let form = st.reply_form();
            st.overlay = Overlay::Compose(form);
        }

        KeyCode::Char('*') | KeyCode::Char('s') => st.toggle_star(),
        KeyCode::Char('u') => st.toggle_read(),
        KeyCode::Char('d') => st.delete_selected(),

        KeyCode::Char('/') => st.overlay = Overlay::Search(String::new()),
        KeyCode::Char('?') => st.overlay = Overlay::Help,

        _ => {}
    }
}

fn scroll(st: &mut State, delta: isize) {
    st.scroll = if delta < 0 {
        st.scroll.saturating_sub(delta.unsigned_abs() as u16)
    } else {
        st.scroll.saturating_add(delta as u16)
    };
}

fn handle_search_key(st: &mut State, key: KeyEvent) {
    let Overlay::Search(query) = &mut st.overlay else {
        return;
    };
    match key.code {
        KeyCode::Esc => st.overlay = Overlay::None,
        KeyCode::Enter => {
            let q = query.clone();
            st.overlay = Overlay::None;
            st.set_query(q);
        }
        KeyCode::Backspace => {
            query.pop();
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            query.push(c);
        }
        _ => {}
    }
}

/// What a keypress inside the compose overlay asks us to do.
enum ComposeAction {
    Nothing,
    Send,
    Close,
}

fn handle_compose_key(st: &mut State, key: KeyEvent) {
    let action = {
        let Overlay::Compose(form) = &mut st.overlay else {
            return;
        };

        // Input is ignored while the message is in flight.
        if form.sending {
            return;
        }

        let ctrl_or_alt = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        match key.code {
            KeyCode::Esc => ComposeAction::Close,
            KeyCode::Tab => {
                form.field = (form.field + 1) % ComposeForm::FIELD_COUNT;
                ComposeAction::Nothing
            }
            KeyCode::BackTab => {
                form.field = (form.field + ComposeForm::FIELD_COUNT - 1) % ComposeForm::FIELD_COUNT;
                ComposeAction::Nothing
            }
            KeyCode::Enter => match form.field {
                // Single-line fields advance instead of inserting a newline.
                0 | 1 => {
                    form.field = (form.field + 1) % ComposeForm::FIELD_COUNT;
                    ComposeAction::Nothing
                }
                2 => {
                    form.body.push('\n');
                    ComposeAction::Nothing
                }
                3 => ComposeAction::Send,
                _ => ComposeAction::Close,
            },
            KeyCode::Backspace => {
                if let Some(text) = active_text(form) {
                    text.pop();
                }
                ComposeAction::Nothing
            }
            KeyCode::Char(c) if !ctrl_or_alt => {
                if let Some(text) = active_text(form) {
                    text.push(c);
                }
                ComposeAction::Nothing
            }
            _ => ComposeAction::Nothing,
        }
    };

    match action {
        ComposeAction::Nothing => {}
        ComposeAction::Send => st.send_compose(),
        ComposeAction::Close => st.overlay = Overlay::None,
    }
}

fn active_text(form: &mut ComposeForm) -> Option<&mut String> {
    match form.field {
        0 => Some(&mut form.to),
        1 => Some(&mut form.subject),
        2 => Some(&mut form.body),
        _ => None,
    }
}
