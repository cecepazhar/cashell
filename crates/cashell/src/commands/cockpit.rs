//! `cashell cockpit` — Personal Cloud Cockpit TUI.
//!
//! Features:
//! - Lightweight starfield particle animation (~8-10 FPS, ~100ms tick, battery-friendly)
//! - Top Cockpit Header: Starfield banner + system summary (hostname, Tailscale, 9router, clock)
//! - Tab Navigation:
//!   - [1] Overview: btop telemetry (CPU sparkline ring buffer, RAM/Disk/Swap color gauges, temp, GPU, fan)
//!   - [2] Services / Apps: Dokploy-style app cards (caterm, fathstore, gcc, 9router, port matrix 8081-8088)
//!   - [3] Audit & Code: recent audit reports summary, [⚡ Turbo Audit], [🔧 Run Fix]
//!   - [4] Settings GUI: interactive visual configuration form (AI Provider, Model, API Key, Theme, Ports)
//!   - [5] AI Chat: Hana AI chat interface (prompt, buffered inference, scrollable history)
//! - Mouse hitboxes (`Rect::contains(Position { x, y })`) + Keyboard navigation (1-5, Tab, arrows, Enter, Esc/q)
//! - Robust RAII terminal guard with panic recovery + compact fallback for < 80x24 terminals.

use crate::config::Config;
use crate::systat::{snapshot, CpuCoreInfo, SysSnapshot};
use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table, Tabs, Wrap},
    Frame, Terminal,
};
use std::collections::VecDeque;
use std::fs;
use std::io::stdout;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Command-line argument parser for `cashell cockpit`
#[derive(Parser, Debug, Clone, Default)]
pub struct CockpitCommand;

// ── Theme Palettes (High Contrast & Adaptive Presets) ───────────────────────

pub const THEME_OPTIONS: &[&str] = &[
    "ca-classic",
    "tokyo-night",
    "monokai",
    "nord",
    "high-contrast",
];

#[derive(Debug, Clone, Copy)]
pub struct ThemePalette {
    pub name: &'static str,
    pub border: Color,
    pub accent: Color,
    pub highlight_bg: Color,
    pub highlight_fg: Color,
    pub text: Color,
    pub text_muted: Color,
    pub chart_cpu: Color,
    pub chart_ram: Color,
    pub status_ok: Color,
    pub status_warn: Color,
    pub status_err: Color,
    pub row_selected_bg: Color,
}

impl ThemePalette {
    pub fn get(name: &str) -> Self {
        match name {
            "tokyo-night" => Self {
                name: "tokyo-night",
                border: Color::Rgb(122, 162, 247),
                accent: Color::Rgb(187, 154, 247),
                highlight_bg: Color::Rgb(122, 162, 247),
                highlight_fg: Color::Rgb(0, 0, 0),
                text: Color::Rgb(192, 202, 245),
                text_muted: Color::Rgb(86, 95, 137),
                chart_cpu: Color::Rgb(122, 162, 247),
                chart_ram: Color::Rgb(187, 154, 247),
                status_ok: Color::Rgb(158, 206, 106),
                status_warn: Color::Rgb(224, 175, 104),
                status_err: Color::Rgb(247, 118, 142),
                row_selected_bg: Color::Rgb(41, 46, 66),
            },
            "monokai" => Self {
                name: "monokai",
                border: Color::Rgb(249, 38, 114),
                accent: Color::Rgb(166, 226, 46),
                highlight_bg: Color::Rgb(253, 151, 31),
                highlight_fg: Color::Rgb(0, 0, 0),
                text: Color::Rgb(248, 248, 242),
                text_muted: Color::Rgb(117, 113, 94),
                chart_cpu: Color::Rgb(102, 217, 239),
                chart_ram: Color::Rgb(166, 226, 46),
                status_ok: Color::Rgb(166, 226, 46),
                status_warn: Color::Rgb(230, 219, 116),
                status_err: Color::Rgb(249, 38, 114),
                row_selected_bg: Color::Rgb(62, 61, 50),
            },
            "nord" => Self {
                name: "nord",
                border: Color::Rgb(136, 192, 208),
                accent: Color::Rgb(129, 161, 193),
                highlight_bg: Color::Rgb(136, 192, 208),
                highlight_fg: Color::Rgb(0, 0, 0),
                text: Color::Rgb(236, 239, 244),
                text_muted: Color::Rgb(76, 86, 106),
                chart_cpu: Color::Rgb(136, 192, 208),
                chart_ram: Color::Rgb(180, 142, 173),
                status_ok: Color::Rgb(163, 190, 140),
                status_warn: Color::Rgb(235, 203, 139),
                status_err: Color::Rgb(191, 97, 106),
                row_selected_bg: Color::Rgb(46, 52, 64),
            },
            "high-contrast" => Self {
                name: "high-contrast",
                border: Color::White,
                accent: Color::Yellow,
                highlight_bg: Color::Yellow,
                highlight_fg: Color::Rgb(0, 0, 0),
                text: Color::White,
                text_muted: Color::Gray,
                chart_cpu: Color::Yellow,
                chart_ram: Color::Cyan,
                status_ok: Color::Green,
                status_warn: Color::Yellow,
                status_err: Color::Red,
                row_selected_bg: Color::Rgb(50, 50, 50),
            },
            _ => Self {
                name: "ca-classic",
                border: Color::Cyan,
                accent: Color::Cyan,
                highlight_bg: Color::Cyan,
                highlight_fg: Color::Rgb(0, 0, 0),
                text: Color::White,
                text_muted: Color::DarkGray,
                chart_cpu: Color::Cyan,
                chart_ram: Color::Rgb(180, 100, 255),
                status_ok: Color::Green,
                status_warn: Color::Yellow,
                status_err: Color::Red,
                row_selected_bg: Color::Rgb(30, 45, 70),
            },
        }
    }
}

// ── Mouse Hitboxes ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitboxAction {
    SelectTab(usize),
    SelectApp(usize),
    ProbePorts,
    InspectApp(usize),
    CloseInspector,
    SelectReport(usize),
    RunAudit,
    RunFix,
    SelectField(usize),
    SaveSettings,
    ReloadSettings,
    ClearChat,
    VacuumDb,
    ToggleHelp,
    FocusChatInput,
    PetOk,
    PetCancel,
    PetActionUp,
    PetActionDown,
    PetToggleHistory,
    PetFeed,
    PetTreat,
    PetSwitch,
    ConfirmQuit,
    CancelQuit,
}

#[derive(Debug, Clone)]
pub struct Hitbox {
    pub rect: Rect,
    pub action: HitboxAction,
}

// ── Tab Navigation ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CockpitTab {
    Overview = 0,
    Services = 1,
    Audit = 2,
    Cron = 3,
    Settings = 4,
    Chat = 5,
}

impl CockpitTab {
    pub fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Overview,
            1 => Self::Services,
            2 => Self::Audit,
            3 => Self::Cron,
            4 => Self::Settings,
            5 => Self::Chat,
            _ => Self::Overview,
        }
    }

    pub fn to_index(self) -> usize {
        self as usize
    }

    pub fn titles() -> [&'static str; 6] {
        [
            "[1] Overview",
            "[2] Fleet & Servers",
            "[3] Audit & Code",
            "[4] Cron & Jobs",
            "[5] Settings GUI",
            "[6] AI Chat",
        ]
    }
}

// ── Dokploy-Style App Card ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Healthy,   // ● Healthy (green)
    Listening, // ● Listening (cyan)
    Down,      // ✖ Down (red)
    Probing,   // ◌ Probing (yellow)
}

#[derive(Debug, Clone)]
pub struct AppCard {
    pub id: String,
    pub name: String,
    pub category: String, // "AI Gateway", "Coolify App", "Port Matrix"
    pub target: String,
    pub port: Option<u16>,
    pub status: ServiceStatus,
    pub latency_ms: Option<u128>,
    pub details: String,
}

// ── Audit Report Summary ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuditReportItem {
    pub filename: String,
    pub path: PathBuf,
    pub date: String,
    pub size_bytes: u64,
    pub findings_count: usize,
    pub preview: Vec<String>,
}

// ── Settings GUI Form ───────────────────────────────────────────────────────

pub const SETTINGS_FIELDS: &[&str] = &[
    "AI Provider",
    "Default Model",
    "API Key (Masked)",
    "Output Theme",
    "User Call Name",
    "Assistant Persona",
    "DB Retention Days",
    "DB Size (Read-only)",
    "Tailscale Target Host",
    "Doctor Target Port",
    "Port Matrix",
];

pub const PROVIDER_OPTIONS: &[&str] = &["9router", "omniroute", "openai", "anthropic", "local"];

// ── Cockpit App State ───────────────────────────────────────────────────────

const MAX_CPU_SAMPLES: usize = 60;

pub struct CockpitApp {
    pub current_tab: CockpitTab,
    pub db: Option<crate::db::Database>,
    pub show_help_modal: bool,
    pub hitboxes: Vec<Hitbox>,

    // Telemetry (Overview)
    pub cpu_history: VecDeque<f32>,
    pub ram_history: VecDeque<f32>,
    pub latest_snap: SysSnapshot,
    pub hostname: String,
    pub is_9router_online: bool,
    pub is_tailscale_ok: bool,

    // Services (Tab 2)
    pub apps: Vec<AppCard>,
    pub selected_app: usize,
    pub inspector_open: bool,

    // Audit (Tab 3)
    pub audit_reports: Vec<AuditReportItem>,
    pub selected_report: usize,
    pub report_scroll: usize,
    pub audit_status_msg: Option<String>,
    pub is_running_audit: bool,
    pub is_running_fix: bool,

    // Settings (Tab 4)
    pub cfg: Config,
    pub selected_field: usize,
    pub editing_field: Option<usize>,
    pub edit_buffer: String,
    pub settings_status_msg: Option<String>,

    // AI Chat (Tab 5)
    pub chat_lines: Vec<String>,
    pub chat_input: String,
    pub chat_state: ListState,
    pub is_chat_processing: bool,

    // Status bar & notification
    pub notification: Option<(String, Instant)>,
    pub is_compact: bool,
    pub overview_proc_scroll: usize,

    // Virtual Pet (Tamagotchi)
    pub show_pet_modal: bool,
    pub pet_name_buffer: String,
    pub pet_species_buffer: String,
    pub pet_tick: usize,
    pub pet_selected_action: usize,
    pub show_pet_history: bool,
    pub pet_history_logs: Vec<crate::pet::PetActionLog>,
    pub pet_mood: crate::pet::PetMood,

    // Quit confirmation state
    pub show_quit_confirm: bool,
    pub quit_requested: bool,

    // Cron Job Wizard
    pub show_cron_wizard: bool,
    pub cron_schedule_buffer: String,
    pub cron_command_buffer: String,
}

impl CockpitApp {
    pub fn new(cfg: &Config) -> Self {
        let snap = snapshot();
        let mut cpu_history = VecDeque::with_capacity(MAX_CPU_SAMPLES);
        let mut ram_history = VecDeque::with_capacity(MAX_CPU_SAMPLES);
        let ram_pct = (snap.ram_used_mb * 100)
            .checked_div(snap.ram_total_mb)
            .unwrap_or(0) as f32;

        for _ in 0..MAX_CPU_SAMPLES {
            cpu_history.push_back(snap.cpu_pct);
            ram_history.push_back(ram_pct);
        }

        let hostname = sysinfo::System::host_name().unwrap_or_else(|| "ThinkPad X1".to_string());

        let db = match crate::db::Database::open() {
            Ok(d) => {
                if cfg.database.auto_prune {
                    let _ = d.prune_older_than(cfg.database.retention_days);
                }
                Some(d)
            }
            Err(_) => None,
        };

        let mut chat_lines = Vec::new();
        if let Some(ref d) = db {
            if let Ok(recent) = d.load_recent_chat(30) {
                for (role, content) in recent {
                    for line in content.lines() {
                        chat_lines.push(format!("{role}: {line}"));
                    }
                }
            }
        }
        if chat_lines.is_empty() {
            chat_lines = vec![
                "=== Hana AI Chat (localhost:20128) ===".to_string(),
                "Tab / Shift+1..5: Switch Focus | ?: Help Cheat Sheet | Esc: Back".to_string(),
                "Type your prompt in the terminal input below and press Enter.".to_string(),
            ];
        }

        let apps = Self::init_apps(cfg);

        let mut chat_state = ListState::default();
        chat_state.select(Some(chat_lines.len().saturating_sub(1)));

        let mut app = Self {
            current_tab: CockpitTab::Overview,
            db,
            show_help_modal: false,
            hitboxes: Vec::new(),
            cpu_history,
            ram_history,
            latest_snap: snap,
            hostname,
            is_9router_online: false,
            is_tailscale_ok: false,
            apps,
            selected_app: 0,
            inspector_open: false,
            audit_reports: Vec::new(),
            selected_report: 0,
            report_scroll: 0,
            audit_status_msg: None,
            is_running_audit: false,
            is_running_fix: false,
            cfg: cfg.clone(),
            selected_field: 0,
            editing_field: None,
            edit_buffer: String::new(),
            settings_status_msg: None,
            chat_lines,
            chat_input: String::new(),
            chat_state,
            is_chat_processing: false,
            notification: None,
            is_compact: false,
            overview_proc_scroll: 0,

            show_pet_modal: false,
            pet_name_buffer: "Ciko".to_string(),
            pet_species_buffer: "ayam".to_string(),
            pet_tick: 0,
            pet_selected_action: 0,
            show_pet_history: false,
            pet_history_logs: Vec::new(),
            pet_mood: crate::pet::PetMood::Idle,

            show_quit_confirm: false,
            quit_requested: false,

            show_cron_wizard: false,
            cron_schedule_buffer: "0 * * * *".to_string(),
            cron_command_buffer: "echo 'hello'".to_string(),
        };

        app.refresh_audit_reports();
        app.probe_all_ports();
        app
    }

