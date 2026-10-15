//! Interactive TUI browser built on ratatui + crossterm.
//!
//! Launched via `cashell product browse` (or bare `cashell product`).
//! Falls back to plain listing when stdout is not a TTY.
//!
//! Keys: ↑/↓ or j/k = move, Enter/i = detail, Esc/q = quit.

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;

use crate::commands::product::{catalog, ProductMeta};

pub struct ProductRow {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub price_range: String,
    pub status: String,
}

impl From<&ProductMeta> for ProductRow {
    fn from(p: &crate::commands::product::ProductMeta) -> Self {
        Self {
            slug: p.slug.into(),
            name: p.name.into(),
            description: p.description.into(),
            price_range: p.price_range.into(),
            status: p.status.into(),
        }
    }
}

struct App {
    items: Vec<ProductRow>,
    state: ListState,
    detail: bool,
}

impl App {
    fn new(items: Vec<ProductRow>) -> Self {
        let mut state = ListState::default();
        if !items.is_empty() {
            state.select(Some(0));
        }
        Self {
            items,
            state,
            detail: false,
        }
    }

    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) if i + 1 < self.items.len() => i + 1,
            _ => 0,
        };
        self.state.select(Some(i));
    }

    fn prev(&mut self) {
        let i = match self.state.selected() {
            Some(0) | None => self.items.len().saturating_sub(1),
            Some(i) => i - 1,
        };
        self.state.select(Some(i));
    }

    fn selected(&self) -> Option<&ProductRow> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

/// Run the interactive product browser. Returns Err if terminal unavailable.
pub fn run_product_browser() -> Result<()> {
    // Guard: TUI needs a real terminal
    if !crate::output::is_tty() {
        anyhow::bail!("TUI requires an interactive terminal (stdout is not a TTY)");
    }

    let items: Vec<ProductRow> = catalog().iter().map(ProductRow::from).collect();
    let mut app = App::new(items);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    if app.detail {
                        app.detail = false;
                    } else {
                        break;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => app.next(),
                KeyCode::Up | KeyCode::Char('k') => app.prev(),
                KeyCode::Enter | KeyCode::Char('i') => app.detail = !app.detail,
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(f.area());

    if app.detail {
        draw_detail(f, chunks[0], app.selected());
    } else {
        draw_list(f, chunks[0], app);
    }

    let hint = Paragraph::new(Line::from(vec![
        Span::styled(" ↑/↓ ", Style::default().fg(Color::Rgb(255, 150, 120))),
        Span::raw("move  "),
        Span::styled(" enter ", Style::default().fg(Color::Rgb(255, 150, 120))),
        Span::raw("detail  "),
        Span::styled(" q ", Style::default().fg(Color::Rgb(255, 150, 120))),
        Span::raw("quit "),
    ]))
    .style(Style::default().fg(Color::DarkGray));
    f.render_widget(hint, chunks[1]);
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
    let items: Vec<ListItem> = app
        .items
        .iter()
        .map(|p| {
            let status_color = match p.status.as_str() {
                "active" => Color::Rgb(120, 220, 160),
                "beta" => Color::Rgb(255, 200, 100),
                _ => Color::DarkGray,
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<14}", p.slug),
                    Style::default()
                        .fg(Color::Rgb(255, 130, 100))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{:<16}", p.name), Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:<8}", p.status),
                    Style::default().fg(status_color),
                ),
                Span::styled(p.price_range.clone(), Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(80, 80, 95)))
                .title(Span::styled(
                    " CA Product Catalog ",
                    Style::default()
                        .fg(Color::Rgb(255, 130, 100))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 48))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut app.state);
}

fn draw_detail(f: &mut Frame, area: Rect, sel: Option<&ProductRow>) {
    let text = match sel {
        Some(p) => vec![
            Line::from(Span::styled(
                p.name.clone(),
                Style::default()
                    .fg(Color::Rgb(255, 130, 100))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("slug       ", Style::default().fg(Color::DarkGray)),
                Span::raw(p.slug.clone()),
            ]),
            Line::from(vec![
                Span::styled("status     ", Style::default().fg(Color::DarkGray)),
                Span::raw(p.status.clone()),
            ]),
            Line::from(vec![
                Span::styled("price      ", Style::default().fg(Color::DarkGray)),
                Span::raw(p.price_range.clone()),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                p.description.clone(),
                Style::default().fg(Color::Gray),
            )),
        ],
        None => vec![Line::from("No product selected")],
    };

    let p = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(80, 80, 95)))
                .title(Span::styled(
                    " Detail ",
                    Style::default().fg(Color::Rgb(255, 130, 100)),
                )),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}
