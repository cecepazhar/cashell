//! Terminal tab: a real shell (local or SSH to a fleet server) in the panel.
//!
//! While the panel has focus every key goes to the shell (Ctrl+C, digits,
//! F-keys, Ctrl+E …). `Ctrl+]` gives the keys back to CAShell so number
//! keys switch tabs again; `Enter` or a click gives them back to the shell.

use super::pty::{key_bytes, PtySession};
use super::{cockpit_theme, CockpitApp, CockpitTab, HitboxAction};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

impl CockpitApp {
    /// Local → each fleet server → Local. Restarts the shell.
    pub fn cycle_terminal_target(&mut self) {
        let mut nodes: Vec<String> = self.cfg.daemon.nodes.keys().cloned().collect();
        nodes.sort();
        self.term_remote = match &self.term_remote {
            None => nodes.first().cloned(),
            Some(cur) => nodes
                .iter()
                .position(|n| n == cur)
                .and_then(|i| nodes.get(i + 1).cloned()),
        };
        self.term_pty = None;
        self.term_error = None;
        self.term_focus = true;
        let label = self.term_remote.clone().unwrap_or_else(|| "Local".into());
        self.set_notification(format!("Terminal target: {label}"));
    }

    fn spawn_terminal(&mut self, rows: u16, cols: u16) {
        let res = match &self.term_remote {
            None => PtySession::local(rows, cols),
            Some(name) => match self.cfg.daemon.nodes.get(name) {
                Some(node) => PtySession::remote(name, node, rows, cols),
                None => Err(anyhow::anyhow!("server '{name}' is no longer registered")),
            },
        };
        match res {
            Ok(s) => {
                self.term_pty = Some(s);
                self.term_error = None;
            }
            Err(e) => self.term_error = Some(e.to_string()),
        }
    }
}

/// Keys while the Terminal tab is open. Returns true when the key was used.
pub fn handle_terminal_key(key: &KeyEvent, app: &mut CockpitApp) -> bool {
    if app.current_tab != CockpitTab::Terminal
        || app.is_locked
        || app.show_upgrade_modal
        || app.show_quit_confirm
        || app.show_help_modal
    {
        return false;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == KeyCode::Char('t') {
        app.cycle_terminal_target();
        return true;
    }
    if !app.term_focus {
        if key.code == KeyCode::Enter {
            app.term_focus = true;
            return true;
        }
        return false; // CAShell shortcuts (digits switch tabs, …)
    }
    if ctrl && matches!(key.code, KeyCode::Char(']') | KeyCode::Char('5')) {
        app.term_focus = false;
        app.set_notification("Keys released — 1-9 switch tabs, Enter returns to the shell");
        return true;
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let exited = app.term_pty.as_mut().is_some_and(|p| p.exited());
    if exited {
        if key.code == KeyCode::Enter {
            app.term_pty = None;
            app.term_error = None;
        }
        return true;
    }
    if let Some(p) = app.term_pty.as_mut() {
        match key.code {
            KeyCode::PageUp if shift => p.scroll(true, 10),
            KeyCode::PageDown if shift => p.scroll(false, 10),
            _ => p.write(&key_bytes(key)),
        }
    }
    true
}

pub fn draw_terminal_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let muted = Style::default().fg(palette.text_muted);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(4)])
        .split(area);
    let (Some(&bar), Some(&body)) = (rows.first(), rows.get(1)) else {
        return;
    };

    let target = app
        .term_remote
        .clone()
        .unwrap_or_else(|| "Local".to_string());
    let keys = if app.term_focus {
        "Ctrl+] release keys · Ctrl+T switch target · Shift+PgUp/PgDn scroll"
    } else {
        "Enter or click to type · 1-9 switch tabs · Ctrl+T switch target"
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Target ",
                Style::default().fg(Color::Black).bg(palette.accent),
            ),
            Span::styled(
                format!(" {target} "),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {keys}"), muted),
        ])),
        bar,
    );

    let title = app
        .term_pty
        .as_ref()
        .map_or_else(|| format!(" ❯ {target} "), |p| format!(" ❯ {} ", p.label));
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if app.term_focus {
            Style::default().fg(palette.accent)
        } else {
            muted
        });
    let inner = block.inner(body);
    f.render_widget(block, body);
    app.register_hitbox(body, HitboxAction::FocusTerminal);

    if app.term_pty.is_none() && app.term_error.is_none() {
        app.spawn_terminal(inner.height, inner.width);
    }
    if let Some(err) = app.term_error.clone() {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!("Could not start the shell: {err}"),
                    Style::default().fg(Color::Red),
                )),
                Line::from(Span::styled("Ctrl+T to pick another target.", muted)),
            ]),
            inner,
        );
        return;
    }
    let mut exited = false;
    let mut cursor = None;
    if let Some(p) = app.term_pty.as_mut() {
        p.resize(inner.height, inner.width);
        cursor = p.render(f.buffer_mut(), inner);
        exited = p.exited();
    }
    if exited {
        let y = inner.y + inner.height.saturating_sub(1);
        f.render_widget(
            Paragraph::new(Span::styled(
                " [session ended — press Enter for a new one] ",
                Style::default().fg(Color::Black).bg(Color::Yellow),
            )),
            Rect::new(inner.x, y, inner.width, 1),
        );
    } else if let (true, Some((x, y))) = (app.term_focus, cursor) {
        f.set_cursor_position((x, y));
    }
}