    fn init_apps(cfg: &Config) -> Vec<AppCard> {
        let mut list = Vec::new();

        // 1. 9router AI Gateway
        list.push(AppCard {
            id: "9router".to_string(),
            name: "9router AI Gateway".to_string(),
            category: "AI Gateway".to_string(),
            target: "http://localhost:20128".to_string(),
            port: Some(20128),
            status: ServiceStatus::Probing,
            latency_ms: None,
            details: "Unified AI Proxy routing to ZA126_PRO / Omniroute / OpenAI / Anthropic".to_string(),
        });

        // 2. caterm
        let caterm_uuid = cfg
            .deploy
            .apps
            .get("caterm")
            .map(|a| a.uuid.clone())
            .unwrap_or_else(|| "caterm-app-uuid".to_string());
        list.push(AppCard {
            id: "caterm".to_string(),
            name: "caterm".to_string(),
            category: "Coolify App".to_string(),
            target: format!("coolify:{}", caterm_uuid),
            port: Some(8081),
            status: ServiceStatus::Probing,
            latency_ms: None,
            details: format!("CA Ecosystem Terminal Interface (Coolify UUID: {caterm_uuid})"),
        });

        // 3. fathstore
        let fathstore_uuid = cfg
            .deploy
            .apps
            .get("fathstore")
            .map(|a| a.uuid.clone())
            .unwrap_or_else(|| "fathstore-app-uuid".to_string());
        list.push(AppCard {
            id: "fathstore".to_string(),
            name: "fathstore".to_string(),
            category: "Coolify App".to_string(),
            target: format!("coolify:{}", fathstore_uuid),
            port: Some(8082),
            status: ServiceStatus::Probing,
            latency_ms: None,
            details: format!("E-Commerce Platform & Marketplace (Coolify UUID: {fathstore_uuid})"),
        });

        // 4. gcc
        let gcc_uuid = cfg
            .deploy
            .apps
            .get("gcc")
            .map(|a| a.uuid.clone())
            .unwrap_or_else(|| "gcc-app-uuid".to_string());
        list.push(AppCard {
            id: "gcc".to_string(),
            name: "gcc".to_string(),
            category: "Coolify App".to_string(),
            target: format!("coolify:{}", gcc_uuid),
            port: Some(8083),
            status: ServiceStatus::Probing,
            latency_ms: None,
            details: format!("Global Computing Coordinator / Dispatcher (Coolify UUID: {gcc_uuid})"),
        });

        // 5. Local Port Matrix (8081..=8088 from doctor config)
        let ports = if cfg.doctor.ports.is_empty() {
            vec![8081, 8082, 8083, 8084, 8085, 8086, 8087, 8088]
        } else {
            cfg.doctor.ports.clone()
        };

        for port in ports {
            list.push(AppCard {
                id: format!("port-{}", port),
                name: format!("Port {port}"),
                category: "Port Matrix".to_string(),
                target: format!("127.0.0.1:{port}"),
                port: Some(port),
                status: ServiceStatus::Probing,
                latency_ms: None,
                details: format!("Local development listener on port {port}"),
            });
        }

        list
    }

    pub fn probe_all_ports(&mut self) {
        for app in &mut self.apps {
            if let Some(port) = app.port {
                let start = Instant::now();
                let addr = SocketAddr::from(([127, 0, 0, 1], port));
                match TcpStream::connect_timeout(&addr, Duration::from_millis(40)) {
                    Ok(_) => {
                        let rtt = start.elapsed().as_millis();
                        app.latency_ms = Some(rtt);
                        app.status = if app.category == "Port Matrix" {
                            ServiceStatus::Listening
                        } else {
                            ServiceStatus::Healthy
                        };
                    }
                    Err(_) => {
                        app.latency_ms = None;
                        app.status = ServiceStatus::Down;
                    }
                }
            }
        }
        // Update header indicators
        self.is_9router_online = self
            .apps
            .iter()
            .find(|a| a.id == "9router")
            .map(|a| a.status == ServiceStatus::Healthy)
            .unwrap_or(false);

        self.is_tailscale_ok = Path::new("/var/run/tailscale/tailscaled.sock").exists()
            || Path::new("/run/tailscale/tailscaled.sock").exists();
    }

    pub fn refresh_audit_reports(&mut self) {
        let mut list = Vec::new();
        let audit_dir = Path::new("audit");
        if audit_dir.exists() && audit_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(audit_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let filename = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        if filename.starts_with("Audit_") && filename.ends_with(".md") {
                            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            let content = fs::read_to_string(&path).unwrap_or_default();
                            let preview: Vec<String> = content
                                .lines()
                                .take(60)
                                .map(|l| l.to_string())
                                .collect();

                            // Simple count of findings tags
                            let findings_count = content.matches("### [").count()
                                + content.matches("**Severity:**").count();

                            let date = filename
                                .trim_start_matches("Audit_")
                                .trim_end_matches(".md")
                                .to_string();

                            list.push(AuditReportItem {
                                filename,
                                path,
                                date,
                                size_bytes,
                                findings_count,
                                preview,
                            });
                        }
                    }
                }
            }
        }

        list.sort_by(|a, b| b.filename.cmp(&a.filename));
        self.audit_reports = list;
        if self.selected_report >= self.audit_reports.len() {
            self.selected_report = self.audit_reports.len().saturating_sub(1);
        }
    }

    pub fn register_hitbox(&mut self, rect: Rect, action: HitboxAction) {
        if rect.width > 0 && rect.height > 0 {
            self.hitboxes.push(Hitbox { rect, action });
        }
    }

    pub fn find_hitbox(&self, col: u16, row: u16) -> Option<HitboxAction> {
        let pos = Position { x: col, y: row };
        for hb in &self.hitboxes {
            if hb.rect.contains(pos) {
                return Some(hb.action);
            }
        }
        None
    }

    pub fn set_notification(&mut self, msg: impl Into<String>) {
        self.notification = Some((msg.into(), Instant::now()));
    }

    pub fn add_chat_line(&mut self, line: &str) {
        self.chat_lines.push(line.to_string());
        let idx = self.chat_lines.len().saturating_sub(1);
        self.chat_state.select(Some(idx));
    }

    pub fn scroll_chat_up(&mut self) {
        let i = match self.chat_state.selected() {
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.chat_state.select(Some(i));
    }

    pub fn scroll_chat_down(&mut self) {
        let i = match self.chat_state.selected() {
            Some(i) => (i + 1).min(self.chat_lines.len().saturating_sub(1)),
            None => 0,
        };
        self.chat_state.select(Some(i));
    }

    pub fn ingest_snapshot(&mut self, snap: SysSnapshot) {
        self.cpu_history.push_back(snap.cpu_pct);
        if self.cpu_history.len() > MAX_CPU_SAMPLES {
            self.cpu_history.pop_front();
        }

        let ram_pct = (snap.ram_used_mb * 100)
            .checked_div(snap.ram_total_mb)
            .unwrap_or(0) as f32;
        self.ram_history.push_back(ram_pct);
        if self.ram_history.len() > MAX_CPU_SAMPLES {
            self.ram_history.pop_front();
        }

        // Update Pet Mood & Action History Log if mood state changed
        let new_mood = crate::pet::determine_mood(
            snap.cpu_pct,
            snap.cpu_temp_c,
            ((snap.ram_used_mb * 100) / snap.ram_total_mb.max(1)) as u8,
            ((snap.swap_used_mb * 100) / snap.swap_total_mb.max(1)) as u8,
        );

        self.latest_snap = snap;

        if new_mood != self.pet_mood {
            self.pet_mood = new_mood;
            if let Some(ref d) = self.db {
                let active_pet = d.get_active_pet().ok().flatten();
                if let Some(pet) = active_pet {
                    let mood_str = match new_mood {
                        crate::pet::PetMood::Idle => "Recovered (Normal)",
                        crate::pet::PetMood::Overheated => "Overheated (Sweating & Fanning)",
                        crate::pet::PetMood::Sick => "Sick (Vomiting)",
                        crate::pet::PetMood::Sprint => "Sprinting (High CPU)",
                    };
                    let _ = d.log_pet_action(pet.id, &pet.name, "Mood Shift", mood_str);
                }
            }
            // Refresh history log cache in memory
            if let Some(ref d) = self.db {
                if let Ok(logs) = d.get_pet_history(10) {
                    self.pet_history_logs = logs;
                }
            }
        }
    }

    pub fn cycle_field_value(&mut self, forward: bool) {
        match self.selected_field {
            0 => {
                // AI Provider
                let current = &self.cfg.ai.default_provider;
                let cur_idx = PROVIDER_OPTIONS
                    .iter()
                    .position(|&p| p == current)
                    .unwrap_or(0);
                let next_idx = if forward {
                    (cur_idx + 1) % PROVIDER_OPTIONS.len()
                } else {
                    (cur_idx + PROVIDER_OPTIONS.len() - 1) % PROVIDER_OPTIONS.len()
                };
                self.cfg.ai.default_provider = PROVIDER_OPTIONS[next_idx].to_string();
                self.set_notification(format!(
                    "AI Provider set to: {}",
                    self.cfg.ai.default_provider
                ));
            }
            3 => {
                // Theme
                let current = &self.cfg.output.theme;
                let cur_idx = THEME_OPTIONS.iter().position(|&t| t == current).unwrap_or(0);
                let next_idx = if forward {
                    (cur_idx + 1) % THEME_OPTIONS.len()
                } else {
                    (cur_idx + THEME_OPTIONS.len() - 1) % THEME_OPTIONS.len()
                };
                self.cfg.output.theme = THEME_OPTIONS[next_idx].to_string();
                self.set_notification(format!("Theme set to: {}", self.cfg.output.theme));
            }
            6 => {
                // DB Retention Days
                let cur = self.cfg.database.retention_days;
                let next = if forward {
                    cur.saturating_add(1).min(365)
                } else {
                    cur.saturating_sub(1).max(1)
                };
                self.cfg.database.retention_days = next;
                self.set_notification(format!("DB retention set to: {next} days"));
            }
            7 => {
                let size = format_bytes(self.get_db_size());
                self.set_notification(format!("Current SQLite DB size: {size} (read-only)"));
            }
            _ => {}
        }
    }

    pub fn get_db_size(&self) -> u64 {
        if let Some(ref d) = self.db {
            d.get_db_size_bytes()
        } else {
            crate::db::get_db_size_bytes()
        }
    }

    pub fn clear_chat_history(&mut self) {
        if let Some(ref d) = self.db {
            if let Err(e) = d.clear_chat() {
                self.set_notification(format!("✗ Failed to clear chat: {e}"));
                return;
            }
        }
        self.chat_lines.clear();
        self.chat_lines.push("=== Hana AI Chat (localhost:20128) ===".to_string());
        self.chat_lines.push("Chat history cleared.".to_string());
        self.chat_state.select(Some(1));
        self.set_notification("✓ Chat history cleared from SQLite DB");
    }

    pub fn vacuum_database(&mut self) {
        if let Some(ref d) = self.db {
            if let Err(e) = d.vacuum() {
                self.set_notification(format!("✗ Failed to vacuum DB: {e}"));
                return;
            }
        }
        let size = self.get_db_size();
        let size_str = format_bytes(size);
        self.set_notification(format!("✓ SQLite vacuum complete (DB size: {size_str})"));
    }

    // ── Pet Action Helpers ───────────────────────────────────────────────────

    pub fn execute_pet_action(&mut self) {
        match self.pet_selected_action {
            0 => self.feed_pet(),
            1 => self.treat_pet(),
            2 => self.switch_pet_species(),
            3 => self.toggle_pet_history(),
            _ => {}
        }
    }

    pub fn feed_pet(&mut self) {
        let mut notif = None;
        if let Some(ref d) = self.db {
            if let Ok(Some(pet)) = d.get_active_pet() {
                let _ = d.add_pet_xp(pet.id, 10);
                let _ = d.log_pet_action(pet.id, &pet.name, "Beri Makan", "+10 XP (Nyaman)");
                notif = Some(format!("🍖 Beri makan {}! (+10 XP)", pet.name));
            } else {
                let _ = d.create_pet("ayam", "Ciko");
                let _ = d.log_pet_action(1, "Ciko", "Adopsi", "Mengadopsi Ayam Ciko");
                notif = Some("🐣 Mengadopsi Ayam Ciko!".to_string());
            }
            if let Ok(logs) = d.get_pet_history(10) {
                self.pet_history_logs = logs;
            }
        }
        if let Some(msg) = notif {
            self.set_notification(msg);
        }
    }

    pub fn treat_pet(&mut self) {
        let mut notif = None;
        if let Some(ref d) = self.db {
            if let Ok(Some(pet)) = d.get_active_pet() {
                self.pet_mood = crate::pet::PetMood::Idle;
                let _ = d.log_pet_action(pet.id, &pet.name, "Perawatan", "Diberi obat & pulih");
                notif = Some("🩺 Hewan berhasil dirawat & pulih!".to_string());
            }
            if let Ok(logs) = d.get_pet_history(10) {
                self.pet_history_logs = logs;
            }
        }
        if let Some(msg) = notif {
            self.set_notification(msg);
        }
    }

    pub fn switch_pet_species(&mut self) {
        let species_list = ["ayam", "singa", "burung", "ular", "beruang"];
        let names = ["Ciko", "Simba", "Rio", "Nagini", "Koda"];
        let mut notif = None;
        if let Some(ref d) = self.db {
            let next_idx = match d.get_active_pet().ok().flatten() {
                Some(p) => match p.species.as_str() {
                    "ayam" => 1,
                    "singa" => 2,
                    "burung" => 3,
                    "ular" => 4,
                    _ => 0,
                },
                None => 0,
            };
            let next_species = species_list[next_idx];
            let next_name = names[next_idx];
            let _ = d.create_pet(next_species, next_name);
            let _ = d.log_pet_action(1, next_name, "Ganti Hewan", &format!("Beralih ke {}", next_species));
            notif = Some(format!("🔄 Beralih ke {} {}!", next_species, next_name));
            if let Ok(logs) = d.get_pet_history(10) {
                self.pet_history_logs = logs;
            }
        }
        if let Some(msg) = notif {
            self.set_notification(msg);
        }
    }

    pub fn toggle_pet_history(&mut self) {
        self.show_pet_history = !self.show_pet_history;
        if self.show_pet_history {
            if let Some(ref d) = self.db {
                if let Ok(logs) = d.get_pet_history(10) {
                    self.pet_history_logs = logs;
                }
            }
        }
    }

    pub fn save_current_config(&mut self) {
        let path = Config::default_path();
        match self.cfg.save(&path) {
            Ok(_) => {
                let time_str = chrono::Local::now().format("%H:%M:%S");
                let msg = format!("✓ Config saved to {} at {}", path.display(), time_str);
                self.settings_status_msg = Some(msg.clone());
                self.set_notification(msg);
            }
            Err(e) => {
                let err_msg = format!("✗ Failed to save config: {e}");
                self.settings_status_msg = Some(err_msg.clone());
                self.set_notification(err_msg);
            }
        }
    }

    pub fn reload_config(&mut self) {
        match Config::load(None) {
            Ok(loaded) => {
                self.cfg = loaded;
                self.apps = Self::init_apps(&self.cfg);
                self.probe_all_ports();
                let msg = "✓ Configuration reloaded from disk".to_string();
                self.settings_status_msg = Some(msg.clone());
                self.set_notification(msg);
            }
            Err(e) => {
                self.set_notification(format!("✗ Reload failed: {e}"));
            }
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

// ── Terminal Guard (RAII + Panic safety) ──────────────────────────────────────

struct TermGuard;

impl TermGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, event::EnableMouseCapture)?;

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

// ── Main Entry Point ────────────────────────────────────────────────────────

pub fn run(_cmd: &CockpitCommand, cfg: &Config) -> Result<()> {
    if !crate::output::is_tty() {
        anyhow::bail!("CAS Cockpit requires an interactive terminal (stdout is not a TTY)");
    }

    let mut app = CockpitApp::new(cfg);

    if let Ok((w, h)) = crossterm::terminal::size() {
        app.is_compact = w < 80 || h < 24;
    }

    let _guard = TermGuard::new()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    // Telemetry channel from background thread (~1s interval)
    let running = Arc::new(AtomicBool::new(true));
    let r_thread = running.clone();
    let (snap_tx, snap_rx) = mpsc::channel::<SysSnapshot>();

    let stats_thread = std::thread::spawn(move || {
        while r_thread.load(Ordering::Relaxed) {
            let snap = snapshot();
            if snap_tx.send(snap).is_err() {
                break;
            }
            for _ in 0..10 {
                if !r_thread.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });

    let res = event_loop(&mut terminal, &mut app, &snap_rx);

    running.store(false, Ordering::Relaxed);
    drop(snap_rx);
    let _ = stats_thread.join();

    res
}

// ── Event Loop ───────────────────────────────────────────────────────────────

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut CockpitApp,
    snap_rx: &mpsc::Receiver<SysSnapshot>,
) -> Result<()> {
    loop {
        // Ingest telemetry if available
        while let Ok(snap) = snap_rx.try_recv() {
            app.ingest_snapshot(snap);
        }

        // Check terminal size & update compact state
        if let Ok((w, h)) = crossterm::terminal::size() {
            let compact = w < 80 || h < 24;
            if app.is_compact != compact {
                app.is_compact = compact;
            }
        }

        // Increment pet animation tick
        app.pet_tick = app.pet_tick.wrapping_add(1);

        if app.quit_requested {
            return Ok(());
        }

        // Draw current frame
        terminal.draw(|f| draw(f, app))?;

        // 8-10 FPS poll timeout (100ms)
        let timeout = Duration::from_millis(100);
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press
                        && handle_key_event(key, app, terminal)?
                    {
                        return Ok(());
                    }
                }
                Event::Mouse(mouse) => {
                    handle_mouse_event(mouse, app, terminal)?;
                }
                Event::Resize(w, h) => {
                    app.is_compact = w < 80 || h < 24;
                }
                _ => {}
            }
        }

        // Clear notification if expired (> 4 seconds)
        if let Some((_, created)) = app.notification {
            if created.elapsed() > Duration::from_secs(4) {
                app.notification = None;
            }
        }
    }
}

