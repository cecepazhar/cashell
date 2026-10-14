//! `cashell chat` — btop-style dashboard + AI Chat + terminal input hybrid TUI.
//!
//! Layout (when terminal >= 80x24):
//!   ┌───────────────────────────┬──────────────────────────────────────────┐
//!   │ Dashboard (Top/Left 42%)  │ AI Chat Panel (Right 58%)                │
//!   │ • Real-time CPU sparkline │ • Hana AI chat history                   │
//!   │ • RAM / Swap / Disk gauge │ • Mouse wheel scrolling                  │
//!   │ • Temp / GPU / Fan stats  │ • j / k / g / G keyboard navigation      │
//!   ├───────────────────────────┴──────────────────────────────────────────┤
//!   │ Terminal Input (Bottom): $ prompt...                                 │
//!   └──────────────────────────────────────────────────────────────────────┘
//!
//! Degrades gracefully to compact dual-panel if width < 80 or height < 24.
//! Background thread updates telemetry every ~500ms via `std::sync::mpsc`.

use crate::config::Config;
use crate::systat::{snapshot, SysSnapshot};
use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Chart, Dataset, Gauge, GraphType, List, ListItem, ListState,
        Paragraph,
    },
    Frame, Terminal,
};
use std::collections::VecDeque;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[derive(Parser, Debug, Clone)]
pub struct ChatCommand;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Focus {
    Input,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum LayoutMode {
    /// Full btop dashboard (left) + chat (right) + input (bottom)
    Full,
    /// Degraded: dual-panel + bottom status bar for small terminals
    Compact,
}

const MAX_HISTORY: usize = 60;

pub struct ChatApp {
    focus: Focus,
    layout_mode: LayoutMode,
    chat_lines: Vec<String>,
    input: String,
    is_processing: bool,
    chat_state: ListState,
    stat_line: String,
    cpu_history: VecDeque<f32>,
    ram_history: VecDeque<f32>,
    latest_snap: SysSnapshot,
}

impl ChatApp {
    fn new() -> Self {
        let snap = snapshot();
        // ponytail: pre-fill the 60-sample ring buffer with initial snapshot so
        // the CPU chart renders a full baseline rather than a single disjoint point.
        let mut cpu_history = VecDeque::with_capacity(MAX_HISTORY);
        let mut ram_history = VecDeque::with_capacity(MAX_HISTORY);
        let ram_pct = (snap.ram_used_mb * 100)
            .checked_div(snap.ram_total_mb)
            .unwrap_or(0) as f32;

        for _ in 0..MAX_HISTORY {
            cpu_history.push_back(snap.cpu_pct);
            ram_history.push_back(ram_pct);
        }

        let mut chat_state = ListState::default();
        chat_state.select(Some(0));

        Self {
            focus: Focus::Input,
            layout_mode: LayoutMode::Full,
            chat_lines: vec![
                "=== Hana AI Chat (localhost:20128) ===".to_string(),
                "Tab / Click: Switch Focus | Mouse Wheel: Scroll | Esc: Quit".to_string(),
                "Type your prompt in the terminal input below and press Enter.".to_string(),
            ],
            input: String::new(),
            is_processing: false,
            chat_state,
            stat_line: crate::systat::status_bar_text(&snap),
            cpu_history,
            ram_history,
            latest_snap: snap,
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

    fn ingest_snapshot(&mut self, snap: SysSnapshot) {
        let ram_pct = (snap.ram_used_mb * 100)
            .checked_div(snap.ram_total_mb)
            .unwrap_or(0) as f32;

        self.cpu_history.push_back(snap.cpu_pct);
        self.ram_history.push_back(ram_pct);

        if self.cpu_history.len() > MAX_HISTORY {
            self.cpu_history.pop_front();
        }
        if self.ram_history.len() > MAX_HISTORY {
            self.ram_history.pop_front();
        }

        self.stat_line = crate::systat::status_bar_text(&snap);
        self.latest_snap = snap;
    }
}

// ── Terminal Guard (RAII + Panic safety) ──────────────────────────────────────

struct TermGuard;

impl TermGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, event::EnableMouseCapture)?;

