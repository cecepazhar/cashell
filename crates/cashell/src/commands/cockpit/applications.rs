//! Dashboard "Applications" table (replaces Host Top Processes).
//!
//! Rows are per desktop application (see [`crate::app_groups`]); `Space`
//! expands an application into its processes. Selection is pinned to the
//! row's identity (app key / pid), not its index, because the list re-sorts
//! every telemetry tick — otherwise Enter could act on a different app.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};

use super::{cpu_load_color, panel_border, truncate_str, CockpitApp, HitboxAction};
use crate::app_groups::{AppInfo, AppSignal};

/// Column the table is sorted by (`S` cycles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppSort {
    #[default]
    Cpu,
    Ram,
    Gpu,
    Read,
    Write,
    Down,
    Up,
}

impl AppSort {
    pub fn next(self) -> Self {
        match self {
            AppSort::Cpu => AppSort::Ram,
            AppSort::Ram => AppSort::Gpu,
            AppSort::Gpu => AppSort::Read,
            AppSort::Read => AppSort::Write,
            AppSort::Write => AppSort::Down,
            AppSort::Down => AppSort::Up,
            AppSort::Up => AppSort::Cpu,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AppSort::Cpu => "CPU",
            AppSort::Ram => "RAM",
            AppSort::Gpu => "GPU",
            AppSort::Read => "READ",
            AppSort::Write => "WRITE",
            AppSort::Down => "DOWN",
            AppSort::Up => "UP",
        }
    }
    fn key(self, a: &AppInfo) -> f64 {
        match self {
            AppSort::Cpu => f64::from(a.cpu_pct),
            AppSort::Ram => a.mem_mb as f64,
            AppSort::Gpu => f64::from(a.gpu_pct.unwrap_or(0.0)),
            AppSort::Read => a.read_kbps,
            AppSort::Write => a.write_kbps,
            AppSort::Down => a.down_kbps.unwrap_or(0.0),
            AppSort::Up => a.up_kbps.unwrap_or(0.0),
        }
    }
}

/// Identity of a table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppRowKey {
    App(String),
    Proc(String, u32),
}

/// What an opened action menu / confirmation acts on (frozen at open time).
#[derive(Debug, Clone, PartialEq)]
pub struct AppActionTarget {
    pub label: String,
    pub pids: Vec<u32>,
    pub stopped: bool,
    /// True when the target includes the process running CAShell's terminal.
    pub contains_self_tree: bool,
}

pub const APP_ACTIONS: [&str; 5] = [
    "End Task (SIGTERM)",
    "Force Kill (SIGKILL)",
    "Pause / Resume",
    "Lower Priority (nice +10)",
    "Cancel",
];

