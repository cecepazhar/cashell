//! CRON tab: CAShell-managed jobs live in SQLite and are written into a
//! marked block of the user's crontab, so the system cron runs them even
//! when CAShell is closed. Other crontab lines are shown read-only and are
//! never touched.

use super::{cockpit_theme, CockpitApp, HitboxAction};
use crate::cron::{self, CronEntry, ManagedJob};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use std::time::{Duration, Instant};

const WIZARD_FIELDS: [&str; 3] = ["Schedule", "Command", "Description"];

impl CockpitApp {
    pub(crate) fn managed_jobs(&self) -> Vec<ManagedJob> {
        self.db
            .as_ref()
            .and_then(|d| d.get_cron_jobs().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|j| ManagedJob {
                id: j.id,
                schedule: j.schedule,
                command: j.command,
                description: j.description,
            })
            .collect()
    }

    /// Write the managed jobs into the user crontab.
    pub fn sync_cron(&mut self) -> bool {
        let jobs = self.managed_jobs();
        self.cron_cache_at = None;
        match cron::install(&jobs) {
            Ok(()) => {
                self.cron_error = None;
                true
            }
            Err(e) => {
                self.cron_error = Some(e.to_string());
                self.set_notification(format!("✗ Could not update crontab: {e}"));
                false
            }
        }
    }

    pub fn open_cron_wizard(&mut self) {
        self.show_cron_wizard = true;
        self.cron_wizard_field = 0;
        self.cron_schedule_buffer = "0 2 * * *".into();
        self.cron_command_buffer.clear();
        self.cron_desc_buffer.clear();
    }

    fn save_cron_wizard(&mut self) {
        let schedule = self.cron_schedule_buffer.trim().to_string();
        let command = self.cron_command_buffer.trim().to_string();
        if let Err(e) =
            cron::validate_schedule(&schedule).and_then(|_| cron::validate_command(&command))
        {
            self.set_notification(format!("✗ {e}"));
            return;
        }
        let desc = if self.cron_desc_buffer.trim().is_empty() {
            "Custom job".to_string()
        } else {
            self.cron_desc_buffer.trim().to_string()
        };
        let Some(ref d) = self.db else {
            self.set_notification("✗ Database not available");
            return;
        };
        if let Err(e) = d.add_cron_job(&schedule, &command, &desc) {
            self.set_notification(format!("✗ Could not save job: {e}"));
            return;
        }
        self.show_cron_wizard = false;
        if self.sync_cron() {
            self.set_notification(format!(
                "✔ Job '{desc}' scheduled ({schedule}) in your crontab"
            ));
        }
    }

    /// Run the selected managed job now (same wrapper as cron uses).
    pub fn run_selected_cron_job(&mut self) {
        let jobs = self.managed_jobs();
        let Some(job) = jobs.get(self.selected_cron_job.min(jobs.len().saturating_sub(1))) else {
            self.set_notification("⚠ No CAShell job selected");
            return;
        };
        let out = crate::host_guard::output(
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&job.command),
        );
        let code = match &out {
            Ok(o) => o.status.code().unwrap_or(-1),
            Err(_) => -1,
        };
        let line = format!(
            "{} {} {code}\n",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"),
            job.id
        );
        let log = cron::run_log_path();
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        if let Some(ref d) = self.db {
            let _ = d.update_cron_run(job.id, code);
        }
        let tail = out
            .ok()
            .map(|o| {
                let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                s.push_str(&String::from_utf8_lossy(&o.stderr));
                s.lines()
                    .last()
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect::<String>()
            })
            .unwrap_or_default();
        let mark = if code == 0 { "✔" } else { "✖" };
        self.set_notification(format!("{mark} '{}' exited {code} {tail}", job.description));
    }

    fn cron_others(&mut self) -> Vec<CronEntry> {
        let fresh = self
            .cron_cache_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(5));
        if !fresh {
            self.cron_cache = cron::read_raw_crontab()
                .map(|raw| cron::other_entries(&raw))
                .unwrap_or_default();
            self.cron_cache_at = Some(Instant::now());
        }
        self.cron_cache.clone()
    }
}

/// Keys while the New Job wizard is open.
pub fn handle_cron_wizard_key(key: &KeyEvent, app: &mut CockpitApp) {
    let buf = match app.cron_wizard_field {
        0 => &mut app.cron_schedule_buffer,
        1 => &mut app.cron_command_buffer,
        _ => &mut app.cron_desc_buffer,
    };
    match key.code {
        KeyCode::Esc => app.show_cron_wizard = false,
        KeyCode::Tab | KeyCode::Down => {
            app.cron_wizard_field = (app.cron_wizard_field + 1) % WIZARD_FIELDS.len()
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.cron_wizard_field =
                (app.cron_wizard_field + WIZARD_FIELDS.len() - 1) % WIZARD_FIELDS.len()
        }
        KeyCode::Enter => app.save_cron_wizard(),
        KeyCode::Backspace => {
            buf.pop();
        }
        KeyCode::Char(c) => buf.push(c),
        _ => {}
    }
}

