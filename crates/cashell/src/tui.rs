//! Interactive TUI browser built on CATUI + Ratatui.
//!
//! Launched via `cashell product browse` (or bare `cashell product`).
//! Falls back to plain listing when stdout is not a TTY.
//!
//! Keys: ↑/↓ or j/k = move, Enter/i = detail, Esc/q = quit, Ctrl+C = emergency exit.

use anyhow::Result;
use crossterm::event::{self, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

use crate::commands::cockpit::cockpit_theme;
use crate::commands::product::{catalog, ProductMeta};
use crate::config::Config;

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
    palette: catui::Theme,
}

impl App {
    fn new(items: Vec<ProductRow>, palette: catui::Theme) -> Self {
        let mut state = ListState::default();
        if !items.is_empty() {
            state.select(Some(0));
        }
        Self {
            items,
            state,
            detail: false,
            palette,
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
    let theme_name = Config::load(None)
        .map(|c| c.output.theme)
        .unwrap_or_else(|_| "ca-classic".to_string());
    let palette = cockpit_theme(&theme_name);
    let mut app = App::new(items, palette);

    let mut session = catui::terminal::TerminalSession::enter(catui::terminal::TerminalOptions {
        mouse: false,
        bracketed_paste: false,
        alternate_screen: true,
    })?;
    let terminal = &mut session.terminal;

    event_loop(terminal, &mut app)
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        if let Some(catui::event::AppEvent::Key(key)) = catui::event::map(event::read()?) {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                break;
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
        draw_detail(f, chunks[0], app.selected(), &app.palette);
    } else {
        draw_list(f, chunks[0], app);
    }

    let p = &app.palette;
    let hint = Paragraph::new(Line::from(vec![
        Span::styled(" ↑/↓ ", Style::default().fg(p.accent)),
        Span::styled("move  ", Style::default().fg(p.text_muted)),
        Span::styled("enter ", Style::default().fg(p.accent)),
        Span::styled("detail  ", Style::default().fg(p.text_muted)),
        Span::styled("q ", Style::default().fg(p.accent)),
        Span::styled("quit", Style::default().fg(p.text_muted)),
    ]))
    .style(Style::default().fg(p.text_muted));
    f.render_widget(hint, chunks[1]);
}

fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
    let p = &app.palette;
    let items: Vec<ListItem> = app
        .items
        .iter()
        .map(|item| {
            let status_color = match item.status.as_str() {
                "active" => p.success,
                "beta" => p.warning,
                _ => p.text_muted,
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<14}", item.slug),
                    Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{:<16}", item.name), Style::default().fg(p.text)),
                Span::styled(
                    format!("{:<8}", item.status),
                    Style::default().fg(status_color),
                ),
                Span::styled(item.price_range.clone(), Style::default().fg(p.text_muted)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(p.border))
                .title(Span::styled(
                    " CA Product Catalog ",
                    Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(p.row_selected_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut app.state);
}

fn draw_detail(f: &mut Frame, area: Rect, sel: Option<&ProductRow>, p: &catui::Theme) {
    let text = match sel {
        Some(item) => vec![
            Line::from(Span::styled(
                item.name.clone(),
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("slug       ", Style::default().fg(p.text_muted)),
                Span::styled(item.slug.clone(), Style::default().fg(p.text)),
            ]),
            Line::from(vec![
                Span::styled("status     ", Style::default().fg(p.text_muted)),
                Span::styled(
                    item.status.clone(),
                    Style::default().fg(if item.status == "active" { p.success } else { p.warning }),
                ),
            ]),
            Line::from(vec![
                Span::styled("price      ", Style::default().fg(p.text_muted)),
                Span::styled(item.price_range.clone(), Style::default().fg(p.text)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                item.description.clone(),
                Style::default().fg(p.text_soft),
            )),
        ],
        None => vec![Line::from("No product selected")],
    };

    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(p.border))
                .title(Span::styled(
                    " Detail ",
                    Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}