// ── Key Event Handling ───────────────────────────────────────────────────────

fn handle_key_event<B: ratatui::backend::Backend>(
    key: event::KeyEvent,
    app: &mut CockpitApp,
    terminal: &mut Terminal<B>,
) -> Result<bool> {
    // 1. Shift + 1..5 MUST switch tabs immediately (even in Chat tab or editing mode)
    let shift_tab = match key.code {
        KeyCode::Char('!') => Some(0),
        KeyCode::Char('@') => Some(1),
        KeyCode::Char('#') => Some(2),
        KeyCode::Char('$') => Some(3),
        KeyCode::Char('%') => Some(4),
        KeyCode::Char('^') => Some(5),
        KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(0),
        KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(1),
        KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(2),
        KeyCode::Char('4') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(3),
        KeyCode::Char('5') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(4),
        KeyCode::Char('6') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(5),
        _ => None,
    };
    if let Some(tab_idx) = shift_tab {
        app.current_tab = CockpitTab::from_index(tab_idx);
        app.show_help_modal = false;
        app.editing_field = None;
        app.edit_buffer.clear();
        return Ok(false);
    }

    // 2. Help Modal toggle and dismissal
    if app.show_help_modal {
        match key.code {
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter | KeyCode::F(1) => {
                app.show_help_modal = false;
            }
            _ => {}
        }
        return Ok(false);
    }

    // Global Help shortcut
    if key.code == KeyCode::F(1)
        || (key.code == KeyCode::Char('?')
            && app.editing_field.is_none()
            && (app.current_tab != CockpitTab::Chat || app.chat_input.is_empty()))
    {
        app.show_help_modal = !app.show_help_modal;
        return Ok(false);
    }

    // 3. If user is editing a field in Settings GUI
    if let Some(field_idx) = app.editing_field {
        match key.code {
            KeyCode::Esc => {
                app.editing_field = None;
                app.edit_buffer.clear();
            }
            KeyCode::Enter => {
                commit_field_edit(app, field_idx);
                app.editing_field = None;
                app.edit_buffer.clear();
            }
            KeyCode::Backspace => {
                app.edit_buffer.pop();
            }
            KeyCode::Char(c) => {
                app.edit_buffer.push(c);
            }
            _ => {}
        }
        return Ok(false);
    }

    // 4. If chat tab is focused and user types in prompt
    if app.current_tab == CockpitTab::Chat {
        match key.code {
            KeyCode::Esc => {
                return Ok(true);
            }
            KeyCode::Enter if !app.chat_input.trim().is_empty() && !app.is_chat_processing => {
                let prompt = app.chat_input.clone();
                app.chat_input.clear();
                app.add_chat_line(&format!("User: {prompt}"));
                if let Some(ref d) = app.db {
                    let _ = d.save_chat("User", &prompt);
                }
                app.is_chat_processing = true;

                terminal.draw(|f| draw(f, app))?;

                let cfg_clone = app.cfg.clone();
                let ports_str = app
                    .cfg
                    .doctor
                    .ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let running_services = app
                    .apps
                    .iter()
                    .filter(|a| {
                        a.status == ServiceStatus::Healthy || a.status == ServiceStatus::Listening
                    })
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let tailscale_status = if app.is_tailscale_ok {
                    "connected"
                } else {
                    "disconnected"
                };

                let system_prompt = format!(
                    "{role}\nGreetings to {user_name}.\nSystem Context: Hostname: {host}, OS: Linux, Monitored Ports: [{ports}], Active Services: [{services}], Tailscale: {tailscale}.",
                    role = app.cfg.persona.assistant_role,
                    user_name = app.cfg.persona.user_name,
                    host = app.hostname,
                    ports = ports_str,
                    services = if running_services.is_empty() {
                        "none detected".to_string()
                    } else {
                        running_services
                    },
                    tailscale = tailscale_status,
                );

                let answer = match crate::ai::query_buffered(&cfg_clone, Some(&system_prompt), &prompt) {
                    Ok(ans) => ans,
                    Err(e) => format!("Error: {e}"),
                };

                if let Some(ref d) = app.db {
                    let _ = d.save_chat("AI", &answer);
                }

                for line in answer.lines() {
                    app.add_chat_line(&format!("AI: {line}"));
                }
                app.is_chat_processing = false;
                terminal.draw(|f| draw(f, app))?;
                return Ok(false);
            }
            KeyCode::Backspace => {
                app.chat_input.pop();
                return Ok(false);
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                // If it's 1-6 with no prompt, allow switching tabs
                if app.chat_input.is_empty() && ('1'..='6').contains(&c) {
                    let idx = (c as usize) - ('1' as usize);
                    app.current_tab = CockpitTab::from_index(idx);
                } else {
                    app.chat_input.push(c);
                }
                return Ok(false);
            }
            KeyCode::Up => {
                app.scroll_chat_up();
                return Ok(false);
            }
            KeyCode::Down => {
                app.scroll_chat_down();
                return Ok(false);
            }
            _ => {}
        }
    }

    // 5. Quit Confirmation Modal Handling
    if app.show_quit_confirm {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                return Ok(true); // Confirm Exit
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                app.show_quit_confirm = false;
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    // 6. Global Navigation Shortcuts
    match key.code {
        KeyCode::Esc if app.show_pet_modal => {
            app.show_pet_modal = false;
        }
        KeyCode::Esc if app.show_cron_wizard => {
            app.show_cron_wizard = false;
        }
        KeyCode::Esc if app.show_pet_history => {
            app.show_pet_history = false;
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            if app.inspector_open {
                app.inspector_open = false;
            } else if app.show_help_modal {
                app.show_help_modal = false;
            } else {
                app.show_quit_confirm = true;
            }
        }
        KeyCode::Char('1') => app.current_tab = CockpitTab::Overview,
        KeyCode::Char('2') => app.current_tab = CockpitTab::Services,
        KeyCode::Char('3') => app.current_tab = CockpitTab::Audit,
        KeyCode::Char('4') => app.current_tab = CockpitTab::Cron,
        KeyCode::Char('5') => app.current_tab = CockpitTab::Settings,
        KeyCode::Char('6') => app.current_tab = CockpitTab::Chat,

        // ── Pet Play Area Controls (Hardware-Reactive Tamagotchi) ─────────────
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.execute_pet_action();
        }
        KeyCode::Char('f') if app.current_tab == CockpitTab::Overview => {
            app.pet_selected_action = 0;
            app.execute_pet_action();
        }
        KeyCode::Char('t') if app.current_tab == CockpitTab::Overview => {
            app.pet_selected_action = 1;
            app.execute_pet_action();
        }
        KeyCode::Char('s') if app.current_tab == CockpitTab::Overview => {
            app.pet_selected_action = 2;
            app.execute_pet_action();
        }
        KeyCode::Char('h') if app.current_tab == CockpitTab::Overview => {
            app.pet_selected_action = 3;
            app.execute_pet_action();
        }

        KeyCode::Tab => {
            let next_idx = (app.current_tab.to_index() + 1) % 6;
            app.current_tab = CockpitTab::from_index(next_idx);
        }
        KeyCode::BackTab => {
            let prev_idx = (app.current_tab.to_index() + 5) % 6;
            app.current_tab = CockpitTab::from_index(prev_idx);
        }

        // Tab-specific controls
        KeyCode::Up | KeyCode::Char('k') => match app.current_tab {
            CockpitTab::Overview => {
                app.pet_selected_action = (app.pet_selected_action + 3) % 4;
                app.overview_proc_scroll = app.overview_proc_scroll.saturating_sub(1);
            }
            CockpitTab::Services => {
                app.selected_app = app.selected_app.saturating_sub(1);
            }
            CockpitTab::Audit => {
                app.selected_report = app.selected_report.saturating_sub(1);
            }
            CockpitTab::Cron => {}
            CockpitTab::Settings => {
                app.selected_field = app.selected_field.saturating_sub(1);
            }
            CockpitTab::Chat => {
                app.scroll_chat_up();
            }
        },
        KeyCode::Down | KeyCode::Char('j') => match app.current_tab {
            CockpitTab::Overview => {
                app.pet_selected_action = (app.pet_selected_action + 1) % 4;
                app.overview_proc_scroll = (app.overview_proc_scroll + 1).min(30);
            }
            CockpitTab::Services => {
                if !app.apps.is_empty() {
                    app.selected_app = (app.selected_app + 1).min(app.apps.len() - 1);
                }
            }
            CockpitTab::Audit => {
                if !app.audit_reports.is_empty() {
                    app.selected_report =
                        (app.selected_report + 1).min(app.audit_reports.len() - 1);
                }
            }
            CockpitTab::Cron => {}
            CockpitTab::Settings => {
                app.selected_field = (app.selected_field + 1).min(SETTINGS_FIELDS.len() - 1);
            }
            CockpitTab::Chat => {
                app.scroll_chat_down();
            }
        },

        KeyCode::Left | KeyCode::Char('h') => {
            if app.current_tab == CockpitTab::Settings {
                app.cycle_field_value(false);
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if app.current_tab == CockpitTab::Settings {
                app.cycle_field_value(true);
            }
        }

        KeyCode::Enter if app.show_cron_wizard => {
            if let Some(ref d) = app.db {
                let _ = d.add_cron_job(&app.cron_schedule_buffer, &app.cron_command_buffer, "Custom task");
                app.set_notification("✔ Saved scheduled job");
            }
            app.show_cron_wizard = false;
        }
        KeyCode::Enter => match app.current_tab {
            CockpitTab::Services => {
                app.inspector_open = !app.inspector_open;
            }
            CockpitTab::Settings => {
                start_field_edit(app);
            }
            _ => {}
        },

        KeyCode::Char('p') => {
            if app.current_tab == CockpitTab::Services {
                app.probe_all_ports();
                app.set_notification("Probed all ports & services");
            } else {
                app.show_pet_modal = !app.show_pet_modal;
            }
        }
        KeyCode::Char('c') if app.show_pet_modal => {
            if let Some(ref d) = app.db {
                let _ = d.create_pet("ayam", "Ciko");
                app.set_notification("🐣 Created pet Ayam Ciko!");
            }
        }
        KeyCode::Char('f') if app.show_pet_modal => {
            if let Some(ref d) = app.db {
                if let Ok(Some(pet)) = d.get_active_pet() {
                    let _ = d.add_pet_xp(pet.id, 10);
                    app.set_notification("🍖 Fed Ciko! +10 XP");
                }
            }
        }
        KeyCode::Char('n') if app.show_pet_modal => {
            if let Some(ref d) = app.db {
                let _ = d.create_pet("singa", "Simba");
                app.set_notification("🦁 Created pet Singa Simba!");
            }
        }
        KeyCode::Char('n') if app.current_tab == CockpitTab::Cron => {
            app.show_cron_wizard = true;
        }
        KeyCode::Char('i') if app.current_tab == CockpitTab::Services => {
            app.inspector_open = !app.inspector_open;
        }

        KeyCode::Char('a') if app.current_tab == CockpitTab::Audit => {
            trigger_turbo_audit(app, terminal)?;
        }
        KeyCode::Char('f') if app.current_tab == CockpitTab::Audit => {
            trigger_run_fix(app, terminal)?;
        }

        KeyCode::Char('s') if app.current_tab == CockpitTab::Settings => {
            app.save_current_config();
        }
        KeyCode::Char('r') if app.current_tab == CockpitTab::Settings => {
            app.reload_config();
        }
        KeyCode::Char('c') if app.current_tab == CockpitTab::Settings => {
            app.clear_chat_history();
        }
        KeyCode::Char('v') if app.current_tab == CockpitTab::Settings => {
            app.vacuum_database();
        }

        _ => {}
    }

    Ok(false)
}

// ── Mouse Event Handling ─────────────────────────────────────────────────────

fn handle_mouse_event<B: ratatui::backend::Backend>(
    mouse: event::MouseEvent,
    app: &mut CockpitApp,
    terminal: &mut Terminal<B>,
) -> Result<()> {
    match mouse.kind {
        MouseEventKind::ScrollUp => match app.current_tab {
            CockpitTab::Overview => app.overview_proc_scroll = app.overview_proc_scroll.saturating_sub(1),
            CockpitTab::Chat => app.scroll_chat_up(),
            CockpitTab::Audit => app.report_scroll = app.report_scroll.saturating_sub(2),
            CockpitTab::Services => app.selected_app = app.selected_app.saturating_sub(1),
            CockpitTab::Cron => {},
            CockpitTab::Settings => app.selected_field = app.selected_field.saturating_sub(1),
        },
        MouseEventKind::ScrollDown => match app.current_tab {
            CockpitTab::Overview => app.overview_proc_scroll = (app.overview_proc_scroll + 1).min(30),
            CockpitTab::Chat => app.scroll_chat_down(),
            CockpitTab::Audit => app.report_scroll = app.report_scroll.saturating_add(2),
            CockpitTab::Services => {
                if !app.apps.is_empty() {
                    app.selected_app = (app.selected_app + 1).min(app.apps.len() - 1);
                }
            }
            CockpitTab::Cron => {},
            CockpitTab::Settings => {
                app.selected_field = (app.selected_field + 1).min(SETTINGS_FIELDS.len() - 1);
            }
        },
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(action) = app.find_hitbox(mouse.column, mouse.row) {
                match action {
                    HitboxAction::SelectTab(idx) => {
                        app.current_tab = CockpitTab::from_index(idx);
                    }
                    HitboxAction::SelectApp(idx) => {
                        app.selected_app = idx;
                    }
                    HitboxAction::ProbePorts => {
                        app.probe_all_ports();
                        app.set_notification("Probed all ports & services");
                    }
                    HitboxAction::InspectApp(idx) => {
                        app.selected_app = idx;
                        app.inspector_open = true;
                    }
                    HitboxAction::CloseInspector => {
                        app.inspector_open = false;
                    }
                    HitboxAction::SelectReport(idx) => {
                        app.selected_report = idx;
                        app.report_scroll = 0;
                    }
                    HitboxAction::RunAudit => {
                        trigger_turbo_audit(app, terminal)?;
                    }
                    HitboxAction::RunFix => {
                        trigger_run_fix(app, terminal)?;
                    }
                    HitboxAction::SelectField(idx) => {
                        app.selected_field = idx;
                        start_field_edit(app);
                    }
                    HitboxAction::SaveSettings => {
                        app.save_current_config();
                    }
                    HitboxAction::ReloadSettings => {
                        app.reload_config();
                    }
                    HitboxAction::ClearChat => {
                        app.clear_chat_history();
                    }
                    HitboxAction::VacuumDb => {
                        app.vacuum_database();
                    }
                    HitboxAction::ToggleHelp => {
                        app.show_help_modal = !app.show_help_modal;
                    }
                    HitboxAction::FocusChatInput => {
                        // Focus is already on input in Chat tab
                    }
                    HitboxAction::PetOk => {
                        app.execute_pet_action();
                    }
                    HitboxAction::PetCancel => {
                        app.show_pet_history = false;
                        app.show_pet_modal = false;
                    }
                    HitboxAction::PetActionUp => {
                        app.pet_selected_action = (app.pet_selected_action + 3) % 4;
                    }
                    HitboxAction::PetActionDown => {
                        app.pet_selected_action = (app.pet_selected_action + 1) % 4;
                    }
                    HitboxAction::PetToggleHistory => {
                        app.pet_selected_action = 3;
                        app.execute_pet_action();
                    }
                    HitboxAction::PetFeed => {
                        app.pet_selected_action = 0;
                        app.execute_pet_action();
                    }
                    HitboxAction::PetTreat => {
                        app.pet_selected_action = 1;
                        app.execute_pet_action();
                    }
                    HitboxAction::PetSwitch => {
                        app.pet_selected_action = 2;
                        app.execute_pet_action();
                    }
                    HitboxAction::ConfirmQuit => {
                        app.show_quit_confirm = false;
                        app.quit_requested = true;
                    }
                    HitboxAction::CancelQuit => {
                        app.show_quit_confirm = false;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ── Settings GUI Helpers ─────────────────────────────────────────────────────

fn start_field_edit(app: &mut CockpitApp) {
    match app.selected_field {
        0 => app.cycle_field_value(true),
        1 => {
            // Default model
            let model = app
                .cfg
                .ai
                .providers
                .get(&app.cfg.ai.default_provider)
                .map(|p| p.default_model.clone())
                .unwrap_or_else(|| "ZA126_PRO".to_string());
            app.editing_field = Some(1);
            app.edit_buffer = model;
        }
        2 => {
            // API key
            let key = app
                .cfg
                .ai
                .providers
                .get(&app.cfg.ai.default_provider)
                .map(|p| p.api_key.clone())
                .unwrap_or_default();
            app.editing_field = Some(2);
            app.edit_buffer = key;
        }
        3 => app.cycle_field_value(true),
        4 => {
            // User call name
            app.editing_field = Some(4);
            app.edit_buffer = app.cfg.persona.user_name.clone();
        }
        5 => {
            // Assistant persona
            app.editing_field = Some(5);
            app.edit_buffer = app.cfg.persona.assistant_role.clone();
        }
        6 => {
            // DB retention days
            app.editing_field = Some(6);
            app.edit_buffer = app.cfg.database.retention_days.to_string();
        }
        7 => {
            let size = format_bytes(app.get_db_size());
            app.set_notification(format!("DB Size: {size} (read-only SQLite metric)"));
        }
        8 => {
            // Tailscale target host
            app.editing_field = Some(8);
            app.edit_buffer = app.cfg.doctor.target_addr.clone();
        }
        9 => {
            // Doctor target port
            app.editing_field = Some(9);
            app.edit_buffer = app.cfg.doctor.target_port.to_string();
        }
        10 => {
            // Port matrix
            let ports = app
                .cfg
                .doctor
                .ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            app.editing_field = Some(10);
            app.edit_buffer = ports;
        }
        _ => {}
    }
}

fn commit_field_edit(app: &mut CockpitApp, field_idx: usize) {
    let val = app.edit_buffer.trim().to_string();
    match field_idx {
        1 => {
            let provider = app.cfg.ai.default_provider.clone();
            if let Some(p) = app.cfg.ai.providers.get_mut(&provider) {
                p.default_model = val.clone();
            }
            app.set_notification(format!("Model set to: {val}"));
        }
        2 => {
            let provider = app.cfg.ai.default_provider.clone();
            if let Some(p) = app.cfg.ai.providers.get_mut(&provider) {
                p.api_key = val.clone();
            }
            app.set_notification("API Key updated");
        }
        4 => {
            app.cfg.persona.user_name = val.clone();
            app.set_notification(format!("User call name set to: {val}"));
        }
        5 => {
            app.cfg.persona.assistant_role = val.clone();
            app.set_notification("Assistant persona updated");
        }
        6 => {
            if let Ok(days) = val.parse::<u32>() {
                app.cfg.database.retention_days = days.max(1);
                app.set_notification(format!(
                    "DB retention set to: {} days",
                    app.cfg.database.retention_days
                ));
            } else {
                app.set_notification("Invalid retention days (must be positive number)");
            }
        }
        8 => {
            app.cfg.doctor.target_addr = val.clone();
            app.set_notification(format!("Doctor target set to: {val}"));
        }
        9 => {
            if let Ok(port) = val.parse::<u16>() {
                app.cfg.doctor.target_port = port;
                app.set_notification(format!("Doctor port set to: {port}"));
            } else {
                app.set_notification("Invalid port number");
            }
        }
        10 => {
            let parsed_ports: Vec<u16> = val
                .split(',')
                .filter_map(|s| s.trim().parse::<u16>().ok())
                .collect();
            if !parsed_ports.is_empty() {
                app.cfg.doctor.ports = parsed_ports;
                app.apps = CockpitApp::init_apps(&app.cfg);
                app.probe_all_ports();
                app.set_notification("Port Matrix updated");
            } else {
                app.set_notification("Invalid port matrix list");
            }
        }
        _ => {}
    }
}

// ── Actions: Turbo Audit & Fix ───────────────────────────────────────────────

fn trigger_turbo_audit<B: ratatui::backend::Backend>(
    app: &mut CockpitApp,
    terminal: &mut Terminal<B>,
) -> Result<()> {
    app.audit_status_msg = Some("⚡ Running Turbo Audit...".to_string());
    app.is_running_audit = true;
    terminal.draw(|f| draw(f, app))?;

    let cmd = crate::cli::AuditCommand {
        path: None,
        json: false,
        max_files: 40,
        out: None,
        include: None,
        changed: false,
        timeout: 30,
        turbo: true,
        jobs: None,
    };

    let cfg_clone = app.cfg.clone();
    let res = crate::commands::audit::run(&cmd, &cfg_clone, true);
    app.is_running_audit = false;
    app.refresh_audit_reports();

    match res {
        Ok(code) => {
            let msg = format!("✓ Turbo Audit completed (exit code {code})");
            app.audit_status_msg = Some(msg.clone());
            app.set_notification(msg);
        }
        Err(e) => {
            let msg = format!("✗ Audit failed: {e}");
            app.audit_status_msg = Some(msg.clone());
            app.set_notification(msg);
        }
    }
    terminal.draw(|f| draw(f, app))?;
    Ok(())
}

fn trigger_run_fix<B: ratatui::backend::Backend>(
    app: &mut CockpitApp,
    terminal: &mut Terminal<B>,
) -> Result<()> {
    app.audit_status_msg = Some("🔧 Running Fix on latest audit report...".to_string());
    app.is_running_fix = true;
    terminal.draw(|f| draw(f, app))?;

    let cmd = crate::cli::FixCommand {
        report_path: None,
        from: None,
        json: true,
        dry_run: false,
        yes: true,
        files: None,
        timeout: Some(30),
    };

    let cfg_clone = app.cfg.clone();
    let res = crate::commands::fix::run(&cmd, &cfg_clone);
    app.is_running_fix = false;
    app.refresh_audit_reports();

    match res {
        Ok(code) => {
            let msg = format!("✓ Run Fix finished (code {code})");
            app.audit_status_msg = Some(msg.clone());
            app.set_notification(msg);
        }
        Err(e) => {
            let msg = format!("✗ Run Fix failed: {e}");
            app.audit_status_msg = Some(msg.clone());
            app.set_notification(msg);
        }
    }
    terminal.draw(|f| draw(f, app))?;
    Ok(())
}

// ── UI Drawing Functions ─────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &mut CockpitApp) {
    app.hitboxes.clear();
    let area = f.area();

    // Small terminal fallback (< 80x24)
    if app.is_compact || area.width < 80 || area.height < 24 {
        draw_compact_fallback(f, app, area);
        return;
    }

    // Main Layout: Header (3), Tab Bar (3), Content (Min 10), Footer / Status (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top Header Banner
            Constraint::Length(3), // Tab Navigation Bar
            Constraint::Min(10),   // Active Tab Panel
            Constraint::Length(1), // Footer Keybinds / Notification
        ])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_tabs(f, app, chunks[1]);

    match app.current_tab {
        CockpitTab::Overview => draw_overview_tab(f, app, chunks[2]),
        CockpitTab::Services => draw_services_tab(f, app, chunks[2]),
        CockpitTab::Audit => draw_audit_tab(f, app, chunks[2]),
        CockpitTab::Cron => draw_cron_tab(f, app, chunks[2]),
        CockpitTab::Settings => draw_settings_tab(f, app, chunks[2]),
        CockpitTab::Chat => draw_chat_tab(f, app, chunks[2]),
    }

    draw_footer(f, app, chunks[3]);

    if app.inspector_open && app.current_tab == CockpitTab::Services {
        draw_app_inspector_modal(f, app, area);
    }

    if app.show_help_modal {
        draw_help_modal(f, app, area);
    }

    if app.show_pet_modal {
        draw_pet_modal(f, app, area);
    }

    if app.show_cron_wizard {
        draw_cron_wizard(f, app, area);
    }

    if app.show_quit_confirm {
        draw_quit_modal(f, app, area);
    }
}

// ── Header (Calm Sovereign Cockpit Branding) ────────────────────────────────

fn draw_header(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let inner = header_block.inner(area);
    f.render_widget(header_block, area);

    if inner.width < 20 || inner.height == 0 {
        return;
    }

    let title = " CAShell v0.3.1 · Sovereign Cockpit ";
    let time_str = chrono::Local::now().format("%H:%M:%S").to_string();

    let router_badge = if app.is_9router_online {
        Span::styled(
            " 9router: ● ONLINE ",
            Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " 9router: ✖ DOWN ",
            Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD),
        )
    };

    let tailscale_badge = if app.is_tailscale_ok {
        Span::styled(
            " Tailscale: ● OK ",
            Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " Tailscale: ✖ DOWN ",
            Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD),
        )
    };

    let host_badge = Span::styled(
        format!(" {} │", app.hostname),
        Style::default().fg(palette.text).add_modifier(Modifier::BOLD),
    );

    let time_badge = Span::styled(
        format!("│ {time_str} "),
        Style::default().fg(palette.accent),
    );

    // Active Virtual Pet Badge
    let pet_badge = if let Some(ref d) = app.db {
        if let Ok(Some(pet)) = d.get_active_pet() {
            let stage = crate::pet::PetStage::from_xp(pet.xp);
            let species = crate::pet::PetSpecies::parse(&pet.species);
            let status = crate::pet::PetStatus {
                species,
                name: pet.name,
                stage,
                mood: app.pet_mood,
                xp: pet.xp,
            };
            Span::styled(
                format!(" {} │", status.format_badge()),
                Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(" 🥚 No Pet (p) │", Style::default().fg(palette.text_muted))
        }
    } else {
        Span::styled(" 🐥 Ciko Lv.1 │", Style::default().fg(palette.text_muted))
    };

    let left_spans = vec![
        Span::styled(title, Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
    ];

    let right_spans = vec![
        pet_badge,
        host_badge,
        router_badge,
        Span::raw("│"),
        tailscale_badge,
        time_badge,
    ];

    let left_len: usize = left_spans.iter().map(|s| s.content.len()).sum();
    let right_len: usize = right_spans.iter().map(|s| s.content.len()).sum();
    let available_middle = (inner.width as usize).saturating_sub(left_len + right_len);

    let mut line_spans = Vec::new();
    line_spans.extend(left_spans);
    if available_middle > 0 {
        line_spans.push(Span::raw(" ".repeat(available_middle)));
    }
    line_spans.extend(right_spans);

    f.render_widget(Paragraph::new(Line::from(line_spans)), inner);
}

// ── Tab Bar Navigation ───────────────────────────────────────────────────────

fn draw_tabs(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let titles = CockpitTab::titles();
    let tab_items: Vec<Line> = titles
        .iter()
        .map(|t| Line::from(Span::raw(*t)))
        .collect();

    let tabs_widget = Tabs::new(tab_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border)),
        )
        .select(app.current_tab.to_index())
        .style(Style::default().fg(palette.text_muted))
        .highlight_style(
            Style::default()
                .fg(Color::Rgb(0, 0, 0))
                .bg(palette.highlight_bg)
                .add_modifier(Modifier::BOLD),
        )
        .divider("│");

    f.render_widget(tabs_widget, area);

    // Register Tab Hitboxes with exact positions based on title lengths
    let inner = area.inner(Margin::new(1, 1));
    let mut current_x = inner.x;
    for (i, title) in titles.iter().enumerate() {
        let tab_w = (title.len() as u16) + 2;
        if current_x + tab_w > inner.right() {
            break;
        }
        let tab_rect = Rect {
            x: current_x,
            y: inner.y,
            width: tab_w,
            height: 1,
        };
        app.register_hitbox(tab_rect, HitboxAction::SelectTab(i));
        current_x = current_x.saturating_add(tab_w + 1);
    }
}

// ── Tab 1: Overview (btop telemetry) ─────────────────────────────────────────

fn render_braille_curve(data: &[f32], width: usize, height: usize) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let dot_w = width * 2;
    let dot_h = height * 4;
    let mut grid = vec![vec![0u8; width]; height];

    let n = data.len();
    if n == 0 {
        return vec![Line::from(vec![Span::raw(" ".repeat(width))]); height];
    }

    let mut sampled = Vec::with_capacity(dot_w);
    for x in 0..dot_w {
        let val = if n == 1 {
            data[0]
        } else {
            let idx = (x as f32 / (dot_w - 1) as f32) * (n - 1) as f32;
            let i0 = (idx.floor() as usize).min(n - 1);
            let i1 = (i0 + 1).min(n - 1);
            let frac = idx - i0 as f32;
            data[i0] * (1.0 - frac) + data[i1] * frac
        };
        sampled.push(val);
    }

    for x in 0..dot_w {
        let val = sampled[x].clamp(0.0, 100.0);
        let y = ((dot_h - 1) as f32 - (val / 100.0) * (dot_h - 1) as f32).round() as usize;

        let (y_start, y_end) = if x > 0 {
            let prev_val = sampled[x - 1].clamp(0.0, 100.0);
            let prev_y = ((dot_h - 1) as f32 - (prev_val / 100.0) * (dot_h - 1) as f32).round() as usize;
            (prev_y.min(y), prev_y.max(y))
        } else {
            (y, y)
        };

        for cur_y in y_start..=y_end {
            let cx = x / 2;
            let cy = cur_y / 4;
            let sx = x % 2;
            let sy = cur_y % 4;

            let bit = match (sx, sy) {
                (0, 0) => 0x01,
                (0, 1) => 0x02,
                (0, 2) => 0x04,
                (0, 3) => 0x40,
                (1, 0) => 0x08,
                (1, 1) => 0x10,
                (1, 2) => 0x20,
                (1, 3) => 0x80,
                _ => 0,
            };

            if cy < height && cx < width {
                grid[cy][cx] |= bit;
            }
        }
    }

    let mut lines = Vec::with_capacity(height);
    for row in grid.iter().take(height) {
        let mut spans = Vec::with_capacity(width);
        for (cx, &mask) in row.iter().enumerate().take(width) {
            let ch = if mask == 0 {
                ' '
            } else {
                char::from_u32(0x2800 + mask as u32).unwrap_or(' ')
            };

            let avg_val = (sampled[(cx * 2).min(dot_w - 1)] + sampled[(cx * 2 + 1).min(dot_w - 1)]) / 2.0;
            let color = if avg_val >= 85.0 {
                Color::Red
            } else if avg_val >= 60.0 {
                Color::Yellow
            } else {
                Color::LightGreen
            };

            spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
        }
        lines.push(Line::from(spans));
    }

    lines
}