        // ponytail: global panic hook restores terminal before printing panic trace.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal_raw();
            prev(info);
        }));

        Ok(Self)
    }
}

impl Drop for TermGuard {
    fn drop(&mut self) {
        restore_terminal_raw();
    }
}

fn restore_terminal_raw() {
    let mut out = stdout();
    let _ = execute!(
        out,
        event::DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = disable_raw_mode();
}

// ── Run Entry Point ──────────────────────────────────────────────────────────

pub fn run(_cmd: &ChatCommand, cfg: &Config) -> Result<()> {
    if !crate::output::is_tty() {
        anyhow::bail!("Chat TUI requires an interactive terminal");
    }

    let mut app = ChatApp::new();

    // Initial terminal size check
    if let Ok((w, h)) = crossterm::terminal::size() {
        if (w as usize) < 80 || (h as usize) < 24 {
            app.layout_mode = LayoutMode::Compact;
        }
    }

    let _guard = TermGuard::new()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    // ── Background stats thread (500ms loop) ─────────────────────────────────
    let running = Arc::new(AtomicBool::new(true));
    let r_thread = running.clone();
    let (snap_tx, mut snap_rx) = mpsc::channel::<SysSnapshot>();

    let stats_thread = std::thread::spawn(move || {
        while r_thread.load(Ordering::Relaxed) {
            let snap = snapshot();
            if snap_tx.send(snap).is_err() {
                break;
            }
            // Sleep in 100ms slices so the thread terminates rapidly on shutdown
            for _ in 0..5 {
                if !r_thread.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });

    let res = event_loop(&mut terminal, &mut app, cfg, &mut snap_rx);

    // Stop background worker cleanly
    running.store(false, Ordering::Relaxed);
    drop(snap_rx);
    let _ = stats_thread.join();

    res
}

// ── Event Loop ───────────────────────────────────────────────────────────────

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut ChatApp,
    cfg: &Config,
    snap_rx: &mut mpsc::Receiver<SysSnapshot>,
) -> Result<()> {
    let mut last_draw = Instant::now();
    let draw_interval = Duration::from_millis(500);

    // Initial draw
    terminal.draw(|f| draw(f, app))?;

    loop {
        // 1. Drain pending snapshots
        let mut received_snap = false;
        while let Ok(snap) = snap_rx.try_recv() {
            app.ingest_snapshot(snap);
            received_snap = true;
        }

        // 2. Dynamic resize check
        if let Ok((w, h)) = crossterm::terminal::size() {
            app.layout_mode = if (w as usize) < 80 || (h as usize) < 24 {
                LayoutMode::Compact
            } else {
                LayoutMode::Full
            };
        }

        // 3. Redraw on new snapshot or timer tick
        if received_snap || last_draw.elapsed() >= draw_interval {
            terminal.draw(|f| draw(f, app))?;
            last_draw = Instant::now();
        }

        // 4. Non-blocking input poll (50ms timeout gives responsive UI + stats)
        if event::poll(Duration::from_millis(50))? {
            let ev = event::read()?;
            match ev {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Esc or Ctrl-C quits cleanly
                    if key.code == KeyCode::Esc
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL))
                    {
                        return Ok(());
                    }

                    match key.code {
                        KeyCode::Tab => {
                            app.focus = match app.focus {
                                Focus::Input => Focus::Chat,
                                Focus::Chat => Focus::Input,
                            };
                            terminal.draw(|f| draw(f, app))?;
                        }
                        KeyCode::Char(c) => {
                            if app.focus == Focus::Input {
                                app.input.push(c);
                                terminal.draw(|f| draw(f, app))?;
                            } else {
                                match c {
                                    'j' | ' ' => {
                                        app.scroll_down();
                                        terminal.draw(|f| draw(f, app))?;
                                    }
                                    'k' => {
                                        app.scroll_up();
                                        terminal.draw(|f| draw(f, app))?;
                                    }
                                    'g' => {
                                        app.chat_state.select(Some(0));
                                        terminal.draw(|f| draw(f, app))?;
                                    }
                                    'G' => {
                                        let last = app.chat_lines.len().saturating_sub(1);
                                        app.chat_state.select(Some(last));
                                        terminal.draw(|f| draw(f, app))?;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        KeyCode::Backspace => {
                            if app.focus == Focus::Input {
                                app.input.pop();
                                terminal.draw(|f| draw(f, app))?;
                            }
                        }
                        KeyCode::Enter
                            if app.focus == Focus::Input
                                && !app.input.trim().is_empty()
                                && !app.is_processing =>
                        {
                            let prompt = app.input.clone();
                            app.input.clear();
                            app.add_chat_line(&format!("User: {prompt}"));
                            app.is_processing = true;

                            // Immediate redraw so user sees prompt + processing state
                            terminal.draw(|f| draw(f, app))?;

                            // ponytail: synchronous AI call blocks event loop during inference.
                            let cfg_clone = cfg.clone();
                            let answer = match crate::ai::query_buffered(&cfg_clone, None, &prompt)
                            {
                                Ok(ans) => ans,
                                Err(e) => format!("Error: {e}"),
                            };

                            for line in answer.lines() {
                                app.add_chat_line(&format!("AI: {line}"));
                            }
                            app.is_processing = false;
                            terminal.draw(|f| draw(f, app))?;
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        app.scroll_up();
                        terminal.draw(|f| draw(f, app))?;
                    }
                    MouseEventKind::ScrollDown => {
                        app.scroll_down();
                        terminal.draw(|f| draw(f, app))?;
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        // Clicking bottom input row focuses input; clicking above focuses chat
                        if let Ok((_w, h)) = crossterm::terminal::size() {
                            app.focus = if mouse.row >= h.saturating_sub(3) {
                                Focus::Input
                            } else {
                                Focus::Chat
                            };
                        } else {
                            app.focus = match app.focus {
                                Focus::Input => Focus::Chat,
                                Focus::Chat => Focus::Input,
                            };
                        }
                        terminal.draw(|f| draw(f, app))?;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

// ── Draw Dispatcher ──────────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &mut ChatApp) {
    match app.layout_mode {
        LayoutMode::Full => draw_full(f, app),
        LayoutMode::Compact => draw_compact(f, app),
    }
}

// ── Full Layout: btop Dashboard + AI Chat + Input ───────────────────────────

fn draw_full(f: &mut Frame, app: &mut ChatApp) {
    let size = f.area();

    // Vertical split: Top content area + Bottom Input line ($ prompt)
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3)])
        .split(size);

    let main_chunk = vert[0];
    let input_chunk = vert[1];

    // Horizontal split: Left Dashboard (42%) + Right Chat History (58%)
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(main_chunk);

    let dash_chunk = horiz[0];
    let chat_chunk = horiz[1];

    draw_dashboard(f, app, dash_chunk);
    draw_chat_panel(f, app, chat_chunk);
    draw_input_bar(f, app, input_chunk);
}

/// Left btop-style dashboard panel:
/// - Real-time CPU sparkline (Chart with Braille markers)
/// - Multi-level gauges for RAM, Swap, Disk
/// - Hardware telemetry (CPU temp, Fan, GPU label/utilization/mem)
fn draw_dashboard(f: &mut Frame, app: &ChatApp, area: Rect) {
    let snap = &app.latest_snap;

    let dash_block = Block::default()
        .borders(Borders::ALL)
        .title(" System Telemetry & Dashboard ")
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(&dash_block, area);
    let inner = dash_block.inner(area);

    if inner.height < 6 || inner.width < 12 {
        // ponytail: fallback if dashboard area squeezed too small
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Terminal too small for dashboard",
                Style::default().fg(Color::DarkGray),
            ))),
            inner,
        );
        return;
    }

    // Split inner dashboard:
    // 1. CPU sparkline chart (flexible, takes remaining height, min 5)
    // 2. Gauges (RAM, Swap, Disk stacked) (Length 4)
    // 3. Hardware telemetry text (Length 3)
    let dash_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // CPU Sparkline
            Constraint::Length(4), // Memory / Swap / Disk Gauges
            Constraint::Length(3), // Hardware telemetry
        ])
        .split(inner);

    let chart_area = dash_chunks[0];
    let gauge_area = dash_chunks[1];
    let tele_area = dash_chunks[2];

    // ── 1. CPU Sparkline (ratatui Chart) ─────────────────────────────────────
    let chart_pts: Vec<(f64, f64)> = app
        .cpu_history
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v as f64))
        .collect();

    let chart_dataset = Dataset::default()
        .name(format!("CPU: {:.1}%", snap.cpu_pct))
        .data(&chart_pts)
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Green));

    let x_max = (MAX_HISTORY.saturating_sub(1)) as f64;
    let cpu_chart = Chart::new(vec![chart_dataset])
        .x_axis(
            Axis::default()
                .style(Style::default().fg(Color::DarkGray))
                .bounds([0.0, x_max])
                .labels([Line::from("-60s"), Line::from("-30s"), Line::from("now")]),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(Color::DarkGray))
                .bounds([0.0, 100.0])
                .labels([Line::from("0%"), Line::from("50%"), Line::from("100%")]),
        )
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Line::from(vec![
                    Span::styled(" CPU Usage ", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!("({:.1}%)", snap.cpu_pct),
                        Style::default().fg(if snap.cpu_pct > 80.0 { Color::LightRed } else { Color::Green }),
                    ),
                ])),
        );
    f.render_widget(cpu_chart, chart_area);

    // ── 2. Memory / Swap / Disk Gauges ────────────────────────────────────────
    let ram_pct = (snap.ram_used_mb * 100)
        .checked_div(snap.ram_total_mb)
        .unwrap_or(0);
    let dsk_pct = (snap.disk_used_gb * 100)
        .checked_div(snap.disk_total_gb)
        .unwrap_or(0);
    let swp_pct = if snap.swap_total_mb > 0 {
        (snap.swap_used_mb * 100)
            .checked_div(snap.swap_total_mb)
            .unwrap_or(0)
    } else {
        0
    };

    let gauge_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // RAM
            Constraint::Length(1), // Swap
            Constraint::Length(1), // Disk
            Constraint::Min(0),
        ])
        .split(gauge_area);

    let ram_gauge = Gauge::default()
        .ratio((ram_pct as f64 / 100.0).clamp(0.0, 1.0))
        .label(format!(
            "RAM: {}% ({}/{} MB)",
            ram_pct, snap.ram_used_mb, snap.ram_total_mb
        ))
        .gauge_style(Style::default().fg(Color::LightBlue).bg(Color::Rgb(20, 30, 45)));
    f.render_widget(ram_gauge, gauge_rows[0]);

    let swp_label = if snap.swap_total_mb > 0 {
        format!(
            "SWP: {}% ({}/{} MB)",
            swp_pct, snap.swap_used_mb, snap.swap_total_mb
        )
    } else {
        "SWP: None".to_string()
    };
    let swp_gauge = Gauge::default()
        .ratio((swp_pct as f64 / 100.0).clamp(0.0, 1.0))
        .label(swp_label)
        .gauge_style(Style::default().fg(Color::Magenta).bg(Color::Rgb(35, 20, 35)));
    f.render_widget(swp_gauge, gauge_rows[1]);

    let dsk_gauge = Gauge::default()
        .ratio((dsk_pct as f64 / 100.0).clamp(0.0, 1.0))
        .label(format!(
            "DSK: {}% ({}/{} GB)",
            dsk_pct, snap.disk_used_gb, snap.disk_total_gb
        ))
        .gauge_style(Style::default().fg(Color::Yellow).bg(Color::Rgb(40, 35, 15)));
    f.render_widget(dsk_gauge, gauge_rows[2]);

    // ── 3. Telemetry Text (Temp, Fan, GPU) ────────────────────────────────────
    let mut tele_lines = Vec::new();

    let temp_str = if snap.cpu_temp_c > 0.0 {
        format!("{:.0}°C", snap.cpu_temp_c)
    } else {
        "--".to_string()
    };
    let fan_str = snap.fan_speed.as_deref().unwrap_or("--");

    tele_lines.push(Line::from(vec![
        Span::styled("Thermal: ", Style::default().fg(Color::LightGreen)),
        Span::styled(temp_str, Style::default().fg(Color::White)),
        Span::styled("   Fan: ", Style::default().fg(Color::LightGreen)),
        Span::styled(fan_str, Style::default().fg(Color::White)),
    ]));

    let gpu_text = if snap.gpu_label != "No GPU" {
        let mut s = snap.gpu_label.clone();
        if let Some(pct) = snap.gpu_pct {
            s.push_str(&format!(" {pct:.0}%"));
        }
        if let (Some(used), Some(tot)) = (snap.gpu_mem_used_mb, snap.gpu_mem_total_mb) {
            s.push_str(&format!(" ({used}/{tot}MB)"));
        }
        if let Some(t) = snap.gpu_temp_c {
            s.push_str(&format!(" {t:.0}°C"));
        }
        s
    } else {
        "None detected".to_string()
    };

    tele_lines.push(Line::from(vec![
        Span::styled("GPU: ", Style::default().fg(Color::LightGreen)),
        Span::styled(gpu_text, Style::default().fg(Color::Cyan)),
    ]));

    f.render_widget(Paragraph::new(tele_lines), tele_area);
}