pub fn draw_cron_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let muted = Style::default().fg(palette.text_muted);
    let bold = |c: Color| Style::default().fg(c).add_modifier(Modifier::BOLD);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Percentage(60),
            Constraint::Min(4),
        ])
        .split(area);
    let (Some(&bar), Some(&top), Some(&bottom)) = (rows.first(), rows.get(1), rows.get(2)) else {
        return;
    };

    let new_btn = " + New job (n) ";
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(new_btn, Style::default().fg(Color::Black).bg(palette.accent).add_modifier(Modifier::BOLD)),
            Span::styled("  ↑/↓ select · r run now · x delete · jobs are written to your crontab and run even when CAShell is closed", muted),
        ])),
        bar,
    );
    app.register_hitbox(
        Rect::new(bar.x, bar.y, catui::text::width(new_btn) as u16, 1),
        HitboxAction::NewCronWizard,
    );

    let jobs = app.managed_jobs();
    if !jobs.is_empty() {
        app.selected_cron_job = app.selected_cron_job.min(jobs.len() - 1);
    }
    let runs = cron::last_runs();
    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled(format!("{:<16}", "SCHEDULE"), bold(palette.accent)),
        Span::styled(format!("{:<22}", "DESCRIPTION"), bold(palette.accent)),
        Span::styled(format!("{:<26}", "LAST RUN"), bold(palette.accent)),
        Span::styled("COMMAND", bold(palette.accent)),
    ])];
    if jobs.is_empty() {
        lines.push(Line::from(Span::styled(
            "No CAShell jobs yet. Press n to schedule one.",
            muted,
        )));
    }
    for (i, j) in jobs.iter().enumerate() {
        let (last, color) = match runs.get(&j.id) {
            Some((ts, 0)) => (format!("{ts} ✔"), Color::Green),
            Some((ts, code)) => (format!("{ts} ✖ {code}"), Color::Red),
            None => ("never".to_string(), palette.text_muted),
        };
        let sel = i == app.selected_cron_job;
        let style = if sel {
            Style::default().bg(palette.row_selected_bg)
        } else {
            Style::default()
        };
        lines.push(
            Line::from(vec![
                Span::styled(if sel { "▶ " } else { "  " }, bold(palette.accent)),
                Span::styled(format!("{:<14}", j.schedule), bold(Color::Yellow)),
                Span::styled(
                    format!("{:<22}", super::topology::short_name(&j.description, 20)),
                    bold(palette.text),
                ),
                Span::styled(format!("{last:<26}"), Style::default().fg(color)),
                Span::styled(j.command.clone(), Style::default().fg(palette.text)),
            ])
            .style(style),
        );
    }
    if let Some(e) = &app.cron_error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Crontab error: {e}"),
            Style::default().fg(Color::Red),
        )));
    }
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" CAShell jobs ({}) ", jobs.len()))
                .border_style(Style::default().fg(palette.accent)),
        ),
        top,
    );

    let others = app.cron_others();
    let mut o: Vec<Line> = Vec::new();
    if others.is_empty() {
        o.push(Line::from(Span::styled("No other crontab entries.", muted)));
    }
    for e in &others {
        o.push(Line::from(vec![
            Span::styled(
                format!("{:<16}", e.schedule),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(e.command.clone(), Style::default().fg(palette.text)),
        ]));
    }
    f.render_widget(
        Paragraph::new(o).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Other crontab entries (read-only, managed outside CAShell) ")
                .border_style(Style::default().fg(palette.border)),
        ),
        bottom,
    );
}

pub fn draw_cron_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let popup = catui::layout::centered_rect(60, 45, area);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(" ⏰ New scheduled job ")
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let values = [
        &app.cron_schedule_buffer,
        &app.cron_command_buffer,
        &app.cron_desc_buffer,
    ];
    let mut lines = vec![Line::from("")];
    for (i, (label, v)) in WIZARD_FIELDS.iter().zip(values).enumerate() {
        let active = i == app.cron_wizard_field;
        lines.push(Line::from(vec![
            Span::styled(
                if active { "▶ " } else { "  " },
                Style::default().fg(palette.accent),
            ),
            Span::styled(
                format!("{label:<12}"),
                Style::default().fg(palette.text_muted),
            ),
            Span::styled(
                format!("{v}{}", if active { "█" } else { "" }),
                Style::default()
                    .fg(if active { Color::Yellow } else { palette.text })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    let hint = match cron::validate_schedule(&app.cron_schedule_buffer) {
        Ok(()) => Span::styled("  ✔ valid schedule", Style::default().fg(Color::Green)),
        Err(e) => Span::styled(format!("  ✖ {e}"), Style::default().fg(Color::Red)),
    };
    lines.extend([
        Line::from(""),
        Line::from(hint),
        Line::from(Span::styled(
            "  Format: minute hour day month weekday — e.g. 0 2 * * * (daily 02:00), */15 * * * *, @hourly",
            Style::default().fg(palette.text_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Save (Enter) ", Style::default().fg(Color::Black).bg(palette.success).add_modifier(Modifier::BOLD)),
            Span::styled("   Tab next field · Esc cancel", Style::default().fg(palette.text_muted)),
        ]),
    ]);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