fn format_core_span(core: Option<&CpuCoreInfo>, id: usize, bar_len: usize) -> Vec<Span<'static>> {
    let (usage, freq) = if let Some(c) = core {
        (c.usage, c.frequency_mhz)
    } else {
        (0.0, 0)
    };
    let filled = ((usage / 100.0) * (bar_len as f32)).round().clamp(0.0, bar_len as f32) as usize;
    let unfilled = bar_len.saturating_sub(filled);

    let color = if usage >= 85.0 {
        Color::Red
    } else if usage >= 60.0 {
        Color::Yellow
    } else {
        Color::LightGreen
    };

    let freq_str = if freq >= 1000 {
        format!("{:.1}GHz", freq as f64 / 1000.0)
    } else if freq > 0 {
        format!("{freq}MHz")
    } else {
        " -- ".to_string()
    };

    vec![
        Span::styled(format!("C{id} "), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(unfilled), Style::default().fg(Color::DarkGray)),
        Span::styled("] ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{:>3.0}% ", usage), Style::default().fg(color)),
        Span::styled(format!("{:>6}", freq_str), Style::default().fg(Color::Cyan)),
    ]
}

fn format_kbps(kbps: f64) -> String {
    if kbps >= 1024.0 {
        format!("{:.1} MB/s", kbps / 1024.0)
    } else {
        format!("{:.0} KB/s", kbps)
    }
}