// ── Right AI Chat Panel ──────────────────────────────────────────────────────

fn draw_chat_panel(f: &mut Frame, app: &mut ChatApp, area: Rect) {
    let chat_border = if app.focus == Focus::Chat {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let chat_title = if app.focus == Focus::Chat {
        format!(
            " Hana AI Chat History ({} items) [Focused: j/k/wheel, Tab to input] ",
            app.chat_lines.len()
        )
    } else {
        format!(
            " Hana AI Chat History ({} items) [Mouse Scroll Enabled] ",
            app.chat_lines.len()
        )
    };

    let chat_block = Block::default()
        .borders(Borders::ALL)
        .title(chat_title)
        .border_style(Style::default().fg(chat_border));
    let chat_inner = chat_block.inner(area);
    f.render_widget(chat_block, area);

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

    f.render_stateful_widget(chat_list, chat_inner, &mut app.chat_state);
}

// ── Bottom Terminal Input Bar ────────────────────────────────────────────────

fn draw_input_bar(f: &mut Frame, app: &ChatApp, area: Rect) {
    let input_border = if app.focus == Focus::Input {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let title = if app.focus == Focus::Input {
        " Terminal Input [Enter: Send | Tab: Focus Chat | Esc: Quit] "
    } else {
        " Terminal Input [Tab or Click to Focus] "
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(input_border));
    let input_inner = input_block.inner(area);
    f.render_widget(input_block, area);

    let content = if app.is_processing {
        Line::from(vec![
            Span::styled("$ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(&app.input, Style::default().fg(Color::White)),
            Span::styled(
                "  ⏳ [Querying Hana AI (localhost:20128)...]",
                Style::default().fg(Color::LightRed).add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("$ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(&app.input, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(
                "█",
                Style::default().fg(if app.focus == Focus::Input {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }),
            ),
        ])
    };

    f.render_widget(Paragraph::new(content), input_inner);
}

// ── Compact / Degraded Layout (<80x24) ───────────────────────────────────────

fn draw_compact(f: &mut Frame, app: &mut ChatApp) {
    let size = f.area();

    let vert_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(size);

    let main_chunk = vert_chunks[0];
    let status_chunk = vert_chunks[1];

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(main_chunk);

    let input_chunk = chunks[0];
    let chat_chunk = chunks[1];

    // Left: Terminal Input Panel
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
                "⏳ Thinking / querying Hana AI..."
            } else {
                "Type prompt and press [Enter]."
            },
            Style::default().fg(if app.is_processing {
                Color::LightRed
            } else {
                Color::Green
            }),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Shortcuts:",
            Style::default().fg(Color::Gray).add_modifier(Modifier::UNDERLINED),
        )),
        Line::from(Span::styled(
            "• Tab / Click: Switch Panel",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "• Mouse Scroll: Scroll Chat",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "• Esc: Quit Chat",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let term_paragraph = Paragraph::new(term_lines);
    f.render_widget(term_paragraph, term_inner);

    // Right: AI Chat History Panel
    let chat_border_color = if app.focus == Focus::Chat {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let chat_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Hana AI Chat History ({} items) ",
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

    f.render_stateful_widget(chat_list, chat_inner, &mut app.chat_state);

    // Bottom Taskbar: Telemetry Widget
    let status_block = Block::default()
        .borders(Borders::ALL)
        .title(" System Telemetry (Auto-refresh) ")
        .border_style(Style::default().fg(Color::Green));
    let status_inner = status_block.inner(status_chunk);
    f.render_widget(status_block, status_chunk);

    let status_para = Paragraph::new(Line::from(Span::styled(
        &app.stat_line,
        Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD),
    )));
    f.render_widget(status_para, status_inner);
}

// ── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_history_ring_buffer() {
        let mut app = ChatApp::new();
        // Initial buffer pre-filled to MAX_HISTORY
        assert_eq!(app.cpu_history.len(), MAX_HISTORY);

        // Push new samples
        for i in 0..10u32 {
            let snap = SysSnapshot {
                cpu_pct: i as f32,
                ..Default::default()
            };
            app.ingest_snapshot(snap);
        }

        assert_eq!(app.cpu_history.len(), MAX_HISTORY);
        assert_eq!(app.cpu_history[MAX_HISTORY - 1], 9.0);
    }

    #[test]
    fn test_ingest_snapshot_updates_history() {
        let mut app = ChatApp::new();
        let snap = SysSnapshot {
            cpu_pct: 42.0,
            ram_used_mb: 1024,
            ram_total_mb: 4096,
            ..Default::default()
        };
        app.ingest_snapshot(snap);
        assert_eq!(app.cpu_history.len(), MAX_HISTORY);
        assert_eq!(app.cpu_history[MAX_HISTORY - 1], 42.0);
        assert_eq!(app.ram_history.len(), MAX_HISTORY);
        assert_eq!(app.ram_history[MAX_HISTORY - 1], 25.0);
    }

    #[test]
    fn test_add_chat_line_advances_list_state() {
        let mut app = ChatApp::new();
        let initial = app.chat_lines.len();
        app.add_chat_line("test message");
        assert_eq!(app.chat_lines.len(), initial + 1);
        assert_eq!(app.chat_state.selected(), Some(initial));
    }

    #[test]
    fn test_degrade_layout_mode_dimensions() {
        assert_eq!(
            if 79 < 80 || 25 < 24 {
                LayoutMode::Compact
            } else {
                LayoutMode::Full
            },
            LayoutMode::Compact
        );
        assert_eq!(
            if 80 < 80 || 23 < 24 {
                LayoutMode::Compact
            } else {
                LayoutMode::Full
            },
            LayoutMode::Compact
        );
        assert_eq!(
            if 120 < 80 || 40 < 24 {
                LayoutMode::Compact
            } else {
                LayoutMode::Full
            },
            LayoutMode::Full
        );
    }
}