pub fn sorted_apps(apps: &[AppInfo], sort: AppSort) -> Vec<AppInfo> {
    let mut v = apps.to_vec();
    v.sort_by(|a, b| {
        sort.key(b)
            .partial_cmp(&sort.key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
    v
}

/// Flatten apps (+ expanded processes) into row keys.
pub fn row_keys(apps: &[AppInfo], expanded: &std::collections::HashSet<String>) -> Vec<AppRowKey> {
    let mut rows = Vec::new();
    for a in apps {
        rows.push(AppRowKey::App(a.key.clone()));
        if expanded.contains(&a.key) {
            for p in &a.processes {
                rows.push(AppRowKey::Proc(a.key.clone(), p.pid));
            }
        }
    }
    rows
}

/// Compact rate: "" for idle, "–" when unavailable.
pub fn fmt_rate(kbps: Option<f64>) -> String {
    match kbps {
        None => "–".to_string(),
        Some(v) if v < 0.05 => String::new(),
        Some(v) if v < 1024.0 => format!("{v:.0}K/s"),
        Some(v) => format!("{:.1}M/s", v / 1024.0),
    }
}

pub fn fmt_mb(mb: u64) -> String {
    if mb == 0 {
        String::new()
    } else if mb < 1024 {
        format!("{mb}M")
    } else {
        format!("{:.1}G", mb as f64 / 1024.0)
    }
}

impl CockpitApp {
    fn app_view(&self) -> (Vec<AppInfo>, Vec<AppRowKey>) {
        let apps = sorted_apps(&self.latest_snap.applications, self.app_sort);
        let rows = row_keys(&apps, &self.app_expanded);
        (apps, rows)
    }

    /// Re-pin `dashboard_proc_selected` to the selected row's identity.
    pub fn sync_app_selection(&mut self) {
        let (_, rows) = self.app_view();
        if rows.is_empty() {
            self.dashboard_proc_selected = 0;
            return;
        }
        // Only pin to an identity once the user picked a row; until then the
        // cursor stays at the top so the heaviest app is what you see.
        match self
            .app_selected_key
            .as_ref()
            .and_then(|k| rows.iter().position(|r| r == k))
        {
            Some(pos) => self.dashboard_proc_selected = pos,
            None => {
                self.dashboard_proc_selected = self.dashboard_proc_selected.min(rows.len() - 1);
                if self.app_selected_key.is_some() {
                    // Picked row vanished (process exited): re-pin to what's there now.
                    self.app_selected_key = rows.get(self.dashboard_proc_selected).cloned();
                }
            }
        }
    }

    pub fn move_app_selection(&mut self, delta: isize) {
        self.sync_app_selection();
        let (_, rows) = self.app_view();
        if rows.is_empty() {
            return;
        }
        let max = rows.len() as isize - 1;
        let idx = (self.dashboard_proc_selected as isize + delta).clamp(0, max) as usize;
        self.dashboard_proc_selected = idx;
        self.app_selected_key = rows.get(idx).cloned();
    }

    pub fn select_app_row(&mut self, idx: usize) {
        let (_, rows) = self.app_view();
        if let Some(k) = rows.get(idx) {
            self.dashboard_proc_selected = idx;
            self.app_selected_key = Some(k.clone());
        }
    }

    pub fn cycle_app_sort(&mut self) {
        self.app_sort = self.app_sort.next();
        self.set_notification(format!("Applications sorted by {}", self.app_sort.label()));
    }

    /// Space: expand / collapse the selected application.
    pub fn toggle_app_expand(&mut self) {
        self.sync_app_selection();
        let current = self.app_selected_key.clone().or_else(|| {
            let (_, rows) = self.app_view();
            rows.get(self.dashboard_proc_selected).cloned()
        });
        let key = match current {
            Some(AppRowKey::App(k)) | Some(AppRowKey::Proc(k, _)) => k,
            None => return,
        };
        if !self.app_expanded.remove(&key) {
            self.app_expanded.insert(key.clone());
        }
        self.app_selected_key = Some(AppRowKey::App(key));
        self.sync_app_selection();
    }

    /// Resolve the selected row into an action target.
    pub fn selected_app_target(&mut self) -> Option<AppActionTarget> {
        self.sync_app_selection();
        let (apps, rows) = self.app_view();
        let me = std::process::id();
        let key = self
            .app_selected_key
            .clone()
            .or_else(|| rows.get(self.dashboard_proc_selected).cloned())?;
        match key {
            AppRowKey::App(k) => {
                let a = apps.iter().find(|a| a.key == k)?;
                if !a.group_actions {
                    return None;
                }
                Some(AppActionTarget {
                    label: a.name.clone(),
                    pids: a.pids(),
                    stopped: a.is_stopped,
                    contains_self_tree: a.processes.iter().any(|p| p.pid == me),
                })
            }
            AppRowKey::Proc(k, pid) => {
                let a = apps.iter().find(|a| a.key == k)?;
                let p = a.processes.iter().find(|p| p.pid == pid)?;
                Some(AppActionTarget {
                    label: format!("{} (PID {})", p.name, p.pid),
                    pids: vec![p.pid],
                    stopped: p.is_stopped,
                    contains_self_tree: p.pid == me,
                })
            }
        }
    }

    /// Enter on the Applications panel.
    pub fn open_app_actions(&mut self) {
        match self.selected_app_target() {
            Some(t) => {
                self.app_action_target = Some(t);
                self.app_action_selected = 0;
                self.show_app_action_menu = true;
            }
            None => {
                // Background / System buckets mix unrelated daemons: act per process.
                self.toggle_app_expand();
                self.set_notification(
                    "This group has several daemons — pick a process (Space to expand/collapse)",
                );
            }
        }
    }

    /// `x` / Delete: ask to End Task on the selected row.
    pub fn prompt_end_selected_app(&mut self) {
        match self.selected_app_target() {
            Some(t) => self.prompt_app_signal(t, false),
            None => {
                self.set_notification("Select an app or process first (Space to expand a group)")
            }
        }
    }

    fn prompt_app_signal(&mut self, t: AppActionTarget, force: bool) {
        self.pending_kill_proc = Some((t.pids.first().copied().unwrap_or(0), t.label.clone()));
        self.pending_kill_pids = t.pids.clone();
        self.pending_kill_force = force;
        self.app_action_target = Some(t);
        self.show_kill_modal = true;
    }

    /// Run the highlighted action-menu entry.
    pub fn run_app_action(&mut self, idx: usize) {
        self.show_app_action_menu = false;
        let Some(t) = self.app_action_target.clone() else {
            return;
        };
        match idx {
            0 => self.prompt_app_signal(t, false),
            1 => self.prompt_app_signal(t, true),
            2 => {
                let sig = if t.stopped {
                    AppSignal::Resume
                } else {
                    AppSignal::Pause
                };
                let (ok, failed) = crate::app_groups::signal_pids(&t.pids, sig);
                let verb = if t.stopped { "Resumed" } else { "Paused" };
                self.set_notification(format!("{verb} {} ({ok} ok, {failed} failed)", t.label));
            }
            3 => {
                let (ok, failed) = crate::app_groups::lower_priority(&t.pids);
                self.set_notification(format!(
                    "Lowered priority of {} ({ok} ok, {failed} failed)",
                    t.label
                ));
            }
            _ => {}
        }
    }
}

/// Columns by priority; added left-to-right while they fit.
const OPTIONAL_COLS: [(&str, u16); 7] = [
    ("DOWN", 8),
    ("UP", 8),
    ("READ", 8),
    ("WRITE", 8),
    ("GPU", 5),
    ("SWAP", 7),
    ("USER", 10),
];
const NAME_MIN: u16 = 16;
const CPU_W: u16 = 11;
const RAM_W: u16 = 7;

fn visible_cols(width: u16) -> Vec<(&'static str, u16)> {
    let mut used = NAME_MIN + CPU_W + RAM_W + 2;
    let mut cols = Vec::new();
    for (name, w) in OPTIONAL_COLS {
        if used + w < width {
            used += w + 1;
            cols.push((name, w));
        }
    }
    cols
}

pub(super) fn draw_applications_panel(
    f: &mut Frame,
    app: &mut CockpitApp,
    area: Rect,
    palette: &catui::Theme,
    is_focused: bool,
) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    app.register_hitbox(area, HitboxAction::FocusDashboardPanel(5));
    app.sync_app_selection();
    let (apps, rows) = app.app_view();
    let selected = app.dashboard_proc_selected;

    let focus_hint = if is_focused {
        " [● FOCUSED · ↑↓ Select · Enter Actions · Space Expand · S Sort]"
    } else {
        ""
    };
    let title = format!(
        " ■ Applications ({}) · sort {}{focus_hint} ",
        apps.len(),
        app.app_sort.label()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(panel_border(palette, is_focused, false));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 {
        return;
    }

    let extra = visible_cols(inner.width);
    let visible_rows = inner.height.saturating_sub(1) as usize;
    let scroll = selected.saturating_sub(visible_rows.saturating_sub(1));

    let mut table_rows = Vec::new();
    for (screen_row, (idx, key)) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_rows)
        .enumerate()
    {
        let (name, cpu, mem, swap, gpu, read, write, down, up, user, stopped) = match key {
            AppRowKey::App(k) => {
                let Some(a) = apps.iter().find(|a| &a.key == k) else {
                    continue;
                };
                let marker = if a.processes.len() > 1 {
                    if app.app_expanded.contains(k) {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                };
                (
                    format!("{marker}{}", a.name),
                    a.cpu_pct,
                    a.mem_mb,
                    a.swap_mb,
                    a.gpu_pct,
                    a.read_kbps,
                    a.write_kbps,
                    a.down_kbps,
                    a.up_kbps,
                    a.user.clone(),
                    a.is_stopped,
                )
            }
            AppRowKey::Proc(k, pid) => {
                let Some(p) = apps
                    .iter()
                    .find(|a| &a.key == k)
                    .and_then(|a| a.processes.iter().find(|p| p.pid == *pid))
                else {
                    continue;
                };
                (
                    format!("   └ {} ({})", p.name, p.pid),
                    p.cpu_pct,
                    p.mem_mb,
                    p.swap_mb,
                    p.gpu_pct,
                    p.read_kbps,
                    p.write_kbps,
                    p.down_kbps,
                    p.up_kbps,
                    p.user.clone(),
                    p.is_stopped,
                )
            }
        };

        let load = cpu_load_color(palette, cpu);
        let bar_len = 4usize;
        let filled = ((cpu / 100.0) * bar_len as f32)
            .ceil()
            .clamp(0.0, bar_len as f32) as usize;
        let filled = if cpu >= 0.1 { filled.max(1) } else { 0 };
        let cpu_cell = Line::from(vec![
            Span::styled("▮".repeat(filled), Style::default().fg(load)),
            Span::styled(
                "▯".repeat(bar_len - filled),
                Style::default().fg(palette.text_muted),
            ),
            Span::styled(format!("{cpu:>6.1}%"), Style::default().fg(load)),
        ]);
        let name = if stopped { format!("{name} ⏸") } else { name };

        let muted = Style::default().fg(palette.text_muted);
        let text = Style::default().fg(palette.text);
        let mut cells = vec![
            Cell::from(truncate_str(&name, 40).to_string()).style(text),
            Cell::from(cpu_cell),
            Cell::from(fmt_mb(mem)).style(text),
        ];
        for (col, _) in &extra {
            let c = match *col {
                "DOWN" => Cell::from(fmt_rate(down)).style(Style::default().fg(palette.accent)),
                "UP" => Cell::from(fmt_rate(up)).style(Style::default().fg(palette.accent)),
                "READ" => Cell::from(fmt_rate(Some(read))).style(text),
                "WRITE" => Cell::from(fmt_rate(Some(write))).style(text),
                "GPU" => Cell::from(match gpu {
                    Some(g) if g >= 0.5 => format!("{g:.0}%"),
                    Some(_) => String::new(),
                    None => "–".to_string(),
                })
                .style(text),
                "SWAP" => Cell::from(fmt_mb(swap)).style(muted),
                _ => Cell::from(truncate_str(&user, 10).to_string()).style(muted),
            };
            cells.push(c);
        }

        let row_y = inner.y + 1 + screen_row as u16;
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: row_y,
                width: inner.width,
                height: 1,
            },
            HitboxAction::SelectProcess(idx),
        );

        let row = Row::new(cells);
        table_rows.push(if idx == selected && is_focused {
            row.style(palette.selected())
        } else if idx == selected {
            row.style(Style::default().add_modifier(Modifier::BOLD))
        } else {
            row
        });
    }

    if table_rows.is_empty() {
        table_rows
            .push(Row::new(vec![Cell::from("  Collecting process data…")
                .style(Style::default().fg(palette.text_muted))]));
    }

    let mut widths = vec![
        Constraint::Min(NAME_MIN),
        Constraint::Length(CPU_W),
        Constraint::Length(RAM_W),
    ];
    let mut header = vec!["NAME", "CPU", "RAM"];
    for (name, w) in &extra {
        widths.push(Constraint::Length(*w));
        header.push(name);
    }
    let table = Table::new(table_rows, widths).header(
        Row::new(header).style(
            Style::default()
                .fg(palette.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
    );
    f.render_widget(table, inner);
}

pub(super) fn draw_app_action_menu(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = super::cockpit_theme(&app.cfg.output.theme);
    let Some(t) = app.app_action_target.clone() else {
        return;
    };
    let w = 44u16.min(area.width);
    let h = (APP_ACTIONS.len() as u16 + 4).min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", truncate_str(&t.label, 36)))
        .border_style(panel_border(&palette, true, false));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines = vec![Line::from(Span::styled(
        format!("{} process(es)", t.pids.len()),
        Style::default().fg(palette.text_muted),
    ))];
    for (i, label) in APP_ACTIONS.iter().enumerate() {
        let label = if i == 2 {
            if t.stopped {
                "Resume (SIGCONT)"
            } else {
                "Pause (SIGSTOP)"
            }
        } else {
            label
        };
        let style = if i == app.app_action_selected {
            palette.highlight()
        } else if i <= 1 {
            Style::default().fg(palette.danger)
        } else {
            Style::default().fg(palette.text)
        };
        let row_rect = Rect {
            x: inner.x,
            y: inner.y + 1 + i as u16,
            width: inner.width,
            height: 1,
        };
        app.register_hitbox(row_rect, HitboxAction::RunAppAction(i));
        lines.push(Line::from(Span::styled(
            format!(" {}  {label} ", i + 1),
            style,
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_groups::AppProcess;
    use crate::config::Config;

    fn app_info(key: &str, cpu: f32, pids: &[u32]) -> AppInfo {
        AppInfo {
            key: key.into(),
            name: key.into(),
            cpu_pct: cpu,
            group_actions: true,
            processes: pids
                .iter()
                .map(|&pid| AppProcess {
                    pid,
                    name: format!("{key}-{pid}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn selection_follows_app_when_list_reorders() {
        let mut app = CockpitApp::new(&Config::default());
        app.latest_snap.applications = vec![app_info("a", 50.0, &[10]), app_info("b", 5.0, &[20])];
        app.move_app_selection(1); // select "b"
        assert_eq!(app.app_selected_key, Some(AppRowKey::App("b".into())));
        // Next tick "b" becomes the heaviest and moves to the top.
        app.latest_snap.applications = vec![app_info("a", 1.0, &[10]), app_info("b", 90.0, &[20])];
        app.sync_app_selection();
        assert_eq!(app.dashboard_proc_selected, 0);
        let t = app.selected_app_target().map(|t| t.pids);
        assert_eq!(t, Some(vec![20]));
    }

    #[test]
    fn cursor_stays_on_top_until_user_picks() {
        let mut app = CockpitApp::new(&Config::default());
        // Startup tick: all CPU 0 -> alphabetical, "a" first.
        app.latest_snap.applications = vec![app_info("a", 0.0, &[10]), app_info("z", 0.0, &[20])];
        app.sync_app_selection();
        assert_eq!(app.app_selected_key, None);
        // Real data arrives: "z" is heaviest; cursor must stay on the top row.
        app.latest_snap.applications = vec![app_info("a", 0.0, &[10]), app_info("z", 40.0, &[20])];
        app.sync_app_selection();
        assert_eq!(app.dashboard_proc_selected, 0);
        assert_eq!(app.selected_app_target().map(|t| t.pids), Some(vec![20]));
    }

    #[test]
    fn expand_shows_processes_and_targets_single_pid() {
        let mut app = CockpitApp::new(&Config::default());
        app.latest_snap.applications = vec![app_info("a", 10.0, &[10, 11])];
        app.toggle_app_expand();
        app.move_app_selection(2);
        assert_eq!(app.app_selected_key, Some(AppRowKey::Proc("a".into(), 11)));
        assert_eq!(app.selected_app_target().map(|t| t.pids), Some(vec![11]));
    }

    #[test]
    fn background_group_has_no_group_actions() {
        let mut app = CockpitApp::new(&Config::default());
        let mut bg = app_info(crate::app_groups::BACKGROUND_GROUP, 10.0, &[10, 11]);
        bg.group_actions = false;
        app.latest_snap.applications = vec![bg];
        app.open_app_actions();
        assert!(!app.show_app_action_menu);
        assert!(app
            .app_expanded
            .contains(crate::app_groups::BACKGROUND_GROUP));
    }

    #[test]
    fn end_task_goes_through_confirmation() {
        let mut app = CockpitApp::new(&Config::default());
        app.latest_snap.applications = vec![app_info("a", 10.0, &[10, 11])];
        app.open_app_actions();
        assert!(app.show_app_action_menu);
        app.run_app_action(0);
        assert!(app.show_kill_modal);
        assert_eq!(app.pending_kill_pids, vec![10, 11]);
        assert!(!app.pending_kill_force);
    }

    #[test]
    fn rate_and_size_formatting() {
        assert_eq!(fmt_rate(None), "–");
        assert_eq!(fmt_rate(Some(0.0)), "");
        assert_eq!(fmt_rate(Some(12.4)), "12K/s");
        assert_eq!(fmt_rate(Some(2048.0)), "2.0M/s");
        assert_eq!(fmt_mb(512), "512M");
        assert_eq!(fmt_mb(2048), "2.0G");
    }
}