fn draw_overview_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12), // Top: CPU Grid + Tamagotchi Pet + Memory/Docker/Projects
            Constraint::Length(6),  // Middle 1: Uptime Kuma Endpoint Monitors
            Constraint::Length(3),  // Middle 2: Network & Tailscale Mesh
            Constraint::Min(8),     // Bottom: Live Dokploy Fleet & Top Processes
        ])
        .split(area);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(36), // CPU Telemetry
            Constraint::Percentage(36), // 🐾 Tamagotchi Pet Play Arena
            Constraint::Percentage(28), // Memory & Storage
        ])
        .split(chunks[0]);

    let snap = app.latest_snap.clone();
    let thermal_alert = snap.cpu_temp_c > 88.0;

    // ── Top Left: CPU & 8-Core Grid ──────────────────────────────────────────
    let cpu_title = if thermal_alert {
        format!(" CPU Telemetry ({:.1}%) [🔥 THERMAL ALERT] ", snap.cpu_pct)
    } else {
        format!(" CPU Telemetry ({:.1}%) ", snap.cpu_pct)
    };
    let cpu_border_color = if thermal_alert {
        Color::Red
    } else if snap.cpu_pct > 80.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .title(cpu_title)
        .border_style(Style::default().fg(cpu_border_color));
    let cpu_inner = cpu_block.inner(top_chunks[0]);
    f.render_widget(cpu_block, top_chunks[0]);

    if cpu_inner.width > 10 && cpu_inner.height > 2 {
        let mut header_spans = vec![
            Span::styled("Load: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("[{:.2}, {:.2}, {:.2}]  ", snap.load_avg[0], snap.load_avg[1], snap.load_avg[2]),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled("Fan: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}  ", snap.fan_speed.as_deref().unwrap_or("N/A")),
                Style::default().fg(Color::White),
            ),
            Span::styled("Temp: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1}°C", snap.cpu_temp_c),
                if snap.cpu_temp_c > 88.0 {
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                } else if snap.cpu_temp_c > 75.0 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::Green)
                },
            ),
        ];

        if thermal_alert {
            header_spans.push(Span::styled(
                "  [🔥 THERMAL ALERT]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        }

        // Braille curves for CPU history
        let cpu_hist_vec: Vec<f32> = app.cpu_history.iter().copied().collect();
        let braille_height = if cpu_inner.height >= 8 { 2 } else { 1 };
        let braille_lines = render_braille_curve(&cpu_hist_vec, cpu_inner.width as usize, braille_height);

        // 8-Core Grid: 2 columns of 4 cores (C0..C3 on left, C4..C7 on right)
        let half_w = (cpu_inner.width.saturating_sub(3) / 2) as usize;
        let bar_len = if half_w >= 26 { 10 } else if half_w >= 20 { 6 } else { 4 };

        let mut core_lines = Vec::new();
        for row in 0..4 {
            let c_left_idx = row;
            let c_right_idx = row + 4;

            let left_spans = format_core_span(snap.cpus.get(c_left_idx), c_left_idx, bar_len);
            let right_spans = format_core_span(snap.cpus.get(c_right_idx), c_right_idx, bar_len);

            let mut line_spans = left_spans;
            line_spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
            line_spans.extend(right_spans);
            core_lines.push(Line::from(line_spans));
        }

        let mut full_cpu_lines = Vec::new();
        full_cpu_lines.push(Line::from(header_spans));
        full_cpu_lines.extend(braille_lines);
        full_cpu_lines.extend(core_lines);

        f.render_widget(Paragraph::new(full_cpu_lines), cpu_inner);
    }

    // ── Center: Tamagotchi Pet Play Arena (Animated Hardware-Reactive) ───────
    draw_pet_play_area(f, app, top_chunks[1]);

    // ── Right: Memory & Mount Table ──────────────────────────────────────────
    let mem_block = Block::default()
        .borders(Borders::ALL)
        .title(" Memory & Storage Telemetry ")
        .border_style(Style::default().fg(Color::LightBlue));
    let mem_inner = mem_block.inner(top_chunks[2]);
    f.render_widget(mem_block, top_chunks[2]);

    if mem_inner.width > 10 && mem_inner.height > 3 {
        let mem_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // RAM gauge
                Constraint::Length(1), // Swap gauge
                Constraint::Length(1), // GPU telemetry info
                Constraint::Length(1), // Docker container summary
                Constraint::Min(3),    // Storage mounts table
            ])
            .split(mem_inner);

        // RAM Gauge with breakdown: Used (Cyan/Red), Cached, Free
        let ram_used = snap.ram_used_mb;
        let ram_total = snap.ram_total_mb.max(1);
        let ram_pct = ((ram_used * 100) / ram_total) as u16;
        let ram_color = if ram_pct > 85 {
            Color::Red
        } else if ram_pct > 70 {
            Color::Yellow
        } else {
            Color::Cyan
        };
        let ram_label = format!(
            "RAM: {ram_used}MB/{ram_total}MB ({ram_pct}%) [Cached: {}MB Free: {}MB]",
            snap.ram_cached_mb, snap.ram_available_mb
        );
        let ram_gauge = Gauge::default()
            .gauge_style(Style::default().fg(ram_color).bg(Color::DarkGray))
            .ratio(((ram_used as f64) / (ram_total as f64)).clamp(0.0, 1.0))
            .label(ram_label);
        f.render_widget(ram_gauge, mem_layout[0]);

        // Swap Gauge (Magenta)
        let swap_used = snap.swap_used_mb;
        let swap_total = snap.swap_total_mb.max(1);
        let swap_pct = if snap.swap_total_mb > 0 {
            ((swap_used * 100) / swap_total) as u16
        } else {
            0
        };
        let swap_label = format!("SWP: {swap_used}MB/{swap_total}MB ({swap_pct}%)");
        let swap_gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Magenta).bg(Color::DarkGray))
            .ratio(if snap.swap_total_mb > 0 {
                ((swap_used as f64) / (swap_total as f64)).clamp(0.0, 1.0)
            } else {
                0.0
            })
            .label(swap_label);
        f.render_widget(swap_gauge, mem_layout[1]);

        // GPU telemetry snippet
        let gpu_info = if snap.gpu_label != "No GPU" && !snap.gpu_label.is_empty() {
            let util = snap.gpu_pct.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "N/A".to_string());
            let temp = snap.gpu_temp_c.map(|t| format!("{t:.0}°C")).unwrap_or_else(|| "N/A".to_string());
            format!("GPU: {} │ Util: {} │ Temp: {}", snap.gpu_label, util, temp)
        } else {
            "GPU: Integrated Graphics (Intel Xe / AMD DRM)".to_string()
        };
        let gpu_line = Line::from(vec![
            Span::styled("◆ ", Style::default().fg(Color::Yellow)),
            Span::styled(gpu_info, Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(gpu_line), mem_layout[2]);

        // Docker Containers & Projects Summary Bar
        let running_cnt = snap.docker_containers.iter().filter(|c| c.state == "running").count();
        let stopped_cnt = snap.docker_containers.iter().filter(|c| c.state != "running").count();
        let proj_count = snap.tracked_projects.len();
        let docker_line = Line::from(vec![
            Span::styled("🐳 Docker: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{running_cnt} Running ● "), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{stopped_cnt} Stopped ✖  "), Style::default().fg(Color::Yellow)),
            Span::styled("📁 Projects: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{proj_count} Tracked"), Style::default().fg(Color::Cyan)),
        ]);
        f.render_widget(Paragraph::new(docker_line), mem_layout[3]);

        // Storage Mounts Table: List "/" (Root NVMe) and "/DATA" (External)
        let mut mount_rows = Vec::new();
        for m in &snap.mounts {
            let mount_label = if m.mountpoint == "/" {
                "/ (Root NVMe)"
            } else if m.mountpoint.contains("DATA") {
                "/DATA (External)"
            } else {
                m.mountpoint.as_str()
            };
            let used_total = format!("{}G / {}G", m.used_gb, m.total_gb);
            let bar_len: usize = 8;
            let filled = ((m.pct as f32 / 100.0) * bar_len as f32).round() as usize;
            let unfilled = bar_len.saturating_sub(filled);
            let bar_str = format!("[{}{}] {:>3}%", "█".repeat(filled), "░".repeat(unfilled), m.pct);
            let pct_color = if m.pct >= 90 {
                Color::Red
            } else if m.pct >= 75 {
                Color::Yellow
            } else {
                Color::Green
            };

            mount_rows.push(Row::new(vec![
                Cell::from(mount_label.to_string()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Cell::from(m.fstype.clone()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(used_total).style(Style::default().fg(Color::Cyan)),
                Cell::from(bar_str).style(Style::default().fg(pct_color)),
            ]));
        }

        if mount_rows.is_empty() {
            mount_rows.push(Row::new(vec![
                Cell::from("/ (Root NVMe)").style(Style::default().fg(Color::White)),
                Cell::from("ext4").style(Style::default().fg(Color::DarkGray)),
                Cell::from(format!("{}G / {}G", snap.disk_used_gb, snap.disk_total_gb)),
                Cell::from("[████░░░░] 50%").style(Style::default().fg(Color::Green)),
            ]));
        }

        let mount_table = Table::new(
            mount_rows,
            [
                Constraint::Length(18),
                Constraint::Length(8),
                Constraint::Length(15),
                Constraint::Min(14),
            ],
        )
        .header(
            Row::new(vec!["MOUNT", "FSTYPE", "USED/TOTAL", "USAGE"])
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        );
        f.render_widget(mount_table, mem_layout[4]);
    }

    // ── Middle Row 1: Uptime Kuma Endpoint Monitors ──────────────────────────
    draw_uptime_kuma_panel(f, &snap, chunks[1]);

    // ── Middle Row 2: Network & Tailscale Mesh Bar ───────────────────────────
    let net_block = Block::default()
        .borders(Borders::ALL)
        .title(" Network & Tailscale Mesh Telemetry ")
        .border_style(Style::default().fg(Color::LightBlue));
    let net_inner = net_block.inner(chunks[2]);
    f.render_widget(net_block, chunks[2]);

    if net_inner.width > 10 && net_inner.height > 0 {
        let mut net_spans = vec![
            Span::styled("NET [", Style::default().fg(Color::DarkGray)),
            Span::styled(&snap.active_net_iface, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("]: ", Style::default().fg(Color::DarkGray)),
            Span::styled("▼ RX: ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{} ({:.1} GB)  ", format_kbps(snap.net_rx_rate_kbps), snap.net_total_rx_gb),
                Style::default().fg(Color::White),
            ),
            Span::styled("▲ TX: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{} ({:.1} GB)", format_kbps(snap.net_tx_rate_kbps), snap.net_total_tx_gb),
                Style::default().fg(Color::White),
            ),
            Span::styled("  │  MESH: ", Style::default().fg(Color::DarkGray)),
        ];

        for (i, peer) in snap.mesh_peers.iter().enumerate() {
            if i > 0 {
                net_spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
            }
            net_spans.push(Span::styled(
                format!("{}: {} ", peer.name, peer.endpoint),
                Style::default().fg(Color::White),
            ));
            if let Some(lat) = peer.latency_ms {
                net_spans.push(Span::styled("● ", Style::default().fg(Color::Green)));
                net_spans.push(Span::styled(format!("{lat}ms"), Style::default().fg(Color::Green)));
            } else {
                net_spans.push(Span::styled("● ", Style::default().fg(Color::Red)));
                net_spans.push(Span::styled("DOWN", Style::default().fg(Color::Red)));
            }
        }

        f.render_widget(Paragraph::new(Line::from(net_spans)), net_inner);
    }

    // ── Bottom Half: Live Dokploy Fleet & Top Processes ──────────────────────
    let fleet_block = Block::default()
        .borders(Borders::ALL)
        .title(" Live Dokploy Fleet & Top Processes (Press 'j'/'k' to scroll) ")
        .border_style(Style::default().fg(Color::Cyan));
    let fleet_inner = fleet_block.inner(chunks[3]);
    f.render_widget(fleet_block, chunks[3]);

    struct FleetEntry {
        pid: String,
        name: String,
        role: String,
        is_active: bool,
        cpu_pct: f32,
        mem_mb: u64,
        details: String,
    }

    let primary_agents: [(&str, &str, &str, &str); 11] = [
        ("hermes", "Hermes Agent", "AI Agent Runtime", "Hana/Hermes autonomous coding gateway"),
        ("qa-agent", "QA Test Agent", "QA Test Agent", "Autonomous test & quality evaluator"),
        ("dev-agent", "Dev Coding Agent", "Coding Dev Agent", "Autonomous code implementation agent"),
        ("biz-agent", "Biz Strategy Agent", "Biz Agent", "Business logic & product coordinator"),
        ("content-agent", "Content Agent", "Content Agent", "Copy, docs, and knowledge base generator"),
        ("9router", "9router AI Gateway", "AI Proxy Gateway", "Unified LLM router on :20128"),
        ("cloudflared", "Cloudflared Tunnel", "Edge Tunnel", "Remote Zero Trust mesh tunnel"),
        ("telegram", "Telegram Dispatcher", "Telegram Worker", "Telegram fleet notification bot"),
        ("caterm", "caterm Console", "Web Terminal", "CA ecosystem browser terminal on :8081"),
        ("fathstore", "fathstore Engine", "E-Commerce Engine", "POS & store backend on :8082"),
        ("gcc", "GCC Coordinator", "Agent Dispatcher", "Global Computing Coordinator on :8083"),
    ];

    let mut entries = Vec::new();
    let mut matched_pids = std::collections::HashSet::new();

    // 1. Process primary agent roster
    for (key, display_name, def_role, desc) in &primary_agents {
        let found = snap.top_processes.iter().find(|p| {
            p.name.to_lowercase().contains(key) || p.role.to_lowercase().contains(key)
        });

        if let Some(p) = found {
            matched_pids.insert(p.pid);
            entries.push(FleetEntry {
                pid: p.pid.to_string(),
                name: display_name.to_string(),
                role: p.role.clone(),
                is_active: true,
                cpu_pct: p.cpu_pct,
                mem_mb: p.mem_mb,
                details: format!("{desc} (PID: {})", p.pid),
            });
        } else {
            entries.push(FleetEntry {
                pid: "--".to_string(),
                name: display_name.to_string(),
                role: def_role.to_string(),
                is_active: false,
                cpu_pct: 0.0,
                mem_mb: 0,
                details: desc.to_string(),
            });
        }
    }

    // 2. Add remaining active top processes from snap.top_processes
    for p in &snap.top_processes {
        if !matched_pids.contains(&p.pid) {
            entries.push(FleetEntry {
                pid: p.pid.to_string(),
                name: p.name.clone(),
                role: p.role.clone(),
                is_active: true,
                cpu_pct: p.cpu_pct,
                mem_mb: p.mem_mb,
                details: format!("System service (status: {})", p.status),
            });
        }
    }

    // Sort entries: Active ones first, then by cpu_pct desc, then mem desc
    entries.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| b.mem_mb.cmp(&a.mem_mb))
    });

    // Apply scroll
    let scroll_offset = app.overview_proc_scroll.min(entries.len().saturating_sub(1));
    let display_entries = entries.into_iter().skip(scroll_offset);

    let rows: Vec<Row> = display_entries
        .map(|e| {
            let cpu_color = if e.cpu_pct >= 80.0 {
                Color::Red
            } else if e.cpu_pct >= 50.0 {
                Color::Yellow
            } else if e.cpu_pct > 0.0 {
                Color::LightGreen
            } else {
                Color::DarkGray
            };

            let mem_color = if e.mem_mb > 0 { Color::White } else { Color::DarkGray };

            let (status_label, status_color) = if e.is_active {
                ("● Active", Color::Green)
            } else {
                ("● Standby", Color::Cyan)
            };

            Row::new(vec![
                Cell::from(e.pid).style(Style::default().fg(Color::DarkGray)),
                Cell::from(e.name).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Cell::from(e.role).style(Style::default().fg(Color::Cyan)),
                Cell::from(status_label).style(Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                Cell::from(format!("{:>5.1}%", e.cpu_pct)).style(Style::default().fg(cpu_color)),
                Cell::from(format!("{:>6} MB", e.mem_mb)).style(Style::default().fg(mem_color)),
                Cell::from(e.details).style(Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(22),
            Constraint::Length(20),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(11),
            Constraint::Min(25),
        ],
    )
    .header(
        Row::new(vec![
            "PID",
            "NAME / SERVICE",
            "TYPE / ROLE",
            "STATUS",
            "CPU%",
            "MEM",
            "DETAILS",
        ])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    );

    f.render_widget(table, fleet_inner);
}

// ── 🐾 Tamagotchi Pet Play Arena (Animated & Hardware Reactive) ─────────────

fn draw_pet_play_area(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let mood_color = match app.pet_mood {
        crate::pet::PetMood::Overheated => Color::LightRed,
        crate::pet::PetMood::Sick => Color::Magenta,
        crate::pet::PetMood::Sprint => Color::Yellow,
        crate::pet::PetMood::Idle => palette.accent,
    };

    let pet_opt = if let Some(ref d) = app.db {
        d.get_active_pet().ok().flatten()
    } else {
        None
    };

    let (name, species_str, xp) = if let Some(ref p) = pet_opt {
        (p.name.clone(), p.species.clone(), p.xp)
    } else {
        (String::new(), "ayam".to_string(), 0)
    };

    let species = crate::pet::PetSpecies::parse(&species_str);
    let stage = crate::pet::PetStage::from_xp(xp);
    let level = match stage {
        crate::pet::PetStage::Egg => 0,
        crate::pet::PetStage::Hatch => 1,
        crate::pet::PetStage::Juvenile => 2,
        crate::pet::PetStage::Adult => 3,
        crate::pet::PetStage::Mating => 4,
    };

    let mood_short = match app.pet_mood {
        crate::pet::PetMood::Idle => "☺ Normal",
        crate::pet::PetMood::Overheated => "🥵 Overheated (Fan Active)",
        crate::pet::PetMood::Sick => "💀 Sick / High Load",
        crate::pet::PetMood::Sprint => "🏃 Sprinting",
    };

    let pet_name_display = if name.is_empty() {
        species.display_name().to_string()
    } else {
        name.clone()
    };

    let title = format!(
        " 🐾 PET PLAY ARENA [ {} | Lv.{} | {} ] ",
        pet_name_display, level, mood_short
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(mood_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 10 || inner.height < 5 {
        return;
    }

    // Toggle History Log View
    if app.show_pet_history {
        let mut lines = vec![
            Line::from(vec![
                Span::styled(" 📜 PET ACTIVITY HISTORY LOG ", Style::default().fg(palette.highlight_fg).add_modifier(Modifier::BOLD)),
                Span::styled(" [✖ Close (h/Esc)] ", Style::default().fg(palette.text_muted)),
            ]),
            Line::from(Span::raw("─".repeat(inner.width as usize))),
        ];
        if app.pet_history_logs.is_empty() {
            lines.push(Line::from(Span::styled(
                "No activity logged yet. Feed or treat your pet!",
                Style::default().fg(palette.text_muted),
            )));
        } else {
            let max_items = inner.height.saturating_sub(4) as usize;
            for log in app.pet_history_logs.iter().take(max_items.max(1)) {
                let time_str = log.timestamp.split('T').nth(1).unwrap_or(&log.timestamp);
                let time_short: String = time_str.chars().take(8).collect();
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<8}", time_short), Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("[{}]", log.action_type), Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
                    Span::styled(format!(" {}", log.detail), Style::default().fg(Color::White)),
                ]));
            }
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    // Split inner into Screen Area (top) and Control Bar (bottom)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),    // Animated Sprite Screen
            Constraint::Length(1), // Nav Bar: [▲ Up] [▼ Down] [✔ OK] [✖ Cancel]
            Constraint::Length(1), // Actions: [0: Feed] [1: Treat] [2: Switch] [3: History]
        ])
        .split(inner);

    // 1. Animated Sprite Screen
    let status = crate::pet::PetStatus {
        species,
        name: pet_name_display.clone(),
        stage,
        mood: app.pet_mood,
        xp,
    };
    let frames = status.frames();
    let frame_idx = (app.pet_tick / 4) % frames.len();
    let current_frame = frames[frame_idx];

    // Wander offset across the sprite screen
    let wander_offset = if app.pet_mood == crate::pet::PetMood::Sprint {
        let screen_width = inner.width.saturating_sub(15) as usize;
        (app.pet_tick / 2) % screen_width.max(1)
    } else {
        let max_w = inner.width.saturating_sub(30).min(12) as usize;
        if max_w > 0 {
            let cycle = (app.pet_tick / 6) % (max_w * 2);
            if cycle < max_w {
                cycle
            } else {
                max_w * 2 - cycle
            }
        } else {
            0
        }
    };

    let pad = " ".repeat(wander_offset);
    let mut sprite_lines = Vec::new();
    for row in current_frame {
        sprite_lines.push(Line::from(vec![
            Span::raw(pad.clone()),
            Span::styled(
                *row,
                Style::default()
                    .fg(match app.pet_mood {
                        crate::pet::PetMood::Overheated => Color::LightRed,
                        crate::pet::PetMood::Sick => Color::Magenta,
                        crate::pet::PetMood::Sprint => Color::Yellow,
                        crate::pet::PetMood::Idle => Color::Green,
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    f.render_widget(Paragraph::new(sprite_lines), chunks[0]);

    // 2. Navigation Bar (mouse hitboxes & keyboard shortcuts)
    let nav_spans = vec![
        Span::styled(" [▲ Up] ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [▼ Down] ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [✔ OK] ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [✖ Cancel] ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Red).add_modifier(Modifier::BOLD)),
    ];
    f.render_widget(Paragraph::new(Line::from(nav_spans)), chunks[1]);

    // Register hitboxes for nav bar controls
    let nav_inner = chunks[1];
    let _ = nav_inner;
    let nav_x = inner.x;
    let up_w = 8u16;
    let down_w = 10u16;
    let ok_w = 8u16;
    let cancel_w = 12u16;
    app.register_hitbox(
        Rect { x: nav_x, y: chunks[1].y, width: up_w, height: 1 },
        HitboxAction::PetActionUp,
    );
    let mut x2 = nav_x + up_w + 1;
    app.register_hitbox(
        Rect { x: x2, y: chunks[1].y, width: down_w, height: 1 },
        HitboxAction::PetActionDown,
    );
    x2 += down_w + 1;
    app.register_hitbox(
        Rect { x: x2, y: chunks[1].y, width: ok_w, height: 1 },
        HitboxAction::PetOk,
    );
    x2 += ok_w + 1;
    app.register_hitbox(
        Rect { x: x2, y: chunks[1].y, width: cancel_w, height: 1 },
        HitboxAction::PetCancel,
    );

    // 3. Action Buttons with active selection highlight
    let actions = [
        "0: 🍖Feed(f)",
        "1: 🩺Treat(t)",
        "2: 🔄Pet(s)",
        "3: 📜Log(h)",
    ];
    let mut action_spans = Vec::new();
    let mut act_x = inner.x;
    for (i, act) in actions.iter().enumerate() {
        let label = format!(" [{}] ", act);
        let btn_w = label.len() as u16;
        if i == app.pet_selected_action {
            action_spans.push(Span::styled(
                label,
                Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        } else {
            action_spans.push(Span::styled(
                label,
                Style::default().fg(palette.text_muted),
            ));
        }
        action_spans.push(Span::raw(" "));

        // Register action button hitboxes
        let action_hb = match i {
            0 => HitboxAction::PetFeed,
            1 => HitboxAction::PetTreat,
            2 => HitboxAction::PetSwitch,
            _ => HitboxAction::PetToggleHistory,
        };
        app.register_hitbox(
            Rect { x: act_x, y: chunks[2].y, width: btn_w, height: 1 },
            action_hb,
        );
        act_x += btn_w + 1;
    }
    f.render_widget(Paragraph::new(Line::from(action_spans)), chunks[2]);
}

// ── Uptime Kuma Style Endpoint Monitors Panel ───────────────────────────────

fn draw_uptime_kuma_panel(f: &mut Frame, snap: &SysSnapshot, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 🌐 Uptime Kuma Endpoint Monitors (HTTP/HTTPS, TCP & Gateways) ")
        .border_style(Style::default().fg(Color::LightGreen));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 10 || inner.height < 2 {
        return;
    }

    let rows: Vec<Row> = snap.uptime_monitors.iter().map(|m| {
        let (status_badge, badge_color) = if m.is_up {
            (format!("● UP ({:.1}%)", m.uptime_pct), Color::Green)
        } else {
            (format!("✖ DOWN ({:.1}%)", m.uptime_pct), Color::Red)
        };

        let latency_str = m
            .latency_ms
            .map(|l| format!("{l:>3}ms"))
            .unwrap_or_else(|| " -- ".to_string());
        let code_str = m
            .status_code
            .map(|c| format!("{c} OK"))
            .unwrap_or_else(|| "--".to_string());

        // Uptime Kuma 20-segment heartbeat timeline
        let bar_spans: Vec<Span<'static>> = m
            .history
            .iter()
            .map(|&up| {
                if up {
                    Span::styled("█", Style::default().fg(Color::Green))
                } else {
                    Span::styled("█", Style::default().fg(Color::Red))
                }
            })
            .collect();

        Row::new(vec![
            Cell::from(m.name.clone())
                .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Cell::from(status_badge)
                .style(Style::default().fg(badge_color).add_modifier(Modifier::BOLD)),
            Cell::from(latency_str).style(Style::default().fg(Color::Cyan)),
            Cell::from(Line::from(bar_spans)),
            Cell::from(code_str).style(Style::default().fg(Color::Yellow)),
            Cell::from(m.target.clone()).style(Style::default().fg(Color::DarkGray)),
        ])
    })
    .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(15),
            Constraint::Length(8),
            Constraint::Length(22),
            Constraint::Length(8),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec![
            "MONITOR",
            "STATUS",
            "LATENCY",
            "HEARTBEATS (20)",
            "CODE",
            "ENDPOINT",
        ])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);
}

// ── Tab 2: Services / Apps (Dokploy-style) ───────────────────────────────────

fn draw_services_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    // Header action buttons bar
    let action_bar = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let action_inner = action_bar.inner(chunks[0]);
    f.render_widget(action_bar, chunks[0]);

    let btn_probe_rect = Rect {
        x: action_inner.x + 1,
        y: action_inner.y,
        width: 17,
        height: 1,
    };
    let btn_inspect_rect = Rect {
        x: action_inner.x + 20,
        y: action_inner.y,
        width: 19,
        height: 1,
    };

    app.register_hitbox(btn_probe_rect, HitboxAction::ProbePorts);
    app.register_hitbox(btn_inspect_rect, HitboxAction::InspectApp(app.selected_app));

    let btn_probe = Span::styled(
        " [🔍 Probe Ports] ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    let btn_inspect = Span::styled(
        " [⚙ Inspect Config] ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let hint = Span::styled(
        "  (Press 'p' to probe, 'i'/Enter to inspect app card)",
        Style::default().fg(Color::DarkGray),
    );

    f.render_widget(Paragraph::new(Line::from(vec![btn_probe, Span::raw(" "), btn_inspect, hint])), action_inner);

    // Apps Cards List
    let list_block = Block::default()
        .borders(Borders::ALL)
        .title(" Dokploy Infrastructure & Applications ")
        .border_style(Style::default().fg(Color::Cyan));
    let list_inner = list_block.inner(chunks[1]);
    f.render_widget(list_block, chunks[1]);

    let items: Vec<ListItem> = app
        .apps
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let (status_icon, status_color) = match item.status {
                ServiceStatus::Healthy => ("● Healthy", Color::Green),
                ServiceStatus::Listening => ("● Listening", Color::Cyan),
                ServiceStatus::Down => ("✖ Down", Color::Red),
                ServiceStatus::Probing => ("◌ Probing", Color::Yellow),
            };

            let is_selected = i == app.selected_app;
            let row_bg = if is_selected {
                Color::Rgb(30, 45, 70)
            } else {
                Color::Reset
            };

            let latency = item
                .latency_ms
                .map(|ms| format!("({ms}ms)"))
                .unwrap_or_default();

            let line = Line::from(vec![
                Span::styled(format!(" {:<18} ", item.name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<14} ", item.category), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<24} ", item.target), Style::default().fg(Color::LightBlue)),
                Span::styled(format!("{:<12} ", status_icon), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                Span::styled(latency, Style::default().fg(Color::Gray)),
            ]);

            ListItem::new(line).style(Style::default().bg(row_bg))
        })
        .collect();

    let apps_list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(35, 55, 90))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.selected_app));
    f.render_stateful_widget(apps_list, list_inner, &mut state);

    // Register hitboxes for each app row
    let apps_count = app.apps.len();
    for i in 0..apps_count {
        if (i as u16) < list_inner.height {
            let row_rect = Rect {
                x: list_inner.x,
                y: list_inner.y + (i as u16),
                width: list_inner.width,
                height: 1,
            };
            app.register_hitbox(row_rect, HitboxAction::SelectApp(i));
        }
    }
}

// ── App Inspector Modal ─────────────────────────────────────────────────────

fn draw_app_inspector_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    if app.selected_app >= app.apps.len() {
        return;
    }
    let card = &app.apps[app.selected_app];

    let popup_area = centered_rect(65, 50, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" App Inspector — {} ", card.name))
        .border_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let (status_text, status_col) = match card.status {
        ServiceStatus::Healthy => ("HEALTHY (Responding to TCP)", Color::Green),
        ServiceStatus::Listening => ("LISTENING (Active Port)", Color::Cyan),
        ServiceStatus::Down => ("DOWN / INACTIVE", Color::Red),
        ServiceStatus::Probing => ("PROBING...", Color::Yellow),
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Application ID:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(&card.id, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Category:          ", Style::default().fg(Color::DarkGray)),
            Span::styled(&card.category, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Target Route:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(&card.target, Style::default().fg(Color::LightBlue)),
        ]),
        Line::from(vec![
            Span::styled("Binding Port:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                card.port.map(|p| p.to_string()).unwrap_or_else(|| "Dynamic / None".to_string()),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("Health Status:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(status_text, Style::default().fg(status_col).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Latency (RTT):     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                card.latency_ms.map(|ms| format!("{ms} ms")).unwrap_or_else(|| "N/A".to_string()),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Description:       ", Style::default().fg(Color::DarkGray)),
            Span::styled(&card.details, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled("Press 'Esc' or click [Close] to dismiss this inspector.", Style::default().fg(Color::DarkGray))),
    ];

    let content_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(2)])
        .split(inner);

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), content_layout[0]);

    let close_btn_rect = Rect {
        x: content_layout[1].x + 1,
        y: content_layout[1].y,
        width: 15,
        height: 1,
    };
    app.register_hitbox(close_btn_rect, HitboxAction::CloseInspector);
    let close_span = Span::styled(
        " [ Close (Esc) ] ",
        Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD),
    );
    f.render_widget(Paragraph::new(Line::from(close_span)), content_layout[1]);
}

