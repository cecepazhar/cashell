//! `cashell chat` — Dual-panel TUI (Terminal + AI Chat) with Mouse & Keyboard support.
//!
//! Uses ratatui + crossterm. Handles `Event::Mouse` for wheel scrolling and clicking.

use crate::config::Config;
use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::io;

#[derive(Parser, Debug, Clone)]
pub struct ChatCommand;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Focus {
    Input,
    Chat,
}

pub struct ChatApp {
    focus: Focus,
    chat_lines: Vec<String>,
    input: String,
    is_processing: bool,
    chat_state: ListState,
    stat_line: String,
}

impl ChatApp {
    fn new() -> Self {
        let mut chat_state = ListState::default();
        chat_state.select(Some(0));
        let snap = crate::systat::snapshot();
        let stat_line = crate::systat::status_bar_text(&snap);
        Self {
            focus: Focus::Input,
            chat_lines: vec![
                "=== Hana AI Chat (localhost:20128) ===".to_string(),
                "Tab: Switch Focus | Mouse Wheel: Scroll | Esc: Quit".to_string(),
                "Type your prompt in the left terminal pane and press Enter.".to_string(),
            ],
            input: String::new(),
            is_processing: false,
            chat_state,
            stat_line,
        }
    }

    fn add_chat_line(&mut self, line: &str) {
        self.chat_lines.push(line.to_string());
        let idx = self.chat_lines.len().saturating_sub(1);
        self.chat_state.select(Some(idx));
    }

    fn scroll_up(&mut self) {
        let i = match self.chat_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.chat_state.select(Some(i));
    }

    fn scroll_down(&mut self) {
        let len = self.chat_lines.len();
        if len == 0 {
            return;
        }
        let i = match self.chat_state.selected() {
            Some(i) if i + 1 < len => i + 1,
            _ => len.saturating_sub(1),
        };
        self.chat_state.select(Some(i));
    }

    fn refresh_stats(&mut self) {
        let snap = crate::systat::snapshot();
        self.stat_line = crate::systat::status_bar_text(&snap);
    }
}

pub fn run(_cmd: &ChatCommand, cfg: &Config) -> Result<()> {
    if !crate::output::is_tty() {
        anyhow::bail!("Chat TUI requires an interactive terminal");
    }

    let mut app = ChatApp::new();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = event_loop(&mut terminal, &mut app, cfg);

    execute!(terminal.backend_mut(), event::DisableMouseCapture)?;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut ChatApp,
    cfg: &Config,
) -> Result<()> {
    let mut last_stats_update = std::time::Instant::now();
    loop {
        // Refresh system stats every 2s
        if last_stats_update.elapsed() >= std::time::Duration::from_secs(2) {
            app.refresh_stats();
            last_stats_update = std::time::Instant::now();
        }

        terminal.draw(|f| draw(f, app))?;

        let event = event::read()?;
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match key.code {
                    KeyCode::Esc => {
                        return Ok(());
                    }
                    KeyCode::Tab => {
                        app.focus = if app.focus == Focus::Input {
                            Focus::Chat
                        } else {
                            Focus::Input
                        };
                    }
                    KeyCode::Char(c) => {
                        if app.focus == Focus::Input {
                            app.input.push(c);
                        } else {
                            match c {
                                'j' | ' ' => app.scroll_down(),
                                'k' => app.scroll_up(),
                                'g' => {
                                    app.chat_state.select(Some(0));
                                }
                                'G' => {
                                    let last = app.chat_lines.len().saturating_sub(1);
                                    app.chat_state.select(Some(last));
                                }
                                _ => {}
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        if app.focus == Focus::Input {
                            app.input.pop();
                        }
                    }
                    KeyCode::Enter => {
                        if app.focus == Focus::Input && !app.input.trim().is_empty() && !app.is_processing {
                            let prompt = app.input.clone();
                            app.input.clear();
                            app.add_chat_line(&format!("User: {prompt}"));
                            app.is_processing = true;

                            // Redraw so the user sees their query immediately
                            terminal.draw(|f| draw(f, app))?;

                            // Query Hana AI
                            let cfg_clone = cfg.clone();
                            let answer = match crate::ai::query_buffered(&cfg_clone, None, &prompt) {
                                Ok(ans) => ans,
                                Err(e) => format!("Error: {e}"),
                            };

                            for line in answer.lines() {
                                app.add_chat_line(&format!("AI: {line}"));
                            }
                            app.is_processing = false;
                        }
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    app.scroll_up();
                }
                MouseEventKind::ScrollDown => {
                    app.scroll_down();
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    // Clicking switches active panel focus
                    app.focus = if app.focus == Focus::Input {
                        Focus::Chat
                    } else {
                        Focus::Input
                    };
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn draw(f: &mut Frame, app: &ChatApp) {
    let size = f.area();

    // Vertical layout: Main area (split left/right) + Bottom Telemetry Taskbar
    let vert_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(size);

    let main_chunk = vert_chunks[0];
    let status_chunk = vert_chunks[1];

    // Split layout: 2 columns (Terminal / Input on Left, AI Chat on Right)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(main_chunk);

    let input_chunk = chunks[0];
    let chat_chunk = chunks[1];

    // Left: Terminal Panel
    let term_border_color = if app.focus == Focus::Input {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let term_block = Block::default()
        .borders(Borders::ALL)
        .title(" Terminal / Input [Focus] ")
        .border_style(Style::default().fg(term_border_color));
    let term_inner = term_block.inner(input_chunk);
    f.render_widget(term_block, input_chunk);

    let term_lines = vec![
        Line::from(Span::styled(
            format!("$ {}", app.input),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            if app.is_processing {
                "⏳ Thinking / querying Hana AI (localhost:20128)..."
            } else {
                "Type prompt or command here and press [Enter]."
            },
            Style::default().fg(if app.is_processing { Color::LightRed } else { Color::Green }),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Shortcuts:",
            Style::default().fg(Color::Gray).add_modifier(Modifier::UNDERLINED),
        )),
        Line::from(Span::styled("• Tab / Click: Switch Panel", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("• Mouse Scroll: Scroll Chat History", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("• Esc: Quit Chat", Style::default().fg(Color::DarkGray))),
    ];
    let term_paragraph = Paragraph::new(term_lines);
    f.render_widget(term_paragraph, term_inner);

    // Right: AI Chat History Panel
    let chat_border_color = if app.focus == Focus::Chat {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let chat_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Hana AI Chat History ({} items) [Mouse Scroll Enabled] ",
            app.chat_lines.len()
        ))
        .border_style(Style::default().fg(chat_border_color));
    let chat_inner = chat_block.inner(chat_chunk);
    f.render_widget(chat_block, chat_chunk);

    let items: Vec<ListItem> = app
        .chat_lines
        .iter()
        .map(|l| {
            let style = if l.starts_with("User:") {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else if l.starts_with("AI:") {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Line::from(Span::styled(l.clone(), style)))
        })
        .collect();

    let chat_list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(30, 40, 60))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(chat_list, chat_inner, &mut app.chat_state.clone());

    // Bottom Taskbar: Telemetry Widget
    let status_block = Block::default()
        .borders(Borders::ALL)
        .title(" System Telemetry (Auto-refresh 2s) ")
        .border_style(Style::default().fg(Color::Green));
    let status_inner = status_block.inner(status_chunk);
    f.render_widget(status_block, status_chunk);

    let status_para = Paragraph::new(Line::from(Span::styled(
        &app.stat_line,
        Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
    )));
    f.render_widget(status_para, status_inner);
}