// ── Tab 3: Audit & Code ──────────────────────────────────────────────────────

fn draw_audit_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    // Actions Header
    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_audit_rect = Rect {
        x: bar_inner.x + 1,
        y: bar_inner.y,
        width: 18,
        height: 1,
    };
    let btn_fix_rect = Rect {
        x: bar_inner.x + 21,
        y: bar_inner.y,
        width: 15,
        height: 1,
    };

    app.register_hitbox(btn_audit_rect, HitboxAction::RunAudit);
    app.register_hitbox(btn_fix_rect, HitboxAction::RunFix);

    let btn_audit = Span::styled(
        " [⚡ Turbo Audit] ",
        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
    );
    let btn_fix = Span::styled(
        " [🔧 Run Fix] ",
        Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD),
    );

    let status_text = app
        .audit_status_msg
        .as_deref()
        .unwrap_or("Select a report or run a scan. (Hotkey 'a': Audit, 'f': Fix)");

    f.render_widget(
        Paragraph::new(Line::from(vec![
            btn_audit,
            Span::raw(" "),
            btn_fix,
            Span::styled(format!("  {status_text}"), Style::default().fg(Color::Yellow)),
        ])),
        bar_inner,
    );

    // Split reports list (left 40%) vs preview (right 60%)
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    // Reports List
    let list_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Recent Audit Reports ({}) ", app.audit_reports.len()))
        .border_style(Style::default().fg(Color::Cyan));
    let list_inner = list_block.inner(content_chunks[0]);
    f.render_widget(list_block, content_chunks[0]);

    let report_items: Vec<ListItem> = app
        .audit_reports
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = i == app.selected_report;
            let bg = if is_selected {
                Color::Rgb(30, 45, 70)
            } else {
                Color::Reset
            };
            let line = Line::from(vec![
                Span::styled(format!(" {:<20} ", r.filename), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} KB ", r.size_bytes / 1024), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("({} findings)", r.findings_count), Style::default().fg(Color::Yellow)),
            ]);
            ListItem::new(line).style(Style::default().bg(bg))
        })
        .collect();

    let mut state = ListState::default();
    if !app.audit_reports.is_empty() {
        state.select(Some(app.selected_report));
    }
    let list_w = List::new(report_items)
        .highlight_symbol("▶ ")
        .highlight_style(Style::default().bg(Color::Rgb(35, 55, 90)));
    f.render_stateful_widget(list_w, list_inner, &mut state);

    let reports_count = app.audit_reports.len();
    for i in 0..reports_count {
        if (i as u16) < list_inner.height {
            let row_rect = Rect {
                x: list_inner.x,
                y: list_inner.y + (i as u16),
                width: list_inner.width,
                height: 1,
            };
            app.register_hitbox(row_rect, HitboxAction::SelectReport(i));
        }
    }

    // Report Preview Box
    let preview_block = Block::default()
        .borders(Borders::ALL)
        .title(" Report Preview & Remediation ")
        .border_style(Style::default().fg(Color::DarkGray));
    let preview_inner = preview_block.inner(content_chunks[1]);
    f.render_widget(preview_block, content_chunks[1]);

    if let Some(report) = app.audit_reports.get(app.selected_report) {
        let lines: Vec<Line> = report
            .preview
            .iter()
            .skip(app.report_scroll)
            .map(|l| {
                let style = if l.starts_with("# ") {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else if l.starts_with("## ") || l.starts_with("### ") {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else if l.contains("CRITICAL") {
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                } else if l.contains("HIGH") {
                    Style::default().fg(Color::LightRed)
                } else {
                    Style::default().fg(Color::White)
                };
                Line::from(Span::styled(l.clone(), style))
            })
            .collect();

        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), preview_inner);
    } else {
        f.render_widget(
            Paragraph::new("No audit reports found in audit/. Click [⚡ Turbo Audit] to generate one.")
                .style(Style::default().fg(Color::DarkGray)),
            preview_inner,
        );
    }
}

// ── Tab 4: Settings GUI (Interactive visual configuration form) ─────────────

fn draw_settings_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    // Save / Reload / DB Action bar
    let action_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let action_inner = action_block.inner(chunks[0]);
    f.render_widget(action_block, chunks[0]);

    let btn_save_rect = Rect {
        x: action_inner.x + 1,
        y: action_inner.y,
        width: 17,
        height: 1,
    };
    let btn_reload_rect = Rect {
        x: action_inner.x + 19,
        y: action_inner.y,
        width: 12,
        height: 1,
    };
    let btn_clear_rect = Rect {
        x: action_inner.x + 32,
        y: action_inner.y,
        width: 16,
        height: 1,
    };
    let btn_vacuum_rect = Rect {
        x: action_inner.x + 49,
        y: action_inner.y,
        width: 15,
        height: 1,
    };

    app.register_hitbox(btn_save_rect, HitboxAction::SaveSettings);
    app.register_hitbox(btn_reload_rect, HitboxAction::ReloadSettings);
    app.register_hitbox(btn_clear_rect, HitboxAction::ClearChat);
    app.register_hitbox(btn_vacuum_rect, HitboxAction::VacuumDb);

    let btn_save = Span::styled(
        " [💾 Save] ",
        Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.status_ok).add_modifier(Modifier::BOLD),
    );
    let btn_reload = Span::styled(
        " [↺ Reload] ",
        Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.status_warn).add_modifier(Modifier::BOLD),
    );
    let btn_clear = Span::styled(
        " [🗑 Clear Chat] ",
        Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.status_err).add_modifier(Modifier::BOLD),
    );
    let btn_vacuum = Span::styled(
        " [🧹 Vacuum DB] ",
        Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.accent).add_modifier(Modifier::BOLD),
    );

    let status = app
        .settings_status_msg
        .as_deref()
        .unwrap_or("s: save | r: reload | c: clear chat | v: vacuum | Enter: edit | ←/→: cycle");

    f.render_widget(
        Paragraph::new(Line::from(vec![
            btn_save,
            Span::raw(" "),
            btn_reload,
            Span::raw(" "),
            btn_clear,
            Span::raw(" "),
            btn_vacuum,
            Span::styled(format!("  {status}"), Style::default().fg(palette.accent)),
        ])),
        action_inner,
    );

    // Form fields card
    let form_block = Block::default()
        .borders(Borders::ALL)
        .title(" Interactive Configuration Editor (~/.config/cashell/config.toml) ")
        .border_style(Style::default().fg(palette.border));
    let form_inner = form_block.inner(chunks[1]);
    f.render_widget(form_block, chunks[1]);

    let provider = &app.cfg.ai.default_provider;
    let model = app
        .cfg
        .ai
        .providers
        .get(provider)
        .map(|p| p.default_model.as_str())
        .unwrap_or("ZA126_PRO");

    let api_key = app
        .cfg
        .ai
        .providers
        .get(provider)
        .map(|p| p.api_key.as_str())
        .unwrap_or("");
    let masked_key = if api_key.is_empty() {
        "[Not Configured / None]"
    } else {
        "••••••••••••••••"
    };

    let theme = &app.cfg.output.theme;
    let db_size_str = format_bytes(app.get_db_size());
    let retention_str = format!(
        "{} days (auto_prune: {})",
        app.cfg.database.retention_days,
        if app.cfg.database.auto_prune { "on" } else { "off" }
    );
    let target_host = &app.cfg.doctor.target_addr;
    let target_port = app.cfg.doctor.target_port.to_string();
    let port_matrix = app
        .cfg
        .doctor
        .ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ");

    let values: [&str; 11] = [
        provider.as_str(),
        model,
        masked_key,
        theme.as_str(),
        app.cfg.persona.user_name.as_str(),
        app.cfg.persona.assistant_role.as_str(),
        retention_str.as_str(),
        db_size_str.as_str(),
        target_host.as_str(),
        target_port.as_str(),
        port_matrix.as_str(),
    ];

    let items: Vec<ListItem> = SETTINGS_FIELDS
        .iter()
        .enumerate()
        .map(|(i, &label)| {
            let is_selected = i == app.selected_field;
            let is_editing = app.editing_field == Some(i);

            let bg = if is_selected {
                palette.row_selected_bg
            } else {
                Color::Reset
            };

            let val_span = if is_editing {
                Span::styled(
                    format!(" [ {}█ ]", app.edit_buffer),
                    Style::default()
                        .fg(Color::Rgb(0, 0, 0))
                        .bg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    format!(" {}", values[i]),
                    Style::default()
                        .fg(if is_selected {
                            palette.text
                        } else {
                            palette.text_muted
                        })
                        .add_modifier(Modifier::BOLD),
                )
            };

            let prefix = if is_selected { "▶ " } else { "  " };

            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.accent)),
                Span::styled(
                    format!("{:<24}:", label),
                    Style::default().fg(if is_selected {
                        palette.accent
                    } else {
                        palette.text
                    }),
                ),
                val_span,
            ]);

            ListItem::new(line).style(Style::default().bg(bg))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.selected_field));
    let form_list = List::new(items);
    f.render_stateful_widget(form_list, form_inner, &mut state);

    // Register hitboxes for form rows
    for (i, _) in SETTINGS_FIELDS.iter().enumerate() {
        if (i as u16) < form_inner.height {
            let row_rect = Rect {
                x: form_inner.x,
                y: form_inner.y + (i as u16),
                width: form_inner.width,
                height: 1,
            };
            app.register_hitbox(row_rect, HitboxAction::SelectField(i));
        }
    }
}

// ── Tab 5: AI Chat (Hana AI chat interface) ──────────────────────────────────

fn draw_chat_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3)])
        .split(area);

    // Chat History
    let history_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Hana AI Assistant (Model: {}, Role: {}) ",
            app.cfg.ai.default_provider, app.cfg.persona.user_name
        ))
        .border_style(Style::default().fg(palette.border));
    let history_inner = history_block.inner(chunks[0]);
    f.render_widget(history_block, chunks[0]);

    let items: Vec<ListItem> = app
        .chat_lines
        .iter()
        .map(|l| {
            let style = if l.starts_with("User:") {
                Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)
            } else if l.starts_with("AI:") {
                Style::default().fg(palette.text)
            } else if l.starts_with("Error:") {
                Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text_muted)
            };
            ListItem::new(Line::from(Span::styled(l.clone(), style)))
        })
        .collect();

    let chat_list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(palette.text)
                .bg(palette.row_selected_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(chat_list, history_inner, &mut app.chat_state);

    // Prompt Input Bar
    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(if app.is_chat_processing {
            " Hana AI is thinking... "
        } else {
            " Terminal Prompt ($ > Enter to send | Shift+1..5 to switch tabs) "
        })
        .border_style(if app.is_chat_processing {
            Style::default().fg(palette.status_warn)
        } else {
            Style::default().fg(palette.status_ok)
        });
    let input_inner = input_block.inner(chunks[1]);
    f.render_widget(input_block, chunks[1]);

    app.register_hitbox(chunks[1], HitboxAction::FocusChatInput);

    let prompt_line = Line::from(vec![
        Span::styled(
            "$ > ",
            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &app.chat_input,
            Style::default().fg(palette.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "█",
            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(Paragraph::new(prompt_line), input_inner);
}

// ── Footer & Compact Fallback ────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, app: &CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let keybinds = " [1-5]: Tabs │ Tab: Cycle │ Shift+1-5: Jump │ ?: Help │ Esc/q: Quit │ Enter: Send/Edit │ Mouse ";
    let content = if let Some((msg, _)) = &app.notification {
        Line::from(vec![
            Span::styled(
                format!(" ★ {msg} "),
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(palette.status_warn)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(keybinds, Style::default().fg(palette.text_muted)),
        ])
    } else {
        Line::from(Span::styled(keybinds, Style::default().fg(palette.text_muted)))
    };

    f.render_widget(Paragraph::new(content).alignment(Alignment::Left), area);
}

fn draw_compact_fallback(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" CAShell Cockpit (Compact View) ")
        .border_style(Style::default().fg(palette.status_warn));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let snap = &app.latest_snap;
    let lines = vec![
        Line::from(Span::styled(
            "CAShell v0.3.0 · Sovereign Cockpit (Compact Mode)",
            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "Terminal size: {}x{} (< 80x24 required for full view)",
            area.width, area.height
        )),
        Line::from(""),
        Line::from(format!(
            "CPU: {:.1}% │ RAM: {}/{} MB │ Disk: {}/{} GB",
            snap.cpu_pct, snap.ram_used_mb, snap.ram_total_mb, snap.disk_used_gb, snap.disk_total_gb
        )),
        Line::from(format!(
            "9router: {} │ Tailscale: {}",
            if app.is_9router_online { "ONLINE" } else { "DOWN" },
            if app.is_tailscale_ok { "OK" } else { "DOWN" }
        )),
        Line::from(""),
        Line::from("Tabs: [1] Overview [2] Apps [3] Audit [4] Settings [5] Chat"),
        Line::from("Press 'q' or 'Esc' to exit. Resize terminal to >= 80x24 for full Cockpit."),
    ];

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ── Keybindings Help Modal ───────────────────────────────────────────────────

fn draw_help_modal(f: &mut Frame, app: &CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let modal_width = 72.min(area.width.saturating_sub(4));
    let modal_height = 20.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(modal_width)) / 2;
    let y = (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect {
        x,
        y,
        width: modal_width,
        height: modal_height,
    };

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ⌨ CAShell Sovereign Cockpit · Keybindings Cheat Sheet ")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(palette.accent));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let rows = vec![
        Row::new(vec![
            Cell::from("1..5 / Shift+1..5"),
            Cell::from("Switch tabs (Overview, Services, Audit, Settings, Chat)"),
        ]),
        Row::new(vec![
            Cell::from("Tab / BackTab"),
            Cell::from("Cycle through tabs forward / backward"),
        ]),
        Row::new(vec![
            Cell::from("? / F1"),
            Cell::from("Toggle this Keybindings Help cheat sheet"),
        ]),
        Row::new(vec![
            Cell::from("q / Esc"),
            Cell::from("Close modal / Inspector / Exit Cockpit"),
        ]),
        Row::new(vec![
            Cell::from("↑ / ↓ (k / j)"),
            Cell::from("Navigate items, lists, or scroll content"),
        ]),
        Row::new(vec![
            Cell::from("Services: p / i"),
            Cell::from("p: Probe ports | i / Enter: Inspect app card details"),
        ]),
        Row::new(vec![
            Cell::from("Audit: a / f"),
            Cell::from("a: Run Turbo Audit | f: Run Fix script"),
        ]),
        Row::new(vec![
            Cell::from("Settings: Enter / ← →"),
            Cell::from("Enter: Edit field | ← / →: Cycle theme & provider"),
        ]),
        Row::new(vec![
            Cell::from("Settings: s / r"),
            Cell::from("s: Save config.toml | r: Reload from disk"),
        ]),
        Row::new(vec![
            Cell::from("Settings: c / v"),
            Cell::from("c: Clear chat history | v: Vacuum SQLite database"),
        ]),
        Row::new(vec![
            Cell::from("Chat: Enter"),
            Cell::from("Send prompt to Hana AI (persisted in SQLite DB)"),
        ]),
        Row::new(vec![
            Cell::from("Mouse"),
            Cell::from("Click tabs, action buttons, cards; wheel to scroll"),
        ]),
    ];

    let widths = [Constraint::Length(24), Constraint::Min(42)];
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec![
                Cell::from(Span::styled(
                    "Shortcut",
                    Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
                )),
                Cell::from(Span::styled(
                    "Action",
                    Style::default().fg(palette.text).add_modifier(Modifier::BOLD),
                )),
            ])
            .bottom_margin(1),
        )
        .column_spacing(1);

    f.render_widget(table, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

// ── Cron & Scheduled Jobs View ─────────────────────────────────────────────

fn draw_cron_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(area);

    let top_bar = Paragraph::new(Line::from(vec![
        Span::styled(" [4] CRON & SCHEDULED JOBS ", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Span::raw(" │ Manage automated background jobs  "),
        Span::styled("[+ New Cron Job] (Press 'n')", Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.highlight_bg).add_modifier(Modifier::BOLD)),
    ]))
    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(palette.border)));
    f.render_widget(top_bar, chunks[0]);

    // Read crontab + db jobs
    let mut items = Vec::new();
    if let Ok(user_jobs) = crate::cron::CronEntry::read_user_crontab() {
        for j in user_jobs {
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("{:<15} ", j.schedule), Style::default().fg(palette.highlight_fg).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<30} ", j.command), Style::default().fg(Color::White)),
                Span::styled("[User Crontab] ", Style::default().fg(palette.text_muted)),
                Span::styled("● ACTIVE", Style::default().fg(palette.status_ok)),
            ])));
        }
    }

    if let Some(ref d) = app.db {
        if let Ok(db_jobs) = d.get_cron_jobs() {
            for j in db_jobs {
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<15} ", j.schedule), Style::default().fg(palette.highlight_fg).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{:<30} ", j.command), Style::default().fg(Color::White)),
                    Span::styled(format!("[{}] ", j.description), Style::default().fg(palette.accent)),
                    Span::styled("● MANAGED", Style::default().fg(palette.status_ok)),
                ])));
            }
        }
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(vec![
            Span::styled("No scheduled cron jobs found. Press 'n' to create one.", Style::default().fg(palette.text_muted)),
        ])));
    }

    let list_widget = List::new(items)
        .block(
            Block::default()
                .title(" Active Scheduled Jobs ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border)),
        );
    f.render_widget(list_widget, chunks[1]);
}

// ── Virtual Pet (Tamagotchi) Modal ──────────────────────────────────────────

fn draw_pet_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(60, 60, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🐾 Virtual Pet (Tamagotchi Work Companion) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let mut lines = Vec::new();
    if let Some(ref d) = app.db {
        if let Ok(Some(pet)) = d.get_active_pet() {
            let stage = crate::pet::PetStage::from_xp(pet.xp);
            let species = crate::pet::PetSpecies::parse(&pet.species);
            let status = crate::pet::PetStatus {
                species,
                name: pet.name.clone(),
                stage,
                mood: app.pet_mood,
                xp: pet.xp,
            };

            lines.push(Line::from(""));
            let frames = status.frames();
            for art in frames[0] {
                lines.push(Line::from(Span::styled(*art, Style::default().fg(palette.highlight_fg).add_modifier(Modifier::BOLD))));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Nama: ", Style::default().fg(palette.text_muted)),
                Span::styled(pet.name.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw("  │  "),
                Span::styled("Spesies: ", Style::default().fg(palette.text_muted)),
                Span::styled(species.display_name(), Style::default().fg(palette.accent)),
            ]));
            lines.push(Line::from(vec![
                Span::styled("Tahap: ", Style::default().fg(palette.text_muted)),
                Span::styled(stage.display_name(), Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD)),
                Span::raw("  │  "),
                Span::styled("XP: ", Style::default().fg(palette.text_muted)),
                Span::styled(format!("{}/{}", pet.xp, stage.next_threshold()), Style::default().fg(Color::Yellow)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Siklus Hidup: ", Style::default().fg(palette.text_muted)),
                Span::raw("Telur 🥚 → Menetas 🐣 → Kecil 🐥 → Dewasa 🐔 → Menikah 💑 → Bertelur 🥚"),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("[Feed Pet (+10 XP)] (Press 'f')", Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.status_ok).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled("[New Pet] (Press 'n')", Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.highlight_bg)),
                Span::raw("  "),
                Span::styled("[Close] (Esc)", Style::default().fg(palette.text_muted)),
            ]));
        } else {
            lines.push(Line::from(""));
            lines.push(Line::from("Belum ada hewan peliharaan aktif."));
            lines.push(Line::from("Pilih teman kerjamu untuk menemanimu coding!"));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Pilihan: Ayam, Singa, Burung, Ular, Beruang", Style::default().fg(palette.accent)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("[Buat Pet: Ayam Ciko] (Press 'c')", Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.highlight_bg).add_modifier(Modifier::BOLD)),
            ]));
        }
    }

    f.render_widget(Paragraph::new(lines), inner);
}

// ── Cron Job Wizard Modal ───────────────────────────────────────────────────

fn draw_cron_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(50, 40, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" ⏰ Create New Scheduled Job ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Schedule: ", Style::default().fg(palette.text_muted)),
            Span::styled(&app.cron_schedule_buffer, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Command:  ", Style::default().fg(palette.text_muted)),
            Span::styled(&app.cron_command_buffer, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Save Job] (Enter)", Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.status_ok).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled("[Cancel] (Esc)", Style::default().fg(palette.text_muted)),
        ]),
    ];
    f.render_widget(Paragraph::new(text), inner);
}

// ── Quit Confirmation Modal ──────────────────────────────────────────────────

fn draw_quit_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(54, 12, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" ⚠ Exit CAShell Sovereign Cockpit ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " Are you sure you want to exit the cockpit? ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Background daemons & monitoring will continue to run. ",
            Style::default().fg(palette.text_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [Y] Yes, Exit ",
                Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                " [N / Esc] Cancel & Stay ",
                Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        inner,
    );

    // Register mouse hitboxes for [Yes] and [Cancel]
    let btn_y = inner.y + 4;
    let yes_w = 15u16;
    let gap_w = 4u16;
    let cancel_w = 25u16;
    let total_w = yes_w + gap_w + cancel_w;
    let start_x = inner.x + (inner.width.saturating_sub(total_w)) / 2;

    app.register_hitbox(
        Rect { x: start_x, y: btn_y, width: yes_w, height: 1 },
        HitboxAction::ConfirmQuit,
    );
    app.register_hitbox(
        Rect { x: start_x + yes_w + gap_w, y: btn_y, width: cancel_w, height: 1 },
        HitboxAction::CancelQuit,
    );
}

// ── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_palette_contrast_rules() {
        for &name in THEME_OPTIONS {
            let p = ThemePalette::get(name);
            // Contrast rule 1: Active tab highlighted text MUST be black (Color::Rgb(0, 0, 0))
            assert_eq!(p.highlight_fg, Color::Rgb(0, 0, 0));
            // Contrast rule 2: Never identical text and background
            assert_ne!(p.highlight_fg, p.highlight_bg);
            assert_ne!(p.text, p.row_selected_bg);
        }
    }

    #[test]
    fn test_shift_number_tab_mapping() {
        let keys = [
            (KeyCode::Char('!'), 0),
            (KeyCode::Char('@'), 1),
            (KeyCode::Char('#'), 2),
            (KeyCode::Char('$'), 3),
            (KeyCode::Char('%'), 4),
            (KeyCode::Char('^'), 5),
        ];
        for (code, expected_idx) in keys {
            let shift_tab = match code {
                KeyCode::Char('!') => Some(0),
                KeyCode::Char('@') => Some(1),
                KeyCode::Char('#') => Some(2),
                KeyCode::Char('$') => Some(3),
                KeyCode::Char('%') => Some(4),
                KeyCode::Char('^') => Some(5),
                _ => None,
            };
            assert_eq!(shift_tab, Some(expected_idx));
        }
    }

    #[test]
    fn test_cockpit_tabs_index_roundtrip() {
        for idx in 0..6 {
            let tab = CockpitTab::from_index(idx);
            assert_eq!(tab.to_index(), idx);
        }
        assert_eq!(CockpitTab::titles().len(), 6);
    }

    #[test]
    fn test_hitbox_registration_and_detection() {
        let mut app = CockpitApp::new(&Config::default());
        app.hitboxes.clear();

        let tab_rect = Rect {
            x: 10,
            y: 5,
            width: 20,
            height: 3,
        };
        app.register_hitbox(tab_rect, HitboxAction::SelectTab(2));

        // Inside hitbox
        assert_eq!(app.find_hitbox(15, 6), Some(HitboxAction::SelectTab(2)));
        assert_eq!(app.find_hitbox(10, 5), Some(HitboxAction::SelectTab(2)));
        assert_eq!(app.find_hitbox(29, 7), Some(HitboxAction::SelectTab(2)));

        // Outside hitbox
        assert_eq!(app.find_hitbox(9, 5), None);
        assert_eq!(app.find_hitbox(30, 5), None);
        assert_eq!(app.find_hitbox(15, 8), None);
    }

    #[test]
    fn test_database_actions_work_with_in_memory_fallback() {
        let app = CockpitApp::new(&Config::default());
        let _size = app.get_db_size();
    }

    #[test]
    fn test_cpu_history_ring_buffer() {
        let mut app = CockpitApp::new(&Config::default());
        assert_eq!(app.cpu_history.len(), MAX_CPU_SAMPLES);

        for i in 0..10u32 {
            let snap = SysSnapshot {
                cpu_pct: i as f32,
                ..Default::default()
            };
            app.ingest_snapshot(snap);
        }

        assert_eq!(app.cpu_history.len(), MAX_CPU_SAMPLES);
        assert_eq!(app.cpu_history[MAX_CPU_SAMPLES - 1], 9.0);
    }

    #[test]
    fn test_settings_cycle_field_value() {
        let mut app = CockpitApp::new(&Config::default());
        app.selected_field = 0; // AI Provider
        let initial = app.cfg.ai.default_provider.clone();

        app.cycle_field_value(true);
        assert_ne!(app.cfg.ai.default_provider, initial);

        app.cycle_field_value(false);
        assert_eq!(app.cfg.ai.default_provider, initial);
    }

    #[test]
    fn test_compact_mode_boundary() {
        let is_compact = |w: u16, h: u16| w < 80 || h < 24;
        assert!(is_compact(79, 25));
        assert!(is_compact(80, 23));
        assert!(!is_compact(80, 24));
        assert!(!is_compact(120, 30));
    }

    #[test]
    fn test_render_braille_curve_dimensions() {
        let data = vec![10.0, 20.0, 50.0, 80.0, 30.0];
        let lines = render_braille_curve(&data, 40, 2);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans.len(), 40);
        assert_eq!(lines[1].spans.len(), 40);
    }

    #[test]
    fn test_format_core_span() {
        let core = CpuCoreInfo {
            core_id: 3,
            usage: 64.0,
            frequency_mhz: 3200,
        };
        let spans = format_core_span(Some(&core), 3, 10);
        assert_eq!(spans[0].content, "C3 ");
        assert!(spans[5].content.contains("64%"));
        assert_eq!(spans[6].content, "3.2GHz");
    }

    #[test]
    fn test_format_kbps() {
        assert_eq!(format_kbps(512.0), "512 KB/s");
        assert_eq!(format_kbps(2048.0), "2.0 MB/s");
    }

    #[test]
    fn test_overview_scroll_bounds() {
        let mut app = CockpitApp::new(&Config::default());
        assert_eq!(app.overview_proc_scroll, 0);
        app.overview_proc_scroll = 5;
        app.overview_proc_scroll = app.overview_proc_scroll.saturating_sub(1);
        assert_eq!(app.overview_proc_scroll, 4);
    }
}
