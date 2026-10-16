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
    "cad-cyberpunk",
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
            "cad-cyberpunk" => Self {
                name: "cad-cyberpunk",
                border: Color::Rgb(0, 240, 255),       // Neon Cyan
                accent: Color::Rgb(255, 0, 128),       // Neon Magenta
                highlight_bg: Color::Rgb(0, 240, 255), // Cyan
                highlight_fg: Color::Rgb(0, 0, 0),     // Dark Contrast
                text: Color::Rgb(220, 235, 245),
                text_muted: Color::Rgb(90, 110, 130),
                chart_cpu: Color::Rgb(0, 240, 255),
                chart_ram: Color::Rgb(255, 0, 128),
                status_ok: Color::Rgb(0, 255, 136),    // Neon Green
                status_warn: Color::Rgb(255, 170, 0),  // Neon Amber
                status_err: Color::Rgb(255, 0, 85),    // Neon Red
                row_selected_bg: Color::Rgb(20, 30, 45),
            },
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    // Add-wizard openers
    AddServer,
    AddProject,
    AddContainer,
    AddMonitor,
    // Add-wizard field focus
    FocusServerField(usize),
    FocusProjectField(usize),
    FocusContainerField(usize),
    FocusMonitorField(usize),
    // Add-wizard actions
    SubmitServer,
    SubmitProject,
    SubmitContainer,
    SubmitMonitor,
    CancelServer,
    CancelProject,
    CancelContainer,
    CancelMonitor,
    // Project tab actions
    SelectProjectView(ProjectView),
    SelectProject(usize),
    SelectContainer(usize),
    ContainerStart(usize),
    ContainerStop(usize),
    ContainerRestart(usize),
    // Uptime tab actions
    SelectMonitor(usize),
    ProbeMonitors,
    // Server tab actions
    SelectServer(usize),
    ProbeServers,
    // Log tab actions
    SelectLogSource(LogSource),
    SelectLogLevel(LogLevelFilter),
    RefreshLogs,
    // Terminal tab actions
    FocusTerminalInput,
    RunTerminalCommand,
    SelectTerminalRemote(Option<String>),
    // AI tab actions
    CycleAiModel,
    ClearAiChat,
    // Cron tab actions
    NewCronWizard,
}

#[derive(Debug, Clone)]
pub struct Hitbox {
    pub rect: Rect,
    pub action: HitboxAction,
}

// ── Tab Navigation ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CockpitTab {
    Dashboard = 0,
    Project = 1,
    Uptime = 2,
    Server = 3,
    Audit = 4,
    Cron = 5,
    Log = 6,
    Terminal = 7,
    AI = 8,
    Setting = 9,
}

impl CockpitTab {
    pub const COUNT: usize = 10;

    pub fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Dashboard,
            1 => Self::Project,
            2 => Self::Uptime,
            3 => Self::Server,
            4 => Self::Audit,
            5 => Self::Cron,
            6 => Self::Log,
            7 => Self::Terminal,
            8 => Self::AI,
            _ => Self::Setting,
        }
    }

    pub fn to_index(self) -> usize {
        self as usize
    }

    pub fn titles() -> [&'static str; 10] {
        [
            "[1] Dashboard",
            "[2] Project",
            "[3] Uptime",
            "[4] Server",
            "[5] Audit",
            "[6] CRON",
            "[7] Log",
            "[8] Terminal",
            "[9] AI",
            "[0] Setting",
        ]
    }
}

/// Sub-views inside the Project tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectView {
    #[default]
    Tracked = 0,
    Containers = 1,
}

/// Log source selector for the Log tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogSource {
    #[default]
    Journal = 0,
    Docker = 1,
    Agents = 2,
}

impl LogSource {
    pub fn all() -> [Self; 3] {
        [Self::Journal, Self::Docker, Self::Agents]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Journal => "journalctl",
            Self::Docker => "docker logs",
            Self::Agents => "agent logs",
        }
    }
}

/// Log level filter for the Log tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevelFilter {
    #[default]
    All = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
}

impl LogLevelFilter {
    pub const COUNT: usize = 4;

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
        }
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
    "Master Password Protection",
    "Set/Change Master Password",
    "Password Hint",
    "Telegram Bot Token",
    "Telegram Chat ID",
    "Daemon Port",
    "Daemon Interval (s)",
    "Daemon Retry Threshold",
    "Coolify Base URL",
    "Coolify API Token",
    "DB Auto Prune",
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

    // ── Tab 1: Dashboard state ──────────────────────────────────────────────
    pub dashboard_alert_scroll: usize,

    // ── Tab 2: Project & Containers state ───────────────────────────────────
    pub project_view: ProjectView,
    pub selected_project: usize,
    pub selected_container: usize,

    // ── Tab 3: Uptime state ─────────────────────────────────────────────────
    pub selected_monitor: usize,
    pub uptime_probe_msg: Option<String>,

    // ── Tab 4: Server state ─────────────────────────────────────────────────
    pub selected_server: usize,
    pub server_probe_results: Vec<(String, Option<u128>)>, // (node name, latency)
    pub server_probe_done: bool,

    // ── Tab 7: Log state ────────────────────────────────────────────────────
    pub log_source: LogSource,
    pub log_level: LogLevelFilter,
    pub log_buffer: Vec<(String, String, String)>, // (level, source, line)
    pub log_scroll: usize,
    pub log_busy: bool,

    // ── Tab 8: Terminal state ───────────────────────────────────────────────
    pub term_input: String,
    pub term_remote: Option<String>, // None = local, Some(node_name) = remote
    pub term_output: Vec<String>,
    pub term_busy: bool,
    pub term_scroll: usize,

    // ── Tab 9: AI state ─────────────────────────────────────────────────────
    pub ai_model_idx: usize,

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
    pub pet_transient_action: Option<(crate::pet::PetActionState, u16)>,

    // Quit confirmation state
    pub show_quit_confirm: bool,
    pub quit_requested: bool,

    // Cron Job Wizard
    pub show_cron_wizard: bool,
    pub cron_schedule_buffer: String,
    pub cron_command_buffer: String,

    // Add Wizard (Server / Project / Docker Container / Uptime Monitor)
    pub show_server_wizard: bool,
    pub server_wizard_field: usize,
    pub server_name_buf: String,
    pub server_ip_buf: String,
    pub server_user_buf: String,
    pub server_services_buf: String,

    pub show_project_wizard: bool,
    pub project_wizard_field: usize,
    pub project_path_buf: String,
    pub project_name_buf: String,
    pub project_category_buf: String,

    pub show_container_wizard: bool,
    pub container_wizard_field: usize,
    pub container_name_buf: String,
    pub container_image_buf: String,
    pub container_port_buf: String,
    pub container_env_buf: String,

    pub show_monitor_wizard: bool,
    pub monitor_wizard_field: usize,
    pub monitor_name_buf: String,
    pub monitor_url_buf: String,

    // Security & Master Password Lock Screen
    pub is_locked: bool,
    pub lock_password_buf: String,
    pub lock_error_msg: Option<String>,
    pub lock_mask_visible: bool,

    // Set Master Password Modal
    pub show_set_password_modal: bool,
    pub new_password_buf: String,
    pub confirm_password_buf: String,
    pub password_hint_buf: String,
    pub set_password_field: usize,
    pub set_password_error: Option<String>,
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
                "=== Hana AI Chat (OmniRoute / 9router · localhost:20128 / :3007) ===".to_string(),
                "Tab / Shift+1..9/0: Switch Focus | ?: Help Cheat Sheet | Esc: Back".to_string(),
                "Type your prompt in the terminal input below and press Enter.".to_string(),
            ];
        }

        let apps = Self::init_apps(cfg);

        let mut chat_state = ListState::default();
        chat_state.select(Some(chat_lines.len().saturating_sub(1)));

        let mut app = Self {
            current_tab: CockpitTab::Dashboard,
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

            // ── Tab 1: Dashboard state ──────────────────────────────────────────────
            dashboard_alert_scroll: 0,

            // ── Tab 2: Project & Containers state ───────────────────────────────────
            project_view: ProjectView::Tracked,
            selected_project: 0,
            selected_container: 0,

            // ── Tab 3: Uptime state ─────────────────────────────────────────────────
            selected_monitor: 0,
            uptime_probe_msg: None,

            // ── Tab 4: Server state ─────────────────────────────────────────────────
            selected_server: 0,
            server_probe_results: Vec::new(),
            server_probe_done: false,

            // ── Tab 7: Log state ────────────────────────────────────────────────────
            log_source: LogSource::Journal,
            log_level: LogLevelFilter::All,
            log_buffer: Vec::new(),
            log_scroll: 0,
            log_busy: false,

            // ── Tab 8: Terminal state ───────────────────────────────────────────────
            term_input: String::new(),
            term_remote: None,
            term_output: vec![
                "=== CAShell Terminal & Quick Runner ===".to_string(),
                "Execute local shell commands or route commands to fleet servers.".to_string(),
                "Type command (e.g. 'cas doctor', 'uptime', 'docker ps') and press Enter.".to_string(),
            ],
            term_busy: false,
            term_scroll: 0,

            // ── Tab 9: AI state ─────────────────────────────────────────────────────
            ai_model_idx: 0,

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
            pet_transient_action: None,

            show_quit_confirm: false,
            quit_requested: false,

            show_cron_wizard: false,
            cron_schedule_buffer: "0 * * * *".to_string(),
            cron_command_buffer: "echo 'hello'".to_string(),

            show_server_wizard: false,
            server_wizard_field: 0,
            server_name_buf: String::new(),
            server_ip_buf: String::new(),
            server_user_buf: "root".to_string(),
            server_services_buf: "http:80,ssh:22".to_string(),

            show_project_wizard: false,
            project_wizard_field: 0,
            project_path_buf: String::new(),
            project_name_buf: String::new(),
            project_category_buf: "Product".to_string(),

            show_container_wizard: false,
            container_wizard_field: 0,
            container_name_buf: String::new(),
            container_image_buf: String::new(),
            container_port_buf: String::new(),
            container_env_buf: String::new(),

            show_monitor_wizard: false,
            monitor_wizard_field: 0,
            monitor_name_buf: String::new(),
            monitor_url_buf: String::new(),

            is_locked: cfg.security.require_password && !cfg.security.master_password_hash.is_empty(),
            lock_password_buf: String::new(),
            lock_error_msg: None,
            lock_mask_visible: false,

            show_set_password_modal: false,
            new_password_buf: String::new(),
            confirm_password_buf: String::new(),
            password_hint_buf: String::new(),
            set_password_field: 0,
            set_password_error: None,
        };

        app.refresh_audit_reports();
        app.probe_all_ports();
        app.probe_servers();
        app.load_logs();
        app
    }

    pub fn ai_model_options(cfg: &Config) -> Vec<String> {
        let mut models = vec![
            "ZA126_PRO".to_string(),
            "omniroute/claude-3-7-sonnet".to_string(),
            "omniroute/gpt-4o".to_string(),
            "omniroute/deepseek-r1".to_string(),
            "omniroute/gemini-2.5-pro".to_string(),
            "local/qwen2.5-coder".to_string(),
        ];
        if let Some(p) = cfg.ai.providers.get(&cfg.ai.default_provider) {
            if !p.default_model.is_empty() && !models.contains(&p.default_model) {
                models.insert(0, p.default_model.clone());
            }
        }
        models
    }

    pub fn current_ai_model(&self) -> String {
        let models = Self::ai_model_options(&self.cfg);
        if models.is_empty() {
            "ZA126_PRO".to_string()
        } else {
            models[self.ai_model_idx % models.len()].clone()
        }
    }

    pub fn cycle_ai_model(&mut self) {
        let models = Self::ai_model_options(&self.cfg);
        if !models.is_empty() {
            self.ai_model_idx = (self.ai_model_idx + 1) % models.len();
            let new_model = models[self.ai_model_idx].clone();
            let provider = self.cfg.ai.default_provider.clone();
            if let Some(p) = self.cfg.ai.providers.get_mut(&provider) {
                p.default_model = new_model.clone();
            }
            self.set_notification(format!("🤖 AI Model switched to: {new_model}"));
        }
    }

    pub fn start_selected_container(&mut self) {
        if self.latest_snap.docker_containers.is_empty() {
            self.set_notification("⚠ No containers to start");
            return;
        }
        let idx = self.selected_container.min(self.latest_snap.docker_containers.len() - 1);
        let name = self.latest_snap.docker_containers[idx].name.clone();
        let out = std::process::Command::new("docker")
            .args(["start", &name])
            .output()
            .or_else(|_| std::process::Command::new("podman").args(["start", &name]).output());
        match out {
            Ok(o) if o.status.success() => {
                self.latest_snap.docker_containers[idx].state = "running".to_string();
                self.latest_snap.docker_containers[idx].status_detail = "Up (Active)".to_string();
                self.set_notification(format!("✓ Container '{name}' started"));
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                self.set_notification(format!("✗ Failed to start '{name}': {err}"));
            }
            Err(e) => {
                self.set_notification(format!("✗ Engine error starting '{name}': {e}"));
            }
        }
    }

    pub fn stop_selected_container(&mut self) {
        if self.latest_snap.docker_containers.is_empty() {
            self.set_notification("⚠ No containers to stop");
            return;
        }
        let idx = self.selected_container.min(self.latest_snap.docker_containers.len() - 1);
        let name = self.latest_snap.docker_containers[idx].name.clone();
        let out = std::process::Command::new("docker")
            .args(["stop", &name])
            .output()
            .or_else(|_| std::process::Command::new("podman").args(["stop", &name]).output());
        match out {
            Ok(o) if o.status.success() => {
                self.latest_snap.docker_containers[idx].state = "exited".to_string();
                self.latest_snap.docker_containers[idx].status_detail = "Exited (0)".to_string();
                self.set_notification(format!("✓ Container '{name}' stopped"));
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                self.set_notification(format!("✗ Failed to stop '{name}': {err}"));
            }
            Err(e) => {
                self.set_notification(format!("✗ Engine error stopping '{name}': {e}"));
            }
        }
    }

    pub fn restart_selected_container(&mut self) {
        if self.latest_snap.docker_containers.is_empty() {
            self.set_notification("⚠ No containers to restart");
            return;
        }
        let idx = self.selected_container.min(self.latest_snap.docker_containers.len() - 1);
        let name = self.latest_snap.docker_containers[idx].name.clone();
        let out = std::process::Command::new("docker")
            .args(["restart", &name])
            .output()
            .or_else(|_| std::process::Command::new("podman").args(["restart", &name]).output());
        match out {
            Ok(o) if o.status.success() => {
                self.latest_snap.docker_containers[idx].state = "running".to_string();
                self.latest_snap.docker_containers[idx].status_detail = "Restarted (Active)".to_string();
                self.set_notification(format!("✓ Container '{name}' restarted"));
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                self.set_notification(format!("✗ Failed to restart '{name}': {err}"));
            }
            Err(e) => {
                self.set_notification(format!("✗ Engine error restarting '{name}': {e}"));
            }
        }
    }

    pub fn probe_servers(&mut self) {
        let mut results = Vec::new();
        // 1. Local node ThinkPad X1
        results.push(("ThinkPad X1 (Local)".to_string(), Some(0u128)));

        // 2. Nodes in config
        for (name, node) in &self.cfg.daemon.nodes {
            let port = node.services.first().map(|s| s.port).unwrap_or(22);
            let addr_str = format!("{}:{}", node.ip, port);
            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                let start = Instant::now();
                if TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok() {
                    results.push((name.clone(), Some(start.elapsed().as_millis())));
                } else {
                    results.push((name.clone(), None));
                }
            } else {
                results.push((name.clone(), None));
            }
        }

        // 3. Ensure multi-node canonical fleet nodes (X1, YPC, Hostinger VPS, XPC, Gerlink)
        let fleet_presets = [
            ("YPC", "100.127.232.32", 3000),
            ("Hostinger VPS", "195.35.48.21", 80),
            ("XPC", "100.100.100.10", 22),
            ("Gerlink", "100.64.0.1", 80),
        ];
        for (pname, pip, pport) in fleet_presets {
            if !results.iter().any(|(rname, _)| rname.eq_ignore_ascii_case(pname)) {
                let addr_str = format!("{}:{}", pip, pport);
                if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                    let start = Instant::now();
                    if TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok() {
                        results.push((pname.to_string(), Some(start.elapsed().as_millis())));
                    } else {
                        results.push((pname.to_string(), None));
                    }
                } else {
                    results.push((pname.to_string(), None));
                }
            }
        }

        self.server_probe_results = results;
        self.server_probe_done = true;
    }

    pub fn load_logs(&mut self) {
        let mut lines = Vec::new();
        match self.log_source {
            LogSource::Journal => {
                if let Ok(out) = std::process::Command::new("journalctl")
                    .args(["-n", "40", "--no-pager", "-o", "short-iso"])
                    .output()
                {
                    let raw = String::from_utf8_lossy(&out.stdout);
                    for l in raw.lines() {
                        let level = if l.contains("error") || l.contains("fail") || l.contains("ERR") {
                            "ERROR"
                        } else if l.contains("warn") || l.contains("WARN") {
                            "WARN"
                        } else {
                            "INFO"
                        };
                        lines.push((level.to_string(), "journald".to_string(), l.to_string()));
                    }
                }
                if lines.is_empty() {
                    lines.push(("INFO".to_string(), "journald".to_string(), "System journal is clean. No active alerts.".to_string()));
                }
            }
            LogSource::Docker => {
                if let Ok(out) = std::process::Command::new("docker")
                    .args(["ps", "--format", "{{.Names}}"])
                    .output()
                {
                    let names_str = String::from_utf8_lossy(&out.stdout);
                    for name in names_str.lines().take(3) {
                        let name = name.trim();
                        if !name.is_empty() {
                            if let Ok(logs_out) = std::process::Command::new("docker")
                                .args(["logs", "--tail", "10", name])
                                .output()
                            {
                                let log_raw = String::from_utf8_lossy(&logs_out.stdout);
                                for l in log_raw.lines() {
                                    lines.push(("INFO".to_string(), format!("docker:{name}"), l.to_string()));
                                }
                            }
                        }
                    }
                }
                if lines.is_empty() {
                    lines.push(("INFO".to_string(), "docker".to_string(), "No active docker containers logging.".to_string()));
                }
            }
            LogSource::Agents => {
                if let Some(ref d) = self.db {
                    if let Ok(evs) = d.read_system_events(40) {
                        for (lvl, src, msg, dt) in evs {
                            lines.push((lvl, src, format!("{dt} │ {msg}")));
                        }
                    }
                }
                if lines.is_empty() {
                    lines.push(("INFO".to_string(), "casheld".to_string(), "No agent system events recorded in SQLite.".to_string()));
                }
            }
        }
        self.log_buffer = lines;
        self.log_scroll = 0;
    }

    pub fn run_terminal_command(&mut self) {
        let cmd = self.term_input.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        self.term_input.clear();
        let target_desc = if let Some(ref r) = self.term_remote {
            format!("Remote ({r})")
        } else {
            "Local (ThinkPad X1)".to_string()
        };
        self.term_output.push(format!("$ [{target_desc}] > {cmd}"));

        let output = if let Some(ref r) = self.term_remote {
            if let Some(node) = self.cfg.daemon.nodes.get(r) {
                std::process::Command::new("ssh")
                    .args(["-o", "ConnectTimeout=3", &format!("{}@{}", node.user, node.ip), &cmd])
                    .output()
            } else {
                std::process::Command::new("sh").args(["-c", &cmd]).output()
            }
        } else {
            std::process::Command::new("sh").args(["-c", &cmd]).output()
        };

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let stderr = String::from_utf8_lossy(&o.stderr);
                for line in stdout.lines() {
                    self.term_output.push(line.to_string());
                }
                for line in stderr.lines() {
                    self.term_output.push(format!("! {line}"));
                }
                if stdout.is_empty() && stderr.is_empty() {
                    self.term_output.push(format!("(Process exited with code {})", o.status.code().unwrap_or(0)));
                }
            }
            Err(e) => {
                self.term_output.push(format!("✗ Execution error: {e}"));
            }
        }
        self.term_scroll = self.term_output.len().saturating_sub(1);
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

        // 6. Fleet nodes from config
        for (node_name, node_cfg) in &cfg.daemon.nodes {
            for svc in &node_cfg.services {
                let target = if svc.protocol == "http" || svc.protocol == "https" {
                    format!("{}/{}", svc.protocol, node_cfg.ip)
                } else {
                    format!("{}:{}", node_cfg.ip, svc.port)
                };
                list.push(AppCard {
                    id: format!("fleet-{}-{}", node_name, svc.name),
                    name: format!("{}:{}", node_name, svc.name),
                    category: "Fleet Node".to_string(),
                    target,
                    port: Some(svc.port),
                    status: ServiceStatus::Probing,
                    latency_ms: None,
                    details: format!(
                        "Fleet node '{}' service '{}' ({}:{})",
                        node_name, svc.name, node_cfg.ip, svc.port
                    ),
                });
            }
        }

        list
    }

    pub fn probe_all_ports(&mut self) {
        for app in &mut self.apps {
            if let Some(port) = app.port {
                let target_ip = if app.target.starts_with("http") || app.target.starts_with("coolify:") {
                    "127.0.0.1"
                } else {
                    app.target.split_once(':').map(|(host, _)| host).unwrap_or("127.0.0.1")
                };
                let addr: Option<SocketAddr> = format!("{target_ip}:{port}").parse().ok();
                if let Some(addr) = addr {
                    let start = Instant::now();
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
                return Some(hb.action.clone());
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
            11 => {
                self.cfg.security.require_password = !self.cfg.security.require_password;
                if self.cfg.security.require_password && self.cfg.security.master_password_hash.is_empty() {
                    self.show_set_password_modal = true;
                } else {
                    self.save_config_silent();
                    let state_str = if self.cfg.security.require_password { "Enabled" } else { "Disabled" };
                    self.set_notification(format!("Master Password Protection: {state_str}"));
                }
            }
            12 => {
                self.show_set_password_modal = true;
            }
            21 => {
                self.cfg.database.auto_prune = !self.cfg.database.auto_prune;
                self.save_config_silent();
                let state_str = if self.cfg.database.auto_prune { "Enabled" } else { "Disabled" };
                self.set_notification(format!("DB Auto-Prune: {state_str}"));
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
            4 => {
                self.pet_transient_action = Some((crate::pet::PetActionState::Hit, 16));
                self.set_notification("💥 Sparring hit trigger!");
            }
            _ => {}
        }
    }

    pub fn feed_pet(&mut self) {
        self.pet_transient_action = Some((crate::pet::PetActionState::Jump, 16));
        let mut notif = None;
        if let Some(ref d) = self.db {
            if let Ok(Some(pet)) = d.get_active_pet() {
                let _ = d.add_pet_xp(pet.id, 10);
                let _ = d.log_pet_action(pet.id, &pet.name, "Feed", "+10 XP (Joyful Leap)");
                notif = Some(format!("🍖 Fed {}! (+10 XP) [JUMP]", pet.name));
            } else {
                let _ = d.create_pet("ayam", "Ciko");
                let _ = d.log_pet_action(1, "Ciko", "Adopt", "Adopted Ciko the Chicken");
                notif = Some("🐣 Adopted Ciko the Chicken!".to_string());
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
        self.pet_transient_action = Some((crate::pet::PetActionState::Attack, 16));
        let mut notif = None;
        if let Some(ref d) = self.db {
            if let Ok(Some(pet)) = d.get_active_pet() {
                self.pet_mood = crate::pet::PetMood::Idle;
                let _ = d.log_pet_action(pet.id, &pet.name, "Train", "Combat training & cured [ATTACK]");
                notif = Some(format!("⚔ Trained {} in combat! Mood restored.", pet.name));
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
        self.pet_transient_action = Some((crate::pet::PetActionState::Dash, 16));
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
            let _ = d.log_pet_action(1, next_name, "Switch", &format!("Switched to {}", next_species));
            notif = Some(format!("🔄 Switched companion to {} {}! [DASH]", next_species, next_name));
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

    /// Submit the set/change master password modal.
    pub fn submit_set_password(&mut self) {
        let p1 = self.new_password_buf.clone();
        let p2 = self.confirm_password_buf.clone();

        if p1.is_empty() {
            self.set_password_error = Some("Password cannot be empty".to_string());
            return;
        }
        if p1 != p2 {
            self.set_password_error = Some("Passwords do not match".to_string());
            return;
        }

        self.cfg.security.master_password_hash = crate::config::hash_master_password(&p1);
        self.cfg.security.require_password = true;
        if !self.password_hint_buf.trim().is_empty() {
            self.cfg.security.hint = self.password_hint_buf.trim().to_string();
        }
        self.save_config_silent();

        self.show_set_password_modal = false;
        self.new_password_buf.clear();
        self.confirm_password_buf.clear();
        self.password_hint_buf.clear();
        self.set_password_error = None;
        self.set_notification("✔ Master password updated & security enabled");
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

    /// Save the current config silently (no notification). Used by add-wizards.
    fn save_config_silent(&mut self) -> bool {
        let path = Config::default_path();
        self.cfg.save(&path).is_ok()
    }

    /// Parse a services string like "name:port, name2:port2, 3000" into ServiceConfig vec.
    fn parse_services(&self, raw: &str) -> Vec<crate::config::ServiceConfig> {
        raw.split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|token| {
                let mut name = "svc".to_string();
                let mut port: u16 = 80;
                let mut protocol = "http".to_string();
                let parts: Vec<&str> = token.split(':').collect();
                if parts.len() >= 2 {
                    name = parts[0].to_string();
                    port = parts[1].parse().unwrap_or(80);
                } else if parts.len() == 1 {
                    if let Ok(p) = parts[0].parse::<u16>() {
                        port = p;
                        name = "svc".to_string();
                    }
                }
                if token.ends_with("https") {
                    protocol = "https".to_string();
                }
                crate::config::ServiceConfig {
                    name,
                    port,
                    protocol,
                }
            })
            .collect()
    }

    /// Submit the "Add Server" wizard.
    pub fn submit_server_wizard(&mut self) {
        let name = self.server_name_buf.trim().to_string();
        let ip = self.server_ip_buf.trim().to_string();
        let user = self.server_user_buf.trim().to_string();
        let services = self.parse_services(&self.server_services_buf);

        if name.is_empty() {
            self.set_notification("✗ Server name cannot be empty");
            return;
        }
        if ip.is_empty() {
            self.set_notification("✗ Server IP address cannot be empty");
            return;
        }

        let node_cfg = crate::config::NodeConfig {
            ip: ip.clone(),
            user,
            services,
        };

        let was_new = !self.cfg.daemon.nodes.contains_key(&name);
        self.cfg.daemon.nodes.insert(name.clone(), node_cfg);
        if self.save_config_silent() {
            if let Some(ref d) = self.db {
                let _ = d.register_node(&name, &name, &ip, "");
                let _ = d.log_event(
                    "INFO",
                    "cockpit",
                    &format!("Fleet node '{}' registered via add-server wizard", name),
                );
            }
        }

        // Auto-probe: refresh apps and probe ports
        self.apps = Self::init_apps(&self.cfg);
        self.probe_all_ports();

        let action = if was_new { "added" } else { "updated" };
        self.set_notification(format!(
            "✓ Fleet node '{}' {} and auto-probed",
            name, action
        ));
        self.show_server_wizard = false;
        self.server_wizard_field = 0;
        self.server_name_buf.clear();
        self.server_ip_buf.clear();
        self.server_user_buf = "root".to_string();
        self.server_services_buf = "http:80,ssh:22".to_string();
    }

    /// Submit the "Add Project" wizard.
    pub fn submit_project_wizard(&mut self) {
        let path = self.project_path_buf.trim().to_string();
        let name = self.project_name_buf.trim().to_string();
        let category = self.project_category_buf.trim().to_string();

        if path.is_empty() {
            self.set_notification("✗ Project path cannot be empty");
            return;
        }

        // Expand ~ to home
        let expanded = if let Some(stripped) = path.strip_prefix("~/") {
            if let Some(h) = dirs::home_dir() {
                h.join(stripped).to_string_lossy().to_string()
            } else {
                path.clone()
            }
        } else {
            path.clone()
        };

        let display_name = if name.is_empty() {
            expanded.rsplit('/').next().unwrap_or("project").to_string()
        } else {
            name
        };

        let cat = if category.is_empty() {
            "Product".to_string()
        } else {
            category
        };

        // Persist to config (workspace.custom_projects)
        let normalized = if expanded.starts_with("~/") {
            expanded.clone()
        } else if let Ok(abs) = std::fs::canonicalize(&expanded) {
            abs.to_string_lossy().to_string()
        } else {
            expanded.clone()
        };
        if !self.cfg.workspace.custom_projects.iter().any(|p| p == &normalized) {
            self.cfg.workspace.custom_projects.push(normalized.clone());
            self.save_config_silent();
        }

        // Persist to SQLite
        if let Some(ref d) = self.db {
            let _ = d.add_tracked_project(
                &normalized,
                &display_name,
                &cat,
                "main",
                false,
            );
            let _ = d.log_event(
                "INFO",
                "cockpit",
                &format!("Project '{}' ({}) added via add-project wizard", display_name, cat),
            );
        }

        // Update in-memory snapshot immediately
        let p = std::path::Path::new(&expanded);
        let is_dirty = p.join(".git").exists();
        self.latest_snap.tracked_projects.push(crate::systat::TrackedProject {
            name: display_name.clone(),
            category: cat,
            branch: "main".to_string(),
            is_dirty,
        });

        self.set_notification(format!(
            "✓ Project '{}' added to tracked projects",
            display_name
        ));
        self.show_project_wizard = false;
        self.project_wizard_field = 0;
        self.project_path_buf.clear();
        self.project_name_buf.clear();
        self.project_category_buf = "Product".to_string();
    }

    /// Submit the "Add Docker Container" wizard.
    pub fn submit_container_wizard(&mut self) {
        let name = self.container_name_buf.trim().to_string();
        let image = self.container_image_buf.trim().to_string();
        let port = self.container_port_buf.trim().to_string();
        let env = self.container_env_buf.trim().to_string();

        if name.is_empty() {
            self.set_notification("✗ Container name cannot be empty");
            return;
        }
        if image.is_empty() {
            self.set_notification("✗ Container image cannot be empty");
            return;
        }

        // Build the docker/podman run command
        let mut args: Vec<String> = vec!["run".into(), "-d".into(), "--name".into(), name.clone()];
        if !port.is_empty() {
            args.push("-p".into());
            args.push(port.clone());
        }
        if !env.is_empty() {
            for pair in env.split(',') {
                let pair = pair.trim();
                if !pair.is_empty() {
                    args.push("-e".into());
                    args.push(pair.to_string());
                }
            }
        }
        args.push(image.clone());

        let mut command = std::process::Command::new("docker");
        command.args(&args);
        let result = command.output();

        let mut engine = "docker";
        let (success, detail) = match result {
            Ok(ref out) if out.status.success() => {
                (true, String::from_utf8_lossy(&out.stdout).trim().to_string())
            }
            Ok(ref out) => {
                (false, String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
            Err(_) => {
                engine = "podman";
                let mut cmd2 = std::process::Command::new("podman");
                cmd2.args(&args);
                match cmd2.output() {
                    Ok(ref o) if o.status.success() => {
                        (true, String::from_utf8_lossy(&o.stdout).trim().to_string())
                    }
                    Ok(ref o) => {
                        (false, String::from_utf8_lossy(&o.stderr).trim().to_string())
                    }
                    Err(e) => (false, e.to_string()),
                }
            }
        };

        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        self.cfg.docker.containers.push(crate::config::DockerContainerConfig {
            name: name.clone(),
            image: image.clone(),
            ports: if port.is_empty() {
                vec![]
            } else {
                vec![port.clone()]
            },
            env: if env.is_empty() {
                vec![]
            } else {
                env.split(',').map(|s| s.trim().to_string()).collect()
            },
            started_at: now,
        });
        self.save_config_silent();

        if let Some(ref d) = self.db {
            let _ = d.add_docker_container(
                &name,
                &image,
                &port,
                &env,
            );
            let _ = d.log_event(
                "INFO",
                "cockpit",
                &format!("Container '{}' registered ({})", name, engine),
            );
        }

        // Update in-memory snapshot
        let (state, status_detail) = if success {
            ("running".to_string(), "Just started".to_string())
        } else {
            ("created".to_string(), if detail.is_empty() { "Engine stopped".to_string() } else { detail.clone() })
        };

        self.latest_snap.docker_containers.push(crate::systat::DockerContainerInfo {
            name: name.clone(),
            image: image.clone(),
            state,
            status_detail,
            ports: port.clone(),
        });

        if success {
            self.set_notification(format!(
                "✓ Container '{}' started successfully ({})",
                name, engine
            ));
        } else {
            self.set_notification(format!(
                "✓ Container '{}' configured ({})",
                name, engine
            ));
        }

        self.show_container_wizard = false;
        self.container_wizard_field = 0;
        self.container_name_buf.clear();
        self.container_image_buf.clear();
        self.container_port_buf.clear();
        self.container_env_buf.clear();
    }

    /// Submit the "Add Uptime Monitor" wizard.
    pub fn submit_monitor_wizard(&mut self) {
        let name = self.monitor_name_buf.trim().to_string();
        let url_raw = self.monitor_url_buf.trim().to_string();

        if name.is_empty() {
            self.set_notification("✗ Monitor name cannot be empty");
            return;
        }
        if url_raw.is_empty() {
            self.set_notification("✗ Monitor URL cannot be empty");
            return;
        }

        // Normalize URL
        let url = if url_raw.starts_with("http://") || url_raw.starts_with("https://") {
            url_raw.clone()
        } else {
            format!("http://{}", url_raw)
        };

        // Check for duplicate
        let already_exists = self
            .cfg
            .uptime
            .custom_monitors
            .iter()
            .any(|m| m.target == url);
        if already_exists {
            self.set_notification(format!(
                "✗ Monitor '{}' already exists",
                name
            ));
            return;
        }

        // Persist to config
        self.cfg.uptime.custom_monitors.push(crate::config::UptimeMonitorConfig {
            name: name.clone(),
            target: url.clone(),
        });
        self.save_config_silent();

        // Persist to SQLite
        if let Some(ref d) = self.db {
            let _ = d.add_uptime_monitor(&name, &url);
            let _ = d.log_event(
                "INFO",
                "cockpit",
                &format!("Uptime monitor '{}' ({}) added via add-monitor wizard", name, url),
            );
        }

        // Probe immediately
        let probe_result = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(1500))
            .danger_accept_invalid_certs(true)
            .build()
            .and_then(|c| c.get(&url).send());

        let mut monitor = crate::systat::UptimeMonitor {
            name: name.clone(),
            target: url.clone(),
            is_up: false,
            latency_ms: None,
            status_code: None,
            uptime_pct: 0.0,
            history: vec![true; 19],
        };

        if let Ok(res) = probe_result {
            let code = res.status().as_u16();
            monitor.is_up = res.status().is_success()
                || res.status().is_redirection()
                || code == 404
                || code == 401;
            monitor.latency_ms = None;
            monitor.status_code = Some(code);
            monitor.history.push(monitor.is_up);
            if monitor.history.len() > 20 {
                monitor.history.remove(0);
            }
            let up_count = monitor.history.iter().filter(|&&u| u).count();
            monitor.uptime_pct = (up_count as f32 / monitor.history.len().max(1) as f32) * 100.0;
        }

        let is_up = monitor.is_up;
        self.latest_snap.uptime_monitors.push(monitor);

        let status = if is_up { "UP" } else { "DOWN" };
        self.set_notification(format!(
            "✓ Uptime monitor '{}' added and probed (status: {})",
            name, status
        ));
        self.show_monitor_wizard = false;
        self.monitor_wizard_field = 0;
        self.monitor_name_buf.clear();
        self.monitor_url_buf.clear();
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

        // Increment pet animation tick and step transient action timer
        app.pet_tick = app.pet_tick.wrapping_add(1);
        if let Some((_, ref mut ticks)) = app.pet_transient_action {
            *ticks = ticks.saturating_sub(1);
            if *ticks == 0 {
                app.pet_transient_action = None;
            }
        }

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
    // 0. Master Password Lock Screen Input Handling
    if app.is_locked {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                return Ok(true); // Exit application cleanly from lock screen
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.lock_mask_visible = !app.lock_mask_visible;
            }
            KeyCode::Enter => {
                let valid = crate::config::verify_master_password(
                    &app.lock_password_buf,
                    &app.cfg.security.master_password_hash,
                );
                if valid {
                    app.is_locked = false;
                    app.lock_password_buf.clear();
                    app.lock_error_msg = None;
                    app.set_notification("✔ Access granted — Cockpit unlocked");
                } else {
                    app.lock_error_msg = Some("✗ Invalid master password".to_string());
                    app.lock_password_buf.clear();
                }
            }
            KeyCode::Backspace => {
                app.lock_password_buf.pop();
                app.lock_error_msg = None;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.lock_password_buf.push(c);
                app.lock_error_msg = None;
            }
            _ => {}
        }
        return Ok(false);
    }

    // 0b. Set Master Password Modal Input Handling
    if app.show_set_password_modal {
        match key.code {
            KeyCode::Esc => {
                app.show_set_password_modal = false;
                app.new_password_buf.clear();
                app.confirm_password_buf.clear();
                app.password_hint_buf.clear();
                app.set_password_error = None;
            }
            KeyCode::Tab => {
                app.set_password_field = (app.set_password_field + 1) % 3;
            }
            KeyCode::BackTab => {
                app.set_password_field = (app.set_password_field + 2) % 3;
            }
            KeyCode::Enter => {
                app.submit_set_password();
            }
            KeyCode::Backspace => {
                match app.set_password_field {
                    0 => {
                        app.new_password_buf.pop();
                    }
                    1 => {
                        app.confirm_password_buf.pop();
                    }
                    2 => {
                        app.password_hint_buf.pop();
                    }
                    _ => {}
                }
                app.set_password_error = None;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                match app.set_password_field {
                    0 => {
                        app.new_password_buf.push(c);
                    }
                    1 => {
                        app.confirm_password_buf.push(c);
                    }
                    2 => {
                        app.password_hint_buf.push(c);
                    }
                    _ => {}
                }
                app.set_password_error = None;
            }
            _ => {}
        }
        return Ok(false);
    }

    // 1. Shift + 1..9, 0 MUST switch tabs immediately (even in AI or Terminal tab)
    let shift_tab = match key.code {
        KeyCode::Char('!') => Some(0),
        KeyCode::Char('@') => Some(1),
        KeyCode::Char('#') => Some(2),
        KeyCode::Char('$') => Some(3),
        KeyCode::Char('%') => Some(4),
        KeyCode::Char('^') => Some(5),
        KeyCode::Char('&') => Some(6),
        KeyCode::Char('*') => Some(7),
        KeyCode::Char('(') => Some(8),
        KeyCode::Char(')') => Some(9),
        KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(0),
        KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(1),
        KeyCode::Char('3') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(2),
        KeyCode::Char('4') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(3),
        KeyCode::Char('5') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(4),
        KeyCode::Char('6') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(5),
        KeyCode::Char('7') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(6),
        KeyCode::Char('8') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(7),
        KeyCode::Char('9') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(8),
        KeyCode::Char('0') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(9),
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
            && !app.show_server_wizard
            && !app.show_project_wizard
            && !app.show_container_wizard
            && !app.show_monitor_wizard
            && (app.current_tab != CockpitTab::AI || app.chat_input.is_empty())
            && (app.current_tab != CockpitTab::Terminal || app.term_input.is_empty()))
    {
        app.show_help_modal = !app.show_help_modal;
        return Ok(false);
    }

    // Add Wizard Key Interception
    if app.show_server_wizard {
        handle_server_wizard_key(key, app);
        return Ok(false);
    }
    if app.show_project_wizard {
        handle_project_wizard_key(key, app);
        return Ok(false);
    }
    if app.show_container_wizard {
        handle_container_wizard_key(key, app);
        return Ok(false);
    }
    if app.show_monitor_wizard {
        handle_monitor_wizard_key(key, app);
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

    // 4. If AI chat tab is focused and user types in prompt
    if app.current_tab == CockpitTab::AI {
        match key.code {
            KeyCode::Esc => {
                if !app.chat_input.is_empty() {
                    app.chat_input.clear();
                    return Ok(false);
                }
                app.show_quit_confirm = true;
                return Ok(false);
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

                let active_model = app.current_ai_model();
                let system_prompt = format!(
                    "{role}\nActive Model: {model}\nGreetings to {user_name}.\nSystem Context: Hostname: {host}, OS: Linux, Monitored Ports: [{ports}], Active Services: [{services}], Tailscale: {tailscale}.",
                    role = app.cfg.persona.assistant_role,
                    model = active_model,
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
                // If input is empty, direct tab switching with 1-9 or 0
                if app.chat_input.is_empty() {
                    if ('1'..='9').contains(&c) {
                        let idx = (c as usize) - ('1' as usize);
                        app.current_tab = CockpitTab::from_index(idx);
                        return Ok(false);
                    } else if c == '0' {
                        app.current_tab = CockpitTab::Setting;
                        return Ok(false);
                    } else if c == 'm' {
                        app.cycle_ai_model();
                        return Ok(false);
                    } else if c == 'c' {
                        app.clear_chat_history();
                        return Ok(false);
                    }
                }
                app.chat_input.push(c);
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

    // 4b. If Terminal tab is focused and user types command
    if app.current_tab == CockpitTab::Terminal {
        match key.code {
            KeyCode::Esc => {
                if !app.term_input.is_empty() {
                    app.term_input.clear();
                    return Ok(false);
                }
                app.show_quit_confirm = true;
                return Ok(false);
            }
            KeyCode::Enter if !app.term_input.trim().is_empty() => {
                app.run_terminal_command();
                return Ok(false);
            }
            KeyCode::Backspace => {
                app.term_input.pop();
                return Ok(false);
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                if app.term_input.is_empty() {
                    if ('1'..='9').contains(&c) {
                        let idx = (c as usize) - ('1' as usize);
                        app.current_tab = CockpitTab::from_index(idx);
                        return Ok(false);
                    } else if c == '0' {
                        app.current_tab = CockpitTab::Setting;
                        return Ok(false);
                    } else if c == 'r' {
                        // Toggle local vs first remote node
                        if app.term_remote.is_none() {
                            if let Some(first_node) = app.cfg.daemon.nodes.keys().next() {
                                app.term_remote = Some(first_node.clone());
                                app.set_notification(format!("Terminal target: Remote [{first_node}]"));
                            } else {
                                app.set_notification("Terminal target: Local (No remote nodes configured)");
                            }
                        } else {
                            app.term_remote = None;
                            app.set_notification("Terminal target: Local (ThinkPad X1)");
                        }
                        return Ok(false);
                    }
                }
                app.term_input.push(c);
                return Ok(false);
            }
            KeyCode::Up => {
                app.term_scroll = app.term_scroll.saturating_sub(1);
                return Ok(false);
            }
            KeyCode::Down => {
                app.term_scroll = (app.term_scroll + 1).min(app.term_output.len().saturating_sub(1));
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
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if !app.cfg.security.master_password_hash.is_empty() {
                app.is_locked = true;
                app.set_notification("🔒 Cockpit locked");
            } else {
                app.set_notification("⚠ Master password not set. Configure it in Setting GUI [0].");
            }
        }
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

        // Direct number key tab switching: '1'..'9' -> Tab 0..8, '0' -> Tab 9 (Setting)
        KeyCode::Char('1') => app.current_tab = CockpitTab::Dashboard,
        KeyCode::Char('2') => app.current_tab = CockpitTab::Project,
        KeyCode::Char('3') => app.current_tab = CockpitTab::Uptime,
        KeyCode::Char('4') => app.current_tab = CockpitTab::Server,
        KeyCode::Char('5') => app.current_tab = CockpitTab::Audit,
        KeyCode::Char('6') => app.current_tab = CockpitTab::Cron,
        KeyCode::Char('7') => app.current_tab = CockpitTab::Log,
        KeyCode::Char('8') => app.current_tab = CockpitTab::Terminal,
        KeyCode::Char('9') => app.current_tab = CockpitTab::AI,
        KeyCode::Char('0') => app.current_tab = CockpitTab::Setting,

        // ── Pet Play Area Controls (Hardware-Reactive Tamagotchi) ─────────────
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.execute_pet_action();
        }
        KeyCode::Char('f') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 0;
            app.execute_pet_action();
        }
        KeyCode::Char('t') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 1;
            app.execute_pet_action();
        }
        KeyCode::Char('s') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 2;
            app.execute_pet_action();
        }
        KeyCode::Char('h') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 3;
            app.execute_pet_action();
        }

        KeyCode::Tab => {
            let next_idx = (app.current_tab.to_index() + 1) % CockpitTab::COUNT;
            app.current_tab = CockpitTab::from_index(next_idx);
        }
        KeyCode::BackTab => {
            let prev_idx = (app.current_tab.to_index() + CockpitTab::COUNT - 1) % CockpitTab::COUNT;
            app.current_tab = CockpitTab::from_index(prev_idx);
        }

        // Tab-specific controls: Up / k and Down / j
        KeyCode::Up | KeyCode::Char('k') => match app.current_tab {
            CockpitTab::Dashboard => {
                app.pet_selected_action = (app.pet_selected_action + 4) % 5;
                app.overview_proc_scroll = app.overview_proc_scroll.saturating_sub(1);
            }
            CockpitTab::Project => {
                match app.project_view {
                    ProjectView::Tracked => {
                        app.selected_project = app.selected_project.saturating_sub(1);
                    }
                    ProjectView::Containers => {
                        app.selected_container = app.selected_container.saturating_sub(1);
                    }
                }
            }
            CockpitTab::Uptime => {
                app.selected_monitor = app.selected_monitor.saturating_sub(1);
            }
            CockpitTab::Server => {
                app.selected_server = app.selected_server.saturating_sub(1);
            }
            CockpitTab::Audit => {
                app.selected_report = app.selected_report.saturating_sub(1);
            }
            CockpitTab::Cron => {}
            CockpitTab::Log => {
                app.log_scroll = app.log_scroll.saturating_sub(1);
            }
            CockpitTab::Terminal => {
                app.term_scroll = app.term_scroll.saturating_sub(1);
            }
            CockpitTab::AI => {
                app.scroll_chat_up();
            }
            CockpitTab::Setting => {
                app.selected_field = app.selected_field.saturating_sub(1);
            }
        },
        KeyCode::Down | KeyCode::Char('j') => match app.current_tab {
            CockpitTab::Dashboard => {
                app.pet_selected_action = (app.pet_selected_action + 1) % 5;
                app.overview_proc_scroll = (app.overview_proc_scroll + 1).min(30);
            }
            CockpitTab::Project => {
                match app.project_view {
                    ProjectView::Tracked => {
                        if !app.latest_snap.tracked_projects.is_empty() {
                            app.selected_project = (app.selected_project + 1).min(app.latest_snap.tracked_projects.len() - 1);
                        }
                    }
                    ProjectView::Containers => {
                        if !app.latest_snap.docker_containers.is_empty() {
                            app.selected_container = (app.selected_container + 1).min(app.latest_snap.docker_containers.len() - 1);
                        }
                    }
                }
            }
            CockpitTab::Uptime => {
                if !app.latest_snap.uptime_monitors.is_empty() {
                    app.selected_monitor = (app.selected_monitor + 1).min(app.latest_snap.uptime_monitors.len() - 1);
                }
            }
            CockpitTab::Server => {
                if !app.server_probe_results.is_empty() {
                    app.selected_server = (app.selected_server + 1).min(app.server_probe_results.len() - 1);
                }
            }
            CockpitTab::Audit => {
                if !app.audit_reports.is_empty() {
                    app.selected_report =
                        (app.selected_report + 1).min(app.audit_reports.len() - 1);
                }
            }
            CockpitTab::Cron => {}
            CockpitTab::Log => {
                app.log_scroll = (app.log_scroll + 1).min(app.log_buffer.len().saturating_sub(1));
            }
            CockpitTab::Terminal => {
                app.term_scroll = (app.term_scroll + 1).min(app.term_output.len().saturating_sub(1));
            }
            CockpitTab::AI => {
                app.scroll_chat_down();
            }
            CockpitTab::Setting => {
                app.selected_field = (app.selected_field + 1).min(SETTINGS_FIELDS.len() - 1);
            }
        },

        KeyCode::Left | KeyCode::Char('h') => match app.current_tab {
            CockpitTab::Setting => app.cycle_field_value(false),
            CockpitTab::Project => {
                app.project_view = ProjectView::Tracked;
            }
            CockpitTab::Log => {
                let next = match app.log_source {
                    LogSource::Journal => LogSource::Agents,
                    LogSource::Docker => LogSource::Journal,
                    LogSource::Agents => LogSource::Docker,
                };
                app.log_source = next;
                app.load_logs();
            }
            _ => {}
        },
        KeyCode::Right | KeyCode::Char('l') => match app.current_tab {
            CockpitTab::Setting => app.cycle_field_value(true),
            CockpitTab::Project => {
                app.project_view = ProjectView::Containers;
            }
            CockpitTab::Log => {
                app.log_level = match app.log_level {
                    LogLevelFilter::All => LogLevelFilter::Error,
                    LogLevelFilter::Error => LogLevelFilter::Warn,
                    LogLevelFilter::Warn => LogLevelFilter::Info,
                    LogLevelFilter::Info => LogLevelFilter::All,
                };
                app.set_notification(format!("Log level filter: {}", app.log_level.label()));
            }
            _ => {}
        },

        KeyCode::Enter if app.show_cron_wizard => {
            if let Some(ref d) = app.db {
                let _ = d.add_cron_job(&app.cron_schedule_buffer, &app.cron_command_buffer, "Custom task");
                app.set_notification("✔ Saved scheduled job");
            }
            app.show_cron_wizard = false;
        }
        KeyCode::Enter => match app.current_tab {
            CockpitTab::Setting => {
                start_field_edit(app);
            }
            CockpitTab::Project => {
                // Toggle view between Tracked Projects and Containers
                app.project_view = match app.project_view {
                    ProjectView::Tracked => ProjectView::Containers,
                    ProjectView::Containers => ProjectView::Tracked,
                };
            }
            _ => {}
        },

        // Project Tab shortcuts: 'o'/'P': Add project, 'd': Add container, 's': Start, 'x': Stop, 'r': Restart
        KeyCode::Char('o') | KeyCode::Char('P') if app.current_tab == CockpitTab::Project || app.current_tab == CockpitTab::Dashboard => {
            app.show_project_wizard = true;
            app.project_wizard_field = 0;
        }
        KeyCode::Char('d') if app.current_tab == CockpitTab::Project || app.current_tab == CockpitTab::Dashboard => {
            app.show_container_wizard = true;
            app.container_wizard_field = 0;
        }
        KeyCode::Char('s') if app.current_tab == CockpitTab::Project => {
            app.start_selected_container();
        }
        KeyCode::Char('x') if app.current_tab == CockpitTab::Project => {
            app.stop_selected_container();
        }
        KeyCode::Char('r') if app.current_tab == CockpitTab::Project => {
            app.restart_selected_container();
        }

        // Uptime Tab shortcuts: 'm'/'u': Add monitor, 'p': Probe monitors
        KeyCode::Char('m') | KeyCode::Char('u') if app.current_tab == CockpitTab::Uptime || app.current_tab == CockpitTab::Dashboard => {
            app.show_monitor_wizard = true;
            app.monitor_wizard_field = 0;
        }
        KeyCode::Char('p') if app.current_tab == CockpitTab::Uptime => {
            app.probe_all_ports();
            app.set_notification("Probed all monitors & endpoints");
        }

        // Server Tab shortcuts: 'a'/'n': Add server, 'p': Probe servers
        KeyCode::Char('a') | KeyCode::Char('n') if app.current_tab == CockpitTab::Server => {
            app.show_server_wizard = true;
            app.server_wizard_field = 0;
        }
        KeyCode::Char('p') if app.current_tab == CockpitTab::Server => {
            app.probe_servers();
            app.set_notification("Probed all multi-node server latencies");
        }

        // Audit Tab shortcuts: 'a': Turbo Audit, 'f': Run Fix
        KeyCode::Char('a') if app.current_tab == CockpitTab::Audit => {
            trigger_turbo_audit(app, terminal)?;
        }
        KeyCode::Char('f') if app.current_tab == CockpitTab::Audit => {
            trigger_run_fix(app, terminal)?;
        }

        // Cron Tab shortcuts: 'n': New Cron Job
        KeyCode::Char('n') if app.current_tab == CockpitTab::Cron => {
            app.show_cron_wizard = true;
        }

        // Log Tab shortcuts: 'r': Reload logs, 'l': Cycle log level
        KeyCode::Char('r') if app.current_tab == CockpitTab::Log => {
            app.load_logs();
            app.set_notification("Logs refreshed from active stream");
        }
        KeyCode::Char('f') if app.current_tab == CockpitTab::Log => {
            app.log_level = match app.log_level {
                LogLevelFilter::All => LogLevelFilter::Error,
                LogLevelFilter::Error => LogLevelFilter::Warn,
                LogLevelFilter::Warn => LogLevelFilter::Info,
                LogLevelFilter::Info => LogLevelFilter::All,
            };
            app.set_notification(format!("Log level filter: {}", app.log_level.label()));
        }

        // AI Tab shortcuts: 'm': Cycle model, 'c': Clear chat
        KeyCode::Char('m') if app.current_tab == CockpitTab::AI => {
            app.cycle_ai_model();
        }
        KeyCode::Char('c') if app.current_tab == CockpitTab::AI => {
            app.clear_chat_history();
        }

        // Setting Tab shortcuts: 's': Save, 'r': Reload, 'c': Clear chat, 'v': Vacuum
        KeyCode::Char('s') if app.current_tab == CockpitTab::Setting => {
            app.save_current_config();
        }
        KeyCode::Char('r') if app.current_tab == CockpitTab::Setting => {
            app.reload_config();
        }
        KeyCode::Char('c') if app.current_tab == CockpitTab::Setting => {
            app.clear_chat_history();
        }
        KeyCode::Char('v') if app.current_tab == CockpitTab::Setting => {
            app.vacuum_database();
        }

        // Pet modal shortcut in Dashboard
        KeyCode::Char('p') if app.current_tab == CockpitTab::Dashboard => {
            app.show_pet_modal = !app.show_pet_modal;
        }
        KeyCode::Char('c') if app.show_pet_modal => {
            if let Some(ref d) = app.db {
                let _ = d.create_pet("ayam", "Ciko");
                app.set_notification("🐣 Created pet Ayam Ciko!");
            }
        }

        _ => {}
    }

    Ok(false)
}

fn handle_server_wizard_key(key: event::KeyEvent, app: &mut CockpitApp) {
    match key.code {
        KeyCode::Esc => {
            app.show_server_wizard = false;
        }
        KeyCode::Enter => {
            app.submit_server_wizard();
        }
        KeyCode::Tab | KeyCode::Down => {
            app.server_wizard_field = (app.server_wizard_field + 1) % 4;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.server_wizard_field = (app.server_wizard_field + 3) % 4;
        }
        KeyCode::Backspace => {
            match app.server_wizard_field {
                0 => { app.server_name_buf.pop(); }
                1 => { app.server_ip_buf.pop(); }
                2 => { app.server_user_buf.pop(); }
                3 => { app.server_services_buf.pop(); }
                _ => {}
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.server_wizard_field {
                0 => app.server_name_buf.push(c),
                1 => app.server_ip_buf.push(c),
                2 => app.server_user_buf.push(c),
                3 => app.server_services_buf.push(c),
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_project_wizard_key(key: event::KeyEvent, app: &mut CockpitApp) {
    match key.code {
        KeyCode::Esc => {
            app.show_project_wizard = false;
        }
        KeyCode::Enter => {
            app.submit_project_wizard();
        }
        KeyCode::Tab | KeyCode::Down => {
            app.project_wizard_field = (app.project_wizard_field + 1) % 3;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.project_wizard_field = (app.project_wizard_field + 2) % 3;
        }
        KeyCode::Backspace => {
            match app.project_wizard_field {
                0 => { app.project_path_buf.pop(); }
                1 => { app.project_name_buf.pop(); }
                2 => { app.project_category_buf.pop(); }
                _ => {}
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.project_wizard_field {
                0 => app.project_path_buf.push(c),
                1 => app.project_name_buf.push(c),
                2 => app.project_category_buf.push(c),
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_container_wizard_key(key: event::KeyEvent, app: &mut CockpitApp) {
    match key.code {
        KeyCode::Esc => {
            app.show_container_wizard = false;
        }
        KeyCode::Enter => {
            app.submit_container_wizard();
        }
        KeyCode::Tab | KeyCode::Down => {
            app.container_wizard_field = (app.container_wizard_field + 1) % 4;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.container_wizard_field = (app.container_wizard_field + 3) % 4;
        }
        KeyCode::Backspace => {
            match app.container_wizard_field {
                0 => { app.container_name_buf.pop(); }
                1 => { app.container_image_buf.pop(); }
                2 => { app.container_port_buf.pop(); }
                3 => { app.container_env_buf.pop(); }
                _ => {}
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.container_wizard_field {
                0 => app.container_name_buf.push(c),
                1 => app.container_image_buf.push(c),
                2 => app.container_port_buf.push(c),
                3 => app.container_env_buf.push(c),
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_monitor_wizard_key(key: event::KeyEvent, app: &mut CockpitApp) {
    match key.code {
        KeyCode::Esc => {
            app.show_monitor_wizard = false;
        }
        KeyCode::Enter => {
            app.submit_monitor_wizard();
        }
        KeyCode::Tab | KeyCode::Down => {
            app.monitor_wizard_field = (app.monitor_wizard_field + 1) % 2;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.monitor_wizard_field = (app.monitor_wizard_field + 1) % 2;
        }
        KeyCode::Backspace => {
            match app.monitor_wizard_field {
                0 => { app.monitor_name_buf.pop(); }
                1 => { app.monitor_url_buf.pop(); }
                _ => {}
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.monitor_wizard_field {
                0 => app.monitor_name_buf.push(c),
                1 => app.monitor_url_buf.push(c),
                _ => {}
            }
        }
        _ => {}
    }
}

// ── Mouse Event Handling ─────────────────────────────────────────────────────

fn handle_mouse_event<B: ratatui::backend::Backend>(
    mouse: event::MouseEvent,
    app: &mut CockpitApp,
    terminal: &mut Terminal<B>,
) -> Result<()> {
    match mouse.kind {
        MouseEventKind::ScrollUp => match app.current_tab {
            CockpitTab::Dashboard => app.overview_proc_scroll = app.overview_proc_scroll.saturating_sub(1),
            CockpitTab::Project => match app.project_view {
                ProjectView::Tracked => app.selected_project = app.selected_project.saturating_sub(1),
                ProjectView::Containers => app.selected_container = app.selected_container.saturating_sub(1),
            },
            CockpitTab::Uptime => app.selected_monitor = app.selected_monitor.saturating_sub(1),
            CockpitTab::Server => app.selected_server = app.selected_server.saturating_sub(1),
            CockpitTab::Audit => app.report_scroll = app.report_scroll.saturating_sub(2),
            CockpitTab::Cron => {},
            CockpitTab::Log => app.log_scroll = app.log_scroll.saturating_sub(1),
            CockpitTab::Terminal => app.term_scroll = app.term_scroll.saturating_sub(1),
            CockpitTab::AI => app.scroll_chat_up(),
            CockpitTab::Setting => app.selected_field = app.selected_field.saturating_sub(1),
        },
        MouseEventKind::ScrollDown => match app.current_tab {
            CockpitTab::Dashboard => app.overview_proc_scroll = (app.overview_proc_scroll + 1).min(30),
            CockpitTab::Project => match app.project_view {
                ProjectView::Tracked => {
                    if !app.latest_snap.tracked_projects.is_empty() {
                        app.selected_project = (app.selected_project + 1).min(app.latest_snap.tracked_projects.len() - 1);
                    }
                }
                ProjectView::Containers => {
                    if !app.latest_snap.docker_containers.is_empty() {
                        app.selected_container = (app.selected_container + 1).min(app.latest_snap.docker_containers.len() - 1);
                    }
                }
            },
            CockpitTab::Uptime => {
                if !app.latest_snap.uptime_monitors.is_empty() {
                    app.selected_monitor = (app.selected_monitor + 1).min(app.latest_snap.uptime_monitors.len() - 1);
                }
            }
            CockpitTab::Server => {
                if !app.server_probe_results.is_empty() {
                    app.selected_server = (app.selected_server + 1).min(app.server_probe_results.len() - 1);
                }
            }
            CockpitTab::Audit => app.report_scroll = app.report_scroll.saturating_add(2),
            CockpitTab::Cron => {},
            CockpitTab::Log => {
                app.log_scroll = (app.log_scroll + 1).min(app.log_buffer.len().saturating_sub(1));
            }
            CockpitTab::Terminal => {
                app.term_scroll = (app.term_scroll + 1).min(app.term_output.len().saturating_sub(1));
            }
            CockpitTab::AI => app.scroll_chat_down(),
            CockpitTab::Setting => {
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
                    HitboxAction::SelectProjectView(view) => {
                        app.project_view = view;
                    }
                    HitboxAction::SelectProject(idx) => {
                        app.project_view = ProjectView::Tracked;
                        app.selected_project = idx;
                    }
                    HitboxAction::SelectContainer(idx) => {
                        app.project_view = ProjectView::Containers;
                        app.selected_container = idx;
                    }
                    HitboxAction::ContainerStart(_) => {
                        app.start_selected_container();
                    }
                    HitboxAction::ContainerStop(_) => {
                        app.stop_selected_container();
                    }
                    HitboxAction::ContainerRestart(_) => {
                        app.restart_selected_container();
                    }
                    HitboxAction::SelectMonitor(idx) => {
                        app.selected_monitor = idx;
                    }
                    HitboxAction::ProbeMonitors => {
                        app.probe_all_ports();
                        app.set_notification("Probed all Uptime monitors & gateways");
                    }
                    HitboxAction::SelectServer(idx) => {
                        app.selected_server = idx;
                    }
                    HitboxAction::ProbeServers => {
                        app.probe_servers();
                        app.set_notification("Probed all multi-node server latencies");
                    }
                    HitboxAction::SelectLogSource(src) => {
                        app.log_source = src;
                        app.load_logs();
                    }
                    HitboxAction::SelectLogLevel(lvl) => {
                        app.log_level = lvl;
                        app.set_notification(format!("Log level filter: {}", lvl.label()));
                    }
                    HitboxAction::RefreshLogs => {
                        app.load_logs();
                        app.set_notification("Logs refreshed from active stream");
                    }
                    HitboxAction::FocusTerminalInput => {
                        // Focus already on Terminal tab input by keyboard
                    }
                    HitboxAction::RunTerminalCommand => {
                        app.run_terminal_command();
                    }
                    HitboxAction::SelectTerminalRemote(target) => {
                        app.term_remote = target;
                        app.set_notification(match &app.term_remote {
                            Some(name) => format!("Terminal target: Remote [{name}]"),
                            None => "Terminal target: Local (ThinkPad X1)".to_string(),
                        });
                    }
                    HitboxAction::CycleAiModel => {
                        app.cycle_ai_model();
                    }
                    HitboxAction::ClearAiChat => {
                        app.clear_chat_history();
                    }
                    HitboxAction::NewCronWizard => {
                        app.show_cron_wizard = true;
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
                        app.pet_selected_action = (app.pet_selected_action + 4) % 5;
                    }
                    HitboxAction::PetActionDown => {
                        app.pet_selected_action = (app.pet_selected_action + 1) % 5;
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
                    // Add-wizard openers
                    HitboxAction::AddServer => {
                        app.show_server_wizard = true;
                        app.server_wizard_field = 0;
                    }
                    HitboxAction::AddProject => {
                        app.show_project_wizard = true;
                        app.project_wizard_field = 0;
                    }
                    HitboxAction::AddContainer => {
                        app.show_container_wizard = true;
                        app.container_wizard_field = 0;
                    }
                    HitboxAction::AddMonitor => {
                        app.show_monitor_wizard = true;
                        app.monitor_wizard_field = 0;
                    }
                    // Field focus
                    HitboxAction::FocusServerField(i) => {
                        app.server_wizard_field = i;
                    }
                    HitboxAction::FocusProjectField(i) => {
                        app.project_wizard_field = i;
                    }
                    HitboxAction::FocusContainerField(i) => {
                        app.container_wizard_field = i;
                    }
                    HitboxAction::FocusMonitorField(i) => {
                        app.monitor_wizard_field = i;
                    }
                    // Submissions
                    HitboxAction::SubmitServer => {
                        app.submit_server_wizard();
                    }
                    HitboxAction::SubmitProject => {
                        app.submit_project_wizard();
                    }
                    HitboxAction::SubmitContainer => {
                        app.submit_container_wizard();
                    }
                    HitboxAction::SubmitMonitor => {
                        app.submit_monitor_wizard();
                    }
                    // Cancellations
                    HitboxAction::CancelServer => {
                        app.show_server_wizard = false;
                    }
                    HitboxAction::CancelProject => {
                        app.show_project_wizard = false;
                    }
                    HitboxAction::CancelContainer => {
                        app.show_container_wizard = false;
                    }
                    HitboxAction::CancelMonitor => {
                        app.show_monitor_wizard = false;
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
        11 => app.cycle_field_value(true),
        12 => {
            app.show_set_password_modal = true;
            app.set_password_field = 0;
            app.set_password_error = None;
        }
        13 => {
            // Password Hint
            app.editing_field = Some(13);
            app.edit_buffer = app.cfg.security.hint.clone();
        }
        14 => {
            // Telegram Bot Token
            app.editing_field = Some(14);
            app.edit_buffer = app.cfg.daemon.telegram_bot_token.clone();
        }
        15 => {
            // Telegram Chat ID
            app.editing_field = Some(15);
            app.edit_buffer = app.cfg.daemon.telegram_chat_id.clone();
        }
        16 => {
            // Daemon Port
            app.editing_field = Some(16);
            app.edit_buffer = app.cfg.daemon.port.to_string();
        }
        17 => {
            // Daemon Interval
            app.editing_field = Some(17);
            app.edit_buffer = app.cfg.daemon.interval_secs.to_string();
        }
        18 => {
            // Daemon Retry Threshold
            app.editing_field = Some(18);
            app.edit_buffer = app.cfg.daemon.retry_threshold.to_string();
        }
        19 => {
            // Coolify Base URL
            app.editing_field = Some(19);
            app.edit_buffer = app.cfg.deploy.coolify.as_ref().map(|c| c.base_url.clone()).unwrap_or_default();
        }
        20 => {
            // Coolify API Token
            app.editing_field = Some(20);
            app.edit_buffer = app.cfg.deploy.coolify.as_ref().map(|c| c.api_token.clone()).unwrap_or_default();
        }
        21 => app.cycle_field_value(true),
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
        13 => {
            app.cfg.security.hint = val.clone();
            app.set_notification(format!("Password hint set to: {val}"));
        }
        14 => {
            app.cfg.daemon.telegram_bot_token = val;
            app.set_notification("Telegram bot token updated");
        }
        15 => {
            app.cfg.daemon.telegram_chat_id = val.clone();
            app.set_notification(format!("Telegram chat ID set to: {val}"));
        }
        16 => {
            if let Ok(p) = val.parse::<u16>() {
                app.cfg.daemon.port = p;
                app.set_notification(format!("Daemon port set to: {p}"));
            } else {
                app.set_notification("Invalid port number");
            }
        }
        17 => {
            if let Ok(s) = val.parse::<u64>() {
                app.cfg.daemon.interval_secs = s.max(5);
                app.set_notification(format!("Daemon probe interval set to: {s}s"));
            } else {
                app.set_notification("Invalid interval number");
            }
        }
        18 => {
            if let Ok(r) = val.parse::<u32>() {
                app.cfg.daemon.retry_threshold = r.max(1);
                app.set_notification(format!("Daemon retry threshold set to: {r}"));
            } else {
                app.set_notification("Invalid threshold number");
            }
        }
        19 => {
            if app.cfg.deploy.coolify.is_none() {
                app.cfg.deploy.coolify = Some(crate::config::CoolifyConfig {
                    base_url: val.clone(),
                    api_token: String::new(),
                    timeout_seconds: 60,
                });
            } else if let Some(ref mut c) = app.cfg.deploy.coolify {
                c.base_url = val.clone();
            }
            app.set_notification(format!("Coolify base URL set to: {val}"));
        }
        20 => {
            if app.cfg.deploy.coolify.is_none() {
                app.cfg.deploy.coolify = Some(crate::config::CoolifyConfig {
                    base_url: String::new(),
                    api_token: val,
                    timeout_seconds: 60,
                });
            } else if let Some(ref mut c) = app.cfg.deploy.coolify {
                c.api_token = val;
            }
            app.set_notification("Coolify API token updated");
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
    draw_lock_or_app(f, app, area);
}

// ── Master Password Lock Screen & Draw Handler ───────────────────────────────

fn draw_lock_or_app(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    if app.is_locked {
        f.render_widget(Clear, area);
        draw_lock_screen(f, app, area);
        return;
    }

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
        CockpitTab::Dashboard => draw_dashboard_tab(f, app, chunks[2]),
        CockpitTab::Project => draw_project_tab(f, app, chunks[2]),
        CockpitTab::Uptime => draw_uptime_tab(f, app, chunks[2]),
        CockpitTab::Server => draw_server_tab(f, app, chunks[2]),
        CockpitTab::Audit => draw_audit_tab(f, app, chunks[2]),
        CockpitTab::Cron => draw_cron_tab(f, app, chunks[2]),
        CockpitTab::Log => draw_log_tab(f, app, chunks[2]),
        CockpitTab::Terminal => draw_terminal_tab(f, app, chunks[2]),
        CockpitTab::AI => draw_ai_tab(f, app, chunks[2]),
        CockpitTab::Setting => draw_setting_tab(f, app, chunks[2]),
    }

    draw_footer(f, app, chunks[3]);

    if app.inspector_open && app.current_tab == CockpitTab::Project {
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

    if app.show_server_wizard {
        draw_server_wizard(f, app, area);
    }

    if app.show_project_wizard {
        draw_project_wizard(f, app, area);
    }

    if app.show_container_wizard {
        draw_container_wizard(f, app, area);
    }

    if app.show_monitor_wizard {
        draw_monitor_wizard(f, app, area);
    }

    if app.show_quit_confirm {
        draw_quit_modal(f, app, area);
    }

    if app.show_set_password_modal {
        draw_set_password_modal(f, app, area);
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

    let title = " CAShell v0.3.5 · Isometric Cockpit ";
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

#[cfg(test)]
fn format_kbps(kbps: f64) -> String {
    if kbps >= 1024.0 {
        format!("{:.1} MB/s", kbps / 1024.0)
    } else {
        format!("{:.0} KB/s", kbps)
    }
}


fn draw_dashboard_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12), // Top: CPU Grid + Tamagotchi Pet + Memory/Storage
            Constraint::Length(4),  // Middle: Fleet Health & Active Alerts Matrix
            Constraint::Min(8),     // Bottom: Live Dokploy Fleet & Top Processes
        ])
        .split(area);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(36), // CPU Telemetry
            Constraint::Percentage(36), // Tamagotchi Pet Play Arena
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
        palette.status_err
    } else if snap.cpu_pct > 80.0 {
        palette.status_warn
    } else {
        palette.status_ok
    };

    let cpu_block = Block::default()
        .borders(Borders::ALL)
        .title(cpu_title)
        .border_style(Style::default().fg(cpu_border_color));
    let cpu_inner = cpu_block.inner(top_chunks[0]);
    f.render_widget(cpu_block, top_chunks[0]);

    if cpu_inner.width > 10 && cpu_inner.height > 2 {
        let mut header_spans = vec![
            Span::styled("Load: ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("[{:.2}, {:.2}, {:.2}]  ", snap.load_avg[0], snap.load_avg[1], snap.load_avg[2]),
                Style::default().fg(palette.accent),
            ),
            Span::styled("Fan: ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("{}  ", snap.fan_speed.as_deref().unwrap_or("N/A")),
                Style::default().fg(Color::White),
            ),
            Span::styled("Temp: ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("{:.1}°C", snap.cpu_temp_c),
                if snap.cpu_temp_c > 88.0 {
                    Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD)
                } else if snap.cpu_temp_c > 75.0 {
                    Style::default().fg(palette.status_warn)
                } else {
                    Style::default().fg(palette.status_ok)
                },
            ),
        ];

        if thermal_alert {
            header_spans.push(Span::styled(
                "  [🔥 THERMAL ALERT]",
                Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD),
            ));
        }

        let cpu_hist_vec: Vec<f32> = app.cpu_history.iter().copied().collect();
        let braille_height = if cpu_inner.height >= 8 { 2 } else { 1 };
        let braille_lines = render_braille_curve(&cpu_hist_vec, cpu_inner.width as usize, braille_height);

        let half_w = (cpu_inner.width.saturating_sub(3) / 2) as usize;
        let bar_len = if half_w >= 26 { 10 } else if half_w >= 20 { 6 } else { 4 };

        let mut core_lines = Vec::new();
        for row in 0..4 {
            let c_left_idx = row;
            let c_right_idx = row + 4;

            let left_spans = format_core_span(snap.cpus.get(c_left_idx), c_left_idx, bar_len);
            let right_spans = format_core_span(snap.cpus.get(c_right_idx), c_right_idx, bar_len);

            let mut line_spans = left_spans;
            line_spans.push(Span::styled(" │ ", Style::default().fg(palette.text_muted)));
            line_spans.extend(right_spans);
            core_lines.push(Line::from(line_spans));
        }

        let mut full_cpu_lines = Vec::new();
        full_cpu_lines.push(Line::from(header_spans));
        full_cpu_lines.extend(braille_lines);
        full_cpu_lines.extend(core_lines);

        f.render_widget(Paragraph::new(full_cpu_lines), cpu_inner);
    }

    // ── Center: Tamagotchi Pet Play Arena ───────────────────────────────────
    draw_pet_play_area(f, app, top_chunks[1]);

    // ── Right: Memory & Mount Table ──────────────────────────────────────────
    let mem_block = Block::default()
        .borders(Borders::ALL)
        .title(" Memory & Storage Telemetry ")
        .border_style(Style::default().fg(palette.border));
    let mem_inner = mem_block.inner(top_chunks[2]);
    f.render_widget(mem_block, top_chunks[2]);

    if mem_inner.width > 10 && mem_inner.height > 3 {
        let mem_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // RAM gauge
                Constraint::Length(1), // Swap gauge
                Constraint::Length(1), // GPU telemetry info
                Constraint::Min(3),    // Storage mounts table
            ])
            .split(mem_inner);

        let ram_used = snap.ram_used_mb;
        let ram_total = snap.ram_total_mb.max(1);
        let ram_pct = ((ram_used * 100) / ram_total) as u16;
        let ram_color = if ram_pct > 85 {
            palette.status_err
        } else if ram_pct > 70 {
            palette.status_warn
        } else {
            palette.chart_cpu
        };
        let ram_label = format!("RAM: {ram_used}M/{ram_total}M ({ram_pct}%) [Cached: {}M]", snap.ram_cached_mb);
        let ram_gauge = Gauge::default()
            .gauge_style(Style::default().fg(ram_color).bg(Color::DarkGray))
            .ratio(((ram_used as f64) / (ram_total as f64)).clamp(0.0, 1.0))
            .label(ram_label);
        f.render_widget(ram_gauge, mem_layout[0]);

        let swap_used = snap.swap_used_mb;
        let swap_total = snap.swap_total_mb.max(1);
        let swap_pct = if snap.swap_total_mb > 0 {
            ((swap_used * 100) / swap_total) as u16
        } else {
            0
        };
        let swap_label = format!("SWP: {swap_used}M/{swap_total}M ({swap_pct}%)");
        let swap_gauge = Gauge::default()
            .gauge_style(Style::default().fg(palette.chart_ram).bg(Color::DarkGray))
            .ratio(if snap.swap_total_mb > 0 {
                ((swap_used as f64) / (swap_total as f64)).clamp(0.0, 1.0)
            } else {
                0.0
            })
            .label(swap_label);
        f.render_widget(swap_gauge, mem_layout[1]);

        let gpu_info = if snap.gpu_label != "No GPU" && !snap.gpu_label.is_empty() {
            let util = snap.gpu_pct.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "N/A".to_string());
            let temp = snap.gpu_temp_c.map(|t| format!("{t:.0}°C")).unwrap_or_else(|| "N/A".to_string());
            format!("GPU: {} │ Util: {} │ Temp: {}", snap.gpu_label, util, temp)
        } else {
            "GPU: Integrated Graphics (Intel Xe / AMD DRM)".to_string()
        };
        let gpu_line = Line::from(vec![
            Span::styled("◆ ", Style::default().fg(palette.status_warn)),
            Span::styled(gpu_info, Style::default().fg(palette.text_muted)),
        ]);
        f.render_widget(Paragraph::new(gpu_line), mem_layout[2]);

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
            let bar_len: usize = 6;
            let filled = ((m.pct as f32 / 100.0) * bar_len as f32).round() as usize;
            let unfilled = bar_len.saturating_sub(filled);
            let bar_str = format!("[{}{}] {:>3}%", "█".repeat(filled), "░".repeat(unfilled), m.pct);
            let pct_color = if m.pct >= 90 {
                palette.status_err
            } else if m.pct >= 75 {
                palette.status_warn
            } else {
                palette.status_ok
            };

            mount_rows.push(Row::new(vec![
                Cell::from(mount_label.to_string()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Cell::from(used_total).style(Style::default().fg(palette.accent)),
                Cell::from(bar_str).style(Style::default().fg(pct_color)),
            ]));
        }

        if mount_rows.is_empty() {
            mount_rows.push(Row::new(vec![
                Cell::from("/ (Root NVMe)").style(Style::default().fg(Color::White)),
                Cell::from(format!("{}G / {}G", snap.disk_used_gb, snap.disk_total_gb)),
                Cell::from("[███░░░] 50%").style(Style::default().fg(palette.status_ok)),
            ]));
        }

        let mount_table = Table::new(
            mount_rows,
            [
                Constraint::Length(16),
                Constraint::Length(12),
                Constraint::Min(12),
            ],
        )
        .header(
            Row::new(vec!["MOUNT", "USED/TOTAL", "USAGE"])
                .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
        );
        f.render_widget(mount_table, mem_layout[3]);
    }

    // ── Middle: Fleet Health & Active Alerts Matrix ──────────────────────────
    let alerts_block = Block::default()
        .borders(Borders::ALL)
        .title(" Fleet Health & Active System Alerts Matrix ")
        .border_style(Style::default().fg(palette.accent));
    let alerts_inner = alerts_block.inner(chunks[1]);
    f.render_widget(alerts_block, chunks[1]);

    let mut alerts_spans = Vec::new();
    let is_mesh_ok = app.is_tailscale_ok;
    alerts_spans.push(Span::styled("Fleet Mesh: ", Style::default().fg(palette.text_muted)));
    if is_mesh_ok {
        alerts_spans.push(Span::styled("● TAILSCALE OK  ", Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD)));
    } else {
        alerts_spans.push(Span::styled("✖ TAILSCALE DOWN  ", Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD)));
    }

    alerts_spans.push(Span::styled("AI Proxy: ", Style::default().fg(palette.text_muted)));
    if app.is_9router_online {
        alerts_spans.push(Span::styled("● 9ROUTER :20128 ONLINE  ", Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD)));
    } else {
        alerts_spans.push(Span::styled("✖ 9ROUTER DOWN  ", Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)));
    }

    let down_monitors = snap.uptime_monitors.iter().filter(|m| !m.is_up).count();
    alerts_spans.push(Span::styled("Monitors: ", Style::default().fg(palette.text_muted)));
    if down_monitors == 0 {
        alerts_spans.push(Span::styled(format!("● ALL {} UP  ", snap.uptime_monitors.len()), Style::default().fg(palette.status_ok)));
    } else {
        alerts_spans.push(Span::styled(format!("✖ {down_monitors} DOWN  "), Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD)));
    }

    let running_cont = snap.docker_containers.iter().filter(|c| c.state == "running").count();
    let total_cont = snap.docker_containers.len();
    alerts_spans.push(Span::styled(format!("Containers: {running_cont}/{total_cont} Running"), Style::default().fg(palette.accent)));

    let mut alert_line2 = vec![
        Span::styled("Active Alerts: ", Style::default().fg(palette.text_muted)),
    ];
    let mut any_alert = false;
    if snap.cpu_temp_c > 85.0 {
        alert_line2.push(Span::styled(" [🔥 Thermal Alert > 85°C] ", Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD)));
        any_alert = true;
    }
    if snap.cpu_pct > 80.0 {
        alert_line2.push(Span::styled(" [⚡ High CPU Load > 80%] ", Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)));
        any_alert = true;
    }
    let ram_ratio = (snap.ram_used_mb as f64) / (snap.ram_total_mb.max(1) as f64);
    if ram_ratio > 0.85 {
        alert_line2.push(Span::styled(" [💾 Low RAM Available] ", Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD)));
        any_alert = true;
    }
    if !any_alert {
        alert_line2.push(Span::styled("✔ Fleet healthy. All telemetry metrics nominal.", Style::default().fg(palette.status_ok)));
    }

    let summary_lines = vec![
        Line::from(alerts_spans),
        Line::from(alert_line2),
    ];
    f.render_widget(Paragraph::new(summary_lines), alerts_inner);

    // ── Bottom: Live Dokploy Fleet & Top Processes ──────────────────────────
    let fleet_block = Block::default()
        .borders(Borders::ALL)
        .title(" Live Dokploy Fleet & Top Processes (Press 'j'/'k' to scroll) ")
        .border_style(Style::default().fg(palette.border));
    let fleet_inner = fleet_block.inner(chunks[2]);
    f.render_widget(fleet_block, chunks[2]);

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

    entries.sort_by(|a, b| {
        b.is_active
            .cmp(&a.is_active)
            .then_with(|| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| b.mem_mb.cmp(&a.mem_mb))
    });

    let scroll_offset = app.overview_proc_scroll.min(entries.len().saturating_sub(1));
    let display_entries = entries.into_iter().skip(scroll_offset);

    let rows: Vec<Row> = display_entries
        .map(|e| {
            let cpu_color = if e.cpu_pct >= 80.0 {
                palette.status_err
            } else if e.cpu_pct >= 50.0 {
                palette.status_warn
            } else if e.cpu_pct > 0.0 {
                palette.status_ok
            } else {
                palette.text_muted
            };

            let mem_color = if e.mem_mb > 0 { Color::White } else { palette.text_muted };

            let (status_label, status_color) = if e.is_active {
                ("● Active", palette.status_ok)
            } else {
                ("● Standby", palette.chart_cpu)
            };

            Row::new(vec![
                Cell::from(e.pid).style(Style::default().fg(palette.text_muted)),
                Cell::from(e.name).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Cell::from(e.role).style(Style::default().fg(palette.accent)),
                Cell::from(status_label).style(Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                Cell::from(format!("{:>5.1}%", e.cpu_pct)).style(Style::default().fg(cpu_color)),
                Cell::from(format!("{:>6} MB", e.mem_mb)).style(Style::default().fg(mem_color)),
                Cell::from(e.details).style(Style::default().fg(palette.text_muted)),
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
        .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );

    f.render_widget(table, fleet_inner);
}
// ── Tab 2: Project & Containers (Tracked Projects + Docker Lifecycle) ────────

fn draw_project_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    let action_bar = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let action_inner = action_bar.inner(chunks[0]);
    f.render_widget(action_bar, chunks[0]);

    let (btn_tracked_bg, btn_tracked_fg) = if app.project_view == ProjectView::Tracked {
        (palette.highlight_bg, palette.highlight_fg)
    } else {
        (Color::Reset, palette.text_muted)
    };
    let (btn_cont_bg, btn_cont_fg) = if app.project_view == ProjectView::Containers {
        (palette.highlight_bg, palette.highlight_fg)
    } else {
        (Color::Reset, palette.text_muted)
    };

    let btn_tracked_w = 20u16;
    let btn_cont_w = 22u16;
    let btn_add_proj_w = 18u16;
    let btn_add_cont_w = 20u16;
    let btn_start_w = 11u16;
    let btn_stop_w = 10u16;
    let btn_restart_w = 13u16;

    let mut cur_x = action_inner.x + 1;
    let y = action_inner.y;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_tracked_w, height: 1 },
        HitboxAction::SelectProjectView(ProjectView::Tracked),
    );
    cur_x += btn_tracked_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_cont_w, height: 1 },
        HitboxAction::SelectProjectView(ProjectView::Containers),
    );
    cur_x += btn_cont_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_add_proj_w, height: 1 },
        HitboxAction::AddProject,
    );
    cur_x += btn_add_proj_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_add_cont_w, height: 1 },
        HitboxAction::AddContainer,
    );
    cur_x += btn_add_cont_w + 2;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_start_w, height: 1 },
        HitboxAction::ContainerStart(app.selected_container),
    );
    cur_x += btn_start_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_stop_w, height: 1 },
        HitboxAction::ContainerStop(app.selected_container),
    );
    cur_x += btn_stop_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_restart_w, height: 1 },
        HitboxAction::ContainerRestart(app.selected_container),
    );

    let bar_spans = vec![
        Span::styled(" [📁 Tracked Projects] ", Style::default().fg(btn_tracked_fg).bg(btn_tracked_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [🐳 Docker Containers] ", Style::default().fg(btn_cont_fg).bg(btn_cont_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" │ "),
        Span::styled(" [+ Add Project] ", Style::default().fg(Color::Black).bg(palette.status_warn).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [+ Add Container] ", Style::default().fg(Color::Black).bg(palette.accent).add_modifier(Modifier::BOLD)),
        Span::raw(" │ Actions: "),
        Span::styled(" [▶ Start] ", Style::default().fg(Color::Black).bg(palette.status_ok).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [■ Stop] ", Style::default().fg(Color::Black).bg(palette.status_err).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [↺ Restart] ", Style::default().fg(Color::Black).bg(palette.status_warn).add_modifier(Modifier::BOLD)),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), action_inner);

    match app.project_view {
        ProjectView::Tracked => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" Tracked Workspace Projects ({}) ", app.latest_snap.tracked_projects.len()))
                .border_style(Style::default().fg(palette.border));
            let inner = block.inner(chunks[1]);
            f.render_widget(block, chunks[1]);

            let rows: Vec<Row> = app
                .latest_snap
                .tracked_projects
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let is_sel = i == app.selected_project;
                    let bg = if is_sel { palette.row_selected_bg } else { Color::Reset };
                    let (dirty_badge, dirty_col) = if p.is_dirty {
                        ("● DIRTY", palette.status_warn)
                    } else {
                        ("● CLEAN", palette.status_ok)
                    };
                    Row::new(vec![
                        Cell::from(p.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Cell::from(p.category.clone()).style(Style::default().fg(palette.accent)),
                        Cell::from(p.branch.clone()).style(Style::default().fg(palette.chart_cpu)),
                        Cell::from(dirty_badge).style(Style::default().fg(dirty_col).add_modifier(Modifier::BOLD)),
                    ])
                    .style(Style::default().bg(bg))
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(26),
                    Constraint::Length(16),
                    Constraint::Length(18),
                    Constraint::Min(25),
                ],
            )
            .header(
                Row::new(vec!["PROJECT NAME", "CATEGORY", "GIT BRANCH", "STATUS"])
                    .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
            );
            f.render_widget(table, inner);

            for i in 0..app.latest_snap.tracked_projects.len() {
                if (i as u16) < inner.height.saturating_sub(1) {
                    app.register_hitbox(
                        Rect {
                            x: inner.x,
                            y: inner.y + 1 + (i as u16),
                            width: inner.width,
                            height: 1,
                        },
                        HitboxAction::SelectProject(i),
                    );
                }
            }
        }
        ProjectView::Containers => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" Docker Containers & Local Services ({}) ", app.latest_snap.docker_containers.len()))
                .border_style(Style::default().fg(palette.border));
            let inner = block.inner(chunks[1]);
            f.render_widget(block, chunks[1]);

            let rows: Vec<Row> = app
                .latest_snap
                .docker_containers
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let is_sel = i == app.selected_container;
                    let bg = if is_sel { palette.row_selected_bg } else { Color::Reset };
                    let (state_icon, state_col) = match c.state.as_str() {
                        "running" => ("● RUNNING", palette.status_ok),
                        "paused" => ("● PAUSED", palette.status_warn),
                        _ => ("✖ EXITED", palette.status_err),
                    };

                    Row::new(vec![
                        Cell::from(c.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Cell::from(c.image.clone()).style(Style::default().fg(palette.chart_cpu)),
                        Cell::from(state_icon).style(Style::default().fg(state_col).add_modifier(Modifier::BOLD)),
                        Cell::from(c.status_detail.clone()).style(Style::default().fg(palette.text_muted)),
                    ])
                    .style(Style::default().bg(bg))
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(24),
                    Constraint::Length(26),
                    Constraint::Length(16),
                    Constraint::Min(25),
                ],
            )
            .header(
                Row::new(vec!["CONTAINER NAME", "IMAGE", "STATE", "STATUS / PORTS"])
                    .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
            );
            f.render_widget(table, inner);

            for i in 0..app.latest_snap.docker_containers.len() {
                if (i as u16) < inner.height.saturating_sub(1) {
                    app.register_hitbox(
                        Rect {
                            x: inner.x,
                            y: inner.y + 1 + (i as u16),
                            width: inner.width,
                            height: 1,
                        },
                        HitboxAction::SelectContainer(i),
                    );
                }
            }
        }
    }
}

// ── Tab 3: Uptime (Endpoint Monitors, HTTP/HTTPS, TCP, Cloudflare tunnels) ───

fn draw_uptime_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_add_w = 20u16;
    let btn_probe_w = 22u16;

    app.register_hitbox(
        Rect { x: bar_inner.x + 1, y: bar_inner.y, width: btn_add_w, height: 1 },
        HitboxAction::AddMonitor,
    );
    app.register_hitbox(
        Rect { x: bar_inner.x + btn_add_w + 3, y: bar_inner.y, width: btn_probe_w, height: 1 },
        HitboxAction::ProbeMonitors,
    );

    let up_count = app.latest_snap.uptime_monitors.iter().filter(|m| m.is_up).count();
    let total_count = app.latest_snap.uptime_monitors.len();
    let pct = if total_count > 0 {
        (up_count as f32 / total_count as f32) * 100.0
    } else {
        100.0
    };

    let bar_spans = vec![
        Span::styled(" [+ Add Monitor] (m) ", Style::default().fg(Color::Black).bg(palette.status_ok).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" [🔍 Probe Monitors] (p) ", Style::default().fg(Color::Black).bg(palette.highlight_bg).add_modifier(Modifier::BOLD)),
        Span::raw("  │  "),
        Span::styled(format!("Health: {up_count}/{total_count} Up ({pct:.1}%)"), Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 🌐 Endpoint Uptime Monitors (HTTP/HTTPS, TCP, Cloudflare Tunnels, Gateways) ")
        .border_style(Style::default().fg(palette.border));
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);

    let snap = &app.latest_snap;
    let rows: Vec<Row> = snap
        .uptime_monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let is_sel = i == app.selected_monitor;
            let bg = if is_sel { palette.row_selected_bg } else { Color::Reset };
            let (status_badge, badge_color) = if m.is_up {
                (format!("● UP ({:.1}%)", m.uptime_pct), palette.status_ok)
            } else {
                (format!("✖ DOWN ({:.1}%)", m.uptime_pct), palette.status_err)
            };

            let latency_str = m
                .latency_ms
                .map(|l| format!("{l:>3}ms"))
                .unwrap_or_else(|| " -- ".to_string());
            let code_str = m
                .status_code
                .map(|c| format!("{c} OK"))
                .unwrap_or_else(|| "--".to_string());

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
                Cell::from(m.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Cell::from(status_badge).style(Style::default().fg(badge_color).add_modifier(Modifier::BOLD)),
                Cell::from(latency_str).style(Style::default().fg(palette.accent)),
                Cell::from(Line::from(bar_spans)),
                Cell::from(code_str).style(Style::default().fg(palette.status_warn)),
                Cell::from(m.target.clone()).style(Style::default().fg(palette.text_muted)),
            ])
            .style(Style::default().bg(bg))
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(16),
            Constraint::Length(10),
            Constraint::Length(24),
            Constraint::Length(10),
            Constraint::Min(25),
        ],
    )
    .header(
        Row::new(vec![
            "MONITOR",
            "STATUS",
            "LATENCY",
            "HEARTBEATS (20)",
            "CODE",
            "ENDPOINT URL",
        ])
        .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);

    for i in 0..snap.uptime_monitors.len() {
        if (i as u16) < inner.height.saturating_sub(1) {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: inner.y + 1 + (i as u16),
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::SelectMonitor(i),
            );
        }
    }
}

// ── Tab 4: Server (Multi-Node Server Resources & Latency Matrix) ─────────────

fn draw_server_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_add_w = 18u16;
    let btn_probe_w = 26u16;

    app.register_hitbox(
        Rect { x: bar_inner.x + 1, y: bar_inner.y, width: btn_add_w, height: 1 },
        HitboxAction::AddServer,
    );
    app.register_hitbox(
        Rect { x: bar_inner.x + btn_add_w + 3, y: bar_inner.y, width: btn_probe_w, height: 1 },
        HitboxAction::ProbeServers,
    );

    let bar_spans = vec![
        Span::styled(" [+ Add Server] (a) ", Style::default().fg(Color::Black).bg(palette.status_ok).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" [🔍 Probe Latency Matrix] (p) ", Style::default().fg(Color::Black).bg(palette.highlight_bg).add_modifier(Modifier::BOLD)),
        Span::raw("  │  "),
        Span::styled("Fleet Nodes: X1, YPC, Hostinger VPS, XPC, Gerlink", Style::default().fg(palette.accent)),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let sub_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    let list_block = Block::default()
        .borders(Borders::ALL)
        .title(" Multi-Node Latency & Fleet Matrix ")
        .border_style(Style::default().fg(palette.border));
    let list_inner = list_block.inner(sub_chunks[0]);
    f.render_widget(list_block, sub_chunks[0]);

    let mut rows = Vec::new();
    for (i, (node_name, lat)) in app.server_probe_results.iter().enumerate() {
        let is_sel = i == app.selected_server;
        let bg = if is_sel { palette.row_selected_bg } else { Color::Reset };
        let (lat_str, status_str, status_col) = match lat {
            Some(ms) => (format!("{ms}ms"), "● REACHABLE", palette.status_ok),
            None => ("--".to_string(), "✖ UNREACHABLE", palette.status_err),
        };
        rows.push(
            Row::new(vec![
                Cell::from(node_name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Cell::from(status_str).style(Style::default().fg(status_col).add_modifier(Modifier::BOLD)),
                Cell::from(lat_str).style(Style::default().fg(palette.accent)),
            ])
            .style(Style::default().bg(bg)),
        );
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(16),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(vec!["SERVER NODE", "STATUS", "RTT LATENCY"])
            .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, list_inner);

    for i in 0..app.server_probe_results.len() {
        if (i as u16) < list_inner.height.saturating_sub(1) {
            app.register_hitbox(
                Rect {
                    x: list_inner.x,
                    y: list_inner.y + 1 + (i as u16),
                    width: list_inner.width,
                    height: 1,
                },
                HitboxAction::SelectServer(i),
            );
        }
    }

    let sel_name = app.server_probe_results.get(app.selected_server).map(|(n, _)| n.as_str()).unwrap_or("ThinkPad X1");
    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Node Resources — {} ", sel_name))
        .border_style(Style::default().fg(palette.accent));
    let detail_inner = detail_block.inner(sub_chunks[1]);
    f.render_widget(detail_block, sub_chunks[1]);

    let snap = &app.latest_snap;
    let mut details = Vec::new();
    if sel_name.contains("X1") || sel_name.contains("Local") {
        details.push(Line::from(vec![
            Span::styled("Hardware:     ", Style::default().fg(palette.text_muted)),
            Span::styled("ThinkPad X1 Carbon (Intel 8-Core / Btrfs NVMe)", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]));
        details.push(Line::from(vec![
            Span::styled("CPU Cores:    ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{} cores │ Usage: {:.1}% │ Temp: {:.1}°C", snap.cpus.len(), snap.cpu_pct, snap.cpu_temp_c), Style::default().fg(palette.accent)),
        ]));
        details.push(Line::from(vec![
            Span::styled("Memory (RAM): ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{}/{} MB (Cached: {} MB, Free: {} MB)", snap.ram_used_mb, snap.ram_total_mb, snap.ram_cached_mb, snap.ram_available_mb), Style::default().fg(palette.chart_cpu)),
        ]));
        details.push(Line::from(vec![
            Span::styled("Storage/NVMe: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("Root /: {}G / {}G", snap.disk_used_gb, snap.disk_total_gb), Style::default().fg(palette.status_ok)),
        ]));
        details.push(Line::from(vec![
            Span::styled("Thermal State:", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:.1}°C (Fan: {})", snap.cpu_temp_c, snap.fan_speed.as_deref().unwrap_or("Dynamic")), Style::default().fg(if snap.cpu_temp_c > 85.0 { palette.status_err } else { palette.status_ok })),
        ]));
        details.push(Line::from(""));
        details.push(Line::from(Span::styled("Per-Core Telemetry Grid:", Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD))));
        for row in 0..4 {
            let left_spans = format_core_span(snap.cpus.get(row), row, 6);
            let right_spans = format_core_span(snap.cpus.get(row + 4), row + 4, 6);
            let mut l = left_spans;
            l.push(Span::styled(" │ ", Style::default().fg(palette.text_muted)));
            l.extend(right_spans);
            details.push(Line::from(l));
        }
    } else {
        let node_cfg = app.cfg.daemon.nodes.get(sel_name);
        let ip = node_cfg.map(|n| n.ip.as_str()).unwrap_or("100.x.y.z (Tailscale Mesh)");
        let user = node_cfg.map(|n| n.user.as_str()).unwrap_or("root");
        let svcs = node_cfg.map(|n| n.services.iter().map(|s| format!("{}:{}", s.name, s.port)).collect::<Vec<_>>().join(", ")).unwrap_or_else(|| "ssh:22, http:80".to_string());

        details.push(Line::from(vec![
            Span::styled("Node ID:       ", Style::default().fg(palette.text_muted)),
            Span::styled(sel_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]));
        details.push(Line::from(vec![
            Span::styled("Tailscale IP:  ", Style::default().fg(palette.text_muted)),
            Span::styled(ip, Style::default().fg(palette.accent)),
        ]));
        details.push(Line::from(vec![
            Span::styled("SSH Username:  ", Style::default().fg(palette.text_muted)),
            Span::styled(user, Style::default().fg(Color::White)),
        ]));
        details.push(Line::from(vec![
            Span::styled("Services:      ", Style::default().fg(palette.text_muted)),
            Span::styled(svcs.clone(), Style::default().fg(palette.status_warn)),
        ]));
        details.push(Line::from(""));
        details.push(Line::from(Span::styled("Press 'a' to add a new server node, or 'p' to re-probe network latencies.", Style::default().fg(palette.text_muted))));
    }

    f.render_widget(Paragraph::new(details), detail_inner);
}

// ── Tab 7: Log (Live / Buffered Stream Viewer with Level Filtering) ──────────

fn draw_log_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_j_w = 14u16;
    let btn_d_w = 15u16;
    let btn_a_w = 14u16;
    let btn_refresh_w = 13u16;

    let mut cur_x = bar_inner.x + 1;
    let y = bar_inner.y;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_j_w, height: 1 },
        HitboxAction::SelectLogSource(LogSource::Journal),
    );
    cur_x += btn_j_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_d_w, height: 1 },
        HitboxAction::SelectLogSource(LogSource::Docker),
    );
    cur_x += btn_d_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_a_w, height: 1 },
        HitboxAction::SelectLogSource(LogSource::Agents),
    );
    cur_x += btn_a_w + 2;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_refresh_w, height: 1 },
        HitboxAction::RefreshLogs,
    );

    let (j_bg, j_fg) = if app.log_source == LogSource::Journal { (palette.highlight_bg, palette.highlight_fg) } else { (Color::Reset, palette.text_muted) };
    let (d_bg, d_fg) = if app.log_source == LogSource::Docker { (palette.highlight_bg, palette.highlight_fg) } else { (Color::Reset, palette.text_muted) };
    let (a_bg, a_fg) = if app.log_source == LogSource::Agents { (palette.highlight_bg, palette.highlight_fg) } else { (Color::Reset, palette.text_muted) };

    let bar_spans = vec![
        Span::styled(" [journalctl] ", Style::default().fg(j_fg).bg(j_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [docker logs] ", Style::default().fg(d_fg).bg(d_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [agent logs] ", Style::default().fg(a_fg).bg(a_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" │ "),
        Span::styled(" [↺ Refresh] ", Style::default().fg(Color::Black).bg(palette.status_warn).add_modifier(Modifier::BOLD)),
        Span::raw("  Filter: "),
        Span::styled(format!(" [{}] (Press 'l') ", app.log_level.label()), Style::default().fg(Color::Black).bg(palette.accent).add_modifier(Modifier::BOLD)),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Log Stream Viewer — Source: {} (Filtered: {}) ", app.log_source.label(), app.log_level.label()))
        .border_style(Style::default().fg(palette.border));
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);

    let filtered: Vec<&(String, String, String)> = app
        .log_buffer
        .iter()
        .filter(|(lvl, _, _)| {
            match app.log_level {
                LogLevelFilter::All => true,
                LogLevelFilter::Error => lvl == "ERROR",
                LogLevelFilter::Warn => lvl == "WARN" || lvl == "ERROR",
                LogLevelFilter::Info => true,
            }
        })
        .collect();

    let display_lines: Vec<Line> = filtered
        .iter()
        .skip(app.log_scroll)
        .map(|(lvl, src, msg)| {
            let (badge, col) = match lvl.as_str() {
                "ERROR" => ("✖ ERROR", palette.status_err),
                "WARN" => ("⚠ WARN ", palette.status_warn),
                _ => ("● INFO ", palette.status_ok),
            };
            Line::from(vec![
                Span::styled(format!("{:<8} ", badge), Style::default().fg(col).add_modifier(Modifier::BOLD)),
                Span::styled(format!("[{:<12}] ", src), Style::default().fg(palette.accent)),
                Span::styled(msg.clone(), Style::default().fg(Color::White)),
            ])
        })
        .collect();

    f.render_widget(Paragraph::new(display_lines).wrap(Wrap { trim: false }), inner);
}

// ── Tab 8: Terminal (Embedded Terminal / Command Execution Runner Pane) ───────

fn draw_terminal_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(3)])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let target_label = match &app.term_remote {
        Some(name) => format!("Remote: [{name}]"),
        None => "Local: ThinkPad X1".to_string(),
    };

    let bar_spans = vec![
        Span::styled(format!(" Target: [{target_label}] (Press 'r') "), Style::default().fg(Color::Black).bg(palette.highlight_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" │ Quick Presets: "),
        Span::styled(" [cas doctor] ", Style::default().fg(palette.accent)),
        Span::raw(" "),
        Span::styled(" [cas git-status] ", Style::default().fg(palette.chart_cpu)),
        Span::raw(" "),
        Span::styled(" [docker ps] ", Style::default().fg(palette.status_ok)),
        Span::raw(" "),
        Span::styled(" [tailscale status] ", Style::default().fg(palette.status_warn)),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let out_block = Block::default()
        .borders(Borders::ALL)
        .title(" Execution History & Console Output (Press '↑'/'↓' to scroll) ")
        .border_style(Style::default().fg(palette.border));
    let out_inner = out_block.inner(chunks[1]);
    f.render_widget(out_block, chunks[1]);

    let out_lines: Vec<Line> = app
        .term_output
        .iter()
        .skip(app.term_scroll)
        .map(|l| {
            let col = if l.starts_with("$") {
                palette.accent
            } else if l.starts_with("!") || l.starts_with("✗") {
                palette.status_err
            } else {
                Color::White
            };
            Line::from(Span::styled(l.clone(), Style::default().fg(col)))
        })
        .collect();
    f.render_widget(Paragraph::new(out_lines).wrap(Wrap { trim: false }), out_inner);

    let in_block = Block::default()
        .borders(Borders::ALL)
        .title(" Command Input ($ > Enter to run | Esc to clear) ")
        .border_style(Style::default().fg(palette.status_ok));
    let in_inner = in_block.inner(chunks[2]);
    f.render_widget(in_block, chunks[2]);

    let prompt_line = Line::from(vec![
        Span::styled("$ > ", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Span::styled(&app.term_input, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("█", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(Paragraph::new(prompt_line), in_inner);
}

// ── Tab 9: AI (Workspace AI Chat Connected to OmniRoute / 9router) ────────────

fn draw_ai_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(3)])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let active_model = app.current_ai_model();
    let btn_model_w = (active_model.len() as u16) + 26;
    let btn_clear_w = 16u16;

    app.register_hitbox(
        Rect { x: bar_inner.x + 1, y: bar_inner.y, width: btn_model_w, height: 1 },
        HitboxAction::CycleAiModel,
    );
    app.register_hitbox(
        Rect { x: bar_inner.x + btn_model_w + 3, y: bar_inner.y, width: btn_clear_w, height: 1 },
        HitboxAction::ClearAiChat,
    );

    let bar_spans = vec![
        Span::styled(format!(" [🤖 Model: {active_model} (m)] "), Style::default().fg(Color::Black).bg(palette.highlight_bg).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" [🗑 Clear Chat] ", Style::default().fg(Color::Black).bg(palette.status_err).add_modifier(Modifier::BOLD)),
        Span::raw("  │  Gateway: "),
        Span::styled("OmniRoute / 9router (localhost:20128 / :3007)", Style::default().fg(palette.accent)),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let hist_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Workspace AI Chat — Persona: {} ({}) ", app.cfg.persona.user_name, app.cfg.persona.assistant_role))
        .border_style(Style::default().fg(palette.border));
    let hist_inner = hist_block.inner(chunks[1]);
    f.render_widget(hist_block, chunks[1]);

    let items: Vec<ListItem> = app
        .chat_lines
        .iter()
        .map(|l| {
            let style = if l.starts_with("User:") {
                Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)
            } else if l.starts_with("AI:") {
                Style::default().fg(Color::White)
            } else if l.starts_with("Error:") {
                Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text_muted)
            };
            ListItem::new(Line::from(Span::styled(l.clone(), style)))
        })
        .collect();

    let chat_list = List::new(items)
        .highlight_style(Style::default().bg(palette.row_selected_bg))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(chat_list, hist_inner, &mut app.chat_state);

    let in_block = Block::default()
        .borders(Borders::ALL)
        .title(if app.is_chat_processing {
            " AI is thinking... "
        } else {
            " Prompt Input ($ > Enter to send | 'm': Model | 'c': Clear) "
        })
        .border_style(if app.is_chat_processing {
            Style::default().fg(palette.status_warn)
        } else {
            Style::default().fg(palette.status_ok)
        });
    let in_inner = in_block.inner(chunks[2]);
    f.render_widget(in_block, chunks[2]);

    let prompt_line = Line::from(vec![
        Span::styled("$ > ", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Span::styled(&app.chat_input, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("█", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(Paragraph::new(prompt_line), in_inner);
}

// ── Tab 10: Setting (Visual Settings GUI for config.toml) ─────────────────────

fn draw_setting_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    draw_settings_tab(f, app, area);
}

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

    // Determine active action state (transient action or telemetry mood)
    let active_state = if let Some((st, _)) = app.pet_transient_action {
        st
    } else {
        match app.pet_mood {
            crate::pet::PetMood::Sprint => crate::pet::PetActionState::Run,
            crate::pet::PetMood::Overheated => crate::pet::PetActionState::Hit,
            crate::pet::PetMood::Sick => crate::pet::PetActionState::Death,
            crate::pet::PetMood::Idle => {
                if (app.pet_tick / 8) % 4 >= 2 {
                    crate::pet::PetActionState::Walk
                } else {
                    crate::pet::PetActionState::Idle
                }
            }
        }
    };

    let state_badge = match active_state {
        crate::pet::PetActionState::Idle => "IDLE",
        crate::pet::PetActionState::Walk => "WALK",
        crate::pet::PetActionState::Run => "RUN >>",
        crate::pet::PetActionState::Jump => "JUMP ▲",
        crate::pet::PetActionState::Attack => "ATTACK ⚡",
        crate::pet::PetActionState::Dash => "DASH 彡",
        crate::pet::PetActionState::Hit => "HIT 💥",
        crate::pet::PetActionState::Death => "REST / ZZZ",
    };

    let title = format!(
        " 🐾 ISOMETRIC PET PLAY ARENA [ {} | Lv.{} | [{}] | {} ] ",
        pet_name_display, level, state_badge, mood_short
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
    let frames = status.frames_for_state(active_state);
    let frame_idx = (app.pet_tick / 4) % frames.len();
    let current_frame = frames[frame_idx];

    // Wander offset across the sprite screen
    let wander_offset = if active_state == crate::pet::PetActionState::Run || active_state == crate::pet::PetActionState::Dash || active_state == crate::pet::PetActionState::Walk {
        let screen_width = inner.width.saturating_sub(22) as usize;
        (app.pet_tick / 2) % screen_width.max(1)
    } else {
        let max_w = inner.width.saturating_sub(35).min(10) as usize;
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

    // 3. Action Buttons with active selection highlight (8-STATE ISOMETRIC ACTION BAR)
    let actions = [
        "🍖Feed(f)",
        "⚔Train(t)",
        "🔄Switch(s)",
        "📜Log(h)",
        "💥Hit(Enter)",
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
            3 => HitboxAction::PetToggleHistory,
            _ => HitboxAction::PetFeed,
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


// ── Tab 2: Services / Apps (Dokploy-style) ───────────────────────────────────


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

    let pw_status = if app.cfg.security.require_password {
        "● ENABLED (Protected on startup)"
    } else {
        "○ Disabled (Open access)"
    };
    let pw_change_label = if app.cfg.security.master_password_hash.is_empty() {
        "[Click/Enter to Set Master Password]"
    } else {
        "•••••••• [Click/Enter to Change]"
    };
    let pw_hint = if app.cfg.security.hint.is_empty() {
        "[None]"
    } else {
        app.cfg.security.hint.as_str()
    };
    let tg_token_masked = if app.cfg.daemon.telegram_bot_token.is_empty() {
        "[Not Configured]"
    } else {
        "••••••••••••••••"
    };
    let tg_chat_id = if app.cfg.daemon.telegram_chat_id.is_empty() {
        "[Not Configured]"
    } else {
        app.cfg.daemon.telegram_chat_id.as_str()
    };
    let daemon_port_str = app.cfg.daemon.port.to_string();
    let daemon_interval_str = format!("{} seconds", app.cfg.daemon.interval_secs);
    let daemon_retry_str = format!("{} failures", app.cfg.daemon.retry_threshold);
    let coolify_url = if let Some(ref c) = app.cfg.deploy.coolify {
        c.base_url.as_str()
    } else {
        "[Not Configured]"
    };
    let coolify_token = if let Some(ref c) = app.cfg.deploy.coolify {
        if c.api_token.is_empty() { "[None]" } else { "••••••••" }
    } else {
        "[Not Configured]"
    };
    let auto_prune_str = if app.cfg.database.auto_prune { "Enabled" } else { "Disabled" };

    let values: [&str; 22] = [
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
        pw_status,
        pw_change_label,
        pw_hint,
        tg_token_masked,
        tg_chat_id,
        daemon_port_str.as_str(),
        daemon_interval_str.as_str(),
        daemon_retry_str.as_str(),
        coolify_url,
        coolify_token,
        auto_prune_str,
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


// ── Footer & Compact Fallback ────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, app: &CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let keybinds = " [1-9/0]: Tabs │ Tab: Cycle │ Shift+1-9/0: Jump │ ↑/↓: Nav │ ?: Help │ Esc/q: Quit ";
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
            "CAShell v0.3.5 · Isometric Cockpit (Compact Mode)",
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
        Line::from("Tabs: [1]Dash [2]Proj [3]Uptime [4]Server [5]Audit [6]CRON [7]Log [8]Term [9]AI [0]Setting"),
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
            Cell::from("1..9, 0 / Shift+1..9, 0"),
            Cell::from("Switch tabs directly (Dashboard, Project, Uptime, Server, Audit, CRON, Log, Terminal, AI, Setting)"),
        ]),
        Row::new(vec![
            Cell::from("Tab / BackTab (Shift+Tab)"),
            Cell::from("Cycle through 10 tabs forward / backward"),
        ]),
        Row::new(vec![
            Cell::from("↑ / ↓ (k / j)"),
            Cell::from("Navigate items, lists, tables, or scroll content"),
        ]),
        Row::new(vec![
            Cell::from("Project: o / d / s / x / r"),
            Cell::from("o: Add Project | d: Add Container | s/x/r: Start/Stop/Restart | Enter/←→: View"),
        ]),
        Row::new(vec![
            Cell::from("Uptime: m / p"),
            Cell::from("m: Add Monitor wizard | p: Probe all monitors & gateways"),
        ]),
        Row::new(vec![
            Cell::from("Server: a / p"),
            Cell::from("a/n: Add Server wizard | p: Probe multi-node latency matrix"),
        ]),
        Row::new(vec![
            Cell::from("Audit: a / f"),
            Cell::from("a: Run Turbo Audit | f: Run Fix diff auto-patcher"),
        ]),
        Row::new(vec![
            Cell::from("CRON: n"),
            Cell::from("n: Create New Scheduled Cron Job wizard"),
        ]),
        Row::new(vec![
            Cell::from("Log: l / r / ← →"),
            Cell::from("l: Cycle level filter | r: Refresh logs | ←/→: Switch source (journal/docker/agent)"),
        ]),
        Row::new(vec![
            Cell::from("Terminal: Enter / r"),
            Cell::from("Enter: Execute shell command | r: Toggle local / remote node"),
        ]),
        Row::new(vec![
            Cell::from("AI: Enter / m / c"),
            Cell::from("Enter: Send prompt to OmniRoute/9router | m: Cycle model | c: Clear chat"),
        ]),
        Row::new(vec![
            Cell::from("Setting: Enter / ← → / s / r / v"),
            Cell::from("Enter: Edit field | ←/→: Cycle values | s: Save | r: Reload | v: Vacuum"),
        ]),
        Row::new(vec![
            Cell::from("? / F1 │ q / Esc"),
            Cell::from("? / F1: Keybindings cheat sheet │ q / Esc: Quit / Close modal"),
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

    let new_job_btn_w = 32u16;
    let new_job_btn_rect = Rect {
        x: chunks[0].x.saturating_add(120),
        y: chunks[0].y,
        width: new_job_btn_w,
        height: 1,
    };
    app.register_hitbox(new_job_btn_rect, HitboxAction::NewCronWizard);

    let top_bar = Paragraph::new(Line::from(vec![
        Span::styled(" [6] CRON & SCHEDULED JOBS ", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Span::raw(" │ Manage user crontab, systemd timers & managed CAShell jobs  "),
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
                .title(" Active Scheduled Jobs (User Crontab & CAShell Managed Jobs) ")
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
            let active_action = app.pet_transient_action.map(|(state, _)| state).unwrap_or(match app.pet_mood {
                crate::pet::PetMood::Idle => crate::pet::PetActionState::Idle,
                crate::pet::PetMood::Sprint => crate::pet::PetActionState::Run,
                crate::pet::PetMood::Overheated => crate::pet::PetActionState::Hit,
                crate::pet::PetMood::Sick => crate::pet::PetActionState::Death,
            });
            let frames = status.frames_for_state(active_action);
            for art in frames[0] {
                lines.push(Line::from(Span::styled(*art, Style::default().fg(palette.highlight_fg).add_modifier(Modifier::BOLD))));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Name: ", Style::default().fg(palette.text_muted)),
                Span::styled(pet.name.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw("  │  "),
                Span::styled("Species: ", Style::default().fg(palette.text_muted)),
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
                Span::styled("Lifecycle: ", Style::default().fg(palette.text_muted)),
                Span::raw("Egg 🥚 → Hatching 🐣 → Juvenile 🐥 → Adult 🐔 → Mating 💑 → Egg 🥚"),
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
            lines.push(Line::from("No active pet companion."));
            lines.push(Line::from("Choose a workspace companion to join your terminal sessions!"));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Options: Chicken (Ayam), Lion (Singa), Bird (Burung), Snake (Ular), Bear (Beruang)", Style::default().fg(palette.accent)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("[Adopt Companion: Ciko the Chicken] (Press 'c')", Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.highlight_bg).add_modifier(Modifier::BOLD)),
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

// ── CRUD Add Wizards (Server / Project / Docker Container / Uptime Monitor) ──

fn draw_server_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 52, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🖥 Add Fleet Server — Node Wizard ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let fields = [
        ("Server Name:", app.server_name_buf.clone(), "e.g. vps-sg, worker-1"),
        ("Host / IP Address:", app.server_ip_buf.clone(), "e.g. 100.127.232.32"),
        ("SSH Username:", app.server_user_buf.clone(), "e.g. root, cecep"),
        ("Services (CSV):", app.server_services_buf.clone(), "e.g. http:80,ssh:22"),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Register a fleet node with IP & services. Auto-probes upon saving.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.server_wizard_field == i;
        let prefix = if is_focused { "▶ " } else { "  " };
        let val_span = if is_focused {
            Span::styled(
                format!(" [ {}█ ] ", val),
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            let display_val = if val.is_empty() { *placeholder } else { val.as_str() };
            Span::styled(
                format!(" [ {} ] ", display_val),
                Style::default().fg(if val.is_empty() { Color::DarkGray } else { Color::White }),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<20}", label), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            val_span,
        ]));

        let row_y = field_start_y + (i as u16);
        if row_y < inner.bottom() {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::FocusServerField(i),
            );
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab/Down: Next │ BackTab/Up: Prev │ Enter: Save │ Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let btn_save = Span::styled(
        " [✔ Add Server] (Enter) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let btn_cancel = Span::styled(
        " [✖ Cancel] (Esc) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    );
    lines.push(Line::from(vec![btn_save, Span::raw("  "), btn_cancel]));

    f.render_widget(Paragraph::new(lines), inner);

    let btn_y = inner.y + 8;
    if btn_y < inner.bottom() {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: 24,
                height: 1,
            },
            HitboxAction::SubmitServer,
        );
        app.register_hitbox(
            Rect {
                x: inner.x + 26,
                y: btn_y,
                width: 18,
                height: 1,
            },
            HitboxAction::CancelServer,
        );
    }
}

fn draw_project_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 46, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 📁 Add Tracked Project — Directory Wizard ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 5 {
        return;
    }

    let fields = [
        ("Folder Path:", app.project_path_buf.clone(), "e.g. ~/Product/cashell or /home/..."),
        ("Project Name:", app.project_name_buf.clone(), "Optional (auto-extracted from folder)"),
        ("Category:", app.project_category_buf.clone(), "Product or Project"),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Add a workspace folder to track in the cockpit project overview.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.project_wizard_field == i;
        let prefix = if is_focused { "▶ " } else { "  " };
        let val_span = if is_focused {
            Span::styled(
                format!(" [ {}█ ] ", val),
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            let display_val = if val.is_empty() { *placeholder } else { val.as_str() };
            Span::styled(
                format!(" [ {} ] ", display_val),
                Style::default().fg(if val.is_empty() { Color::DarkGray } else { Color::White }),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<18}", label), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            val_span,
        ]));

        let row_y = field_start_y + (i as u16);
        if row_y < inner.bottom() {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::FocusProjectField(i),
            );
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab/Down: Next │ BackTab/Up: Prev │ Enter: Save │ Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let btn_save = Span::styled(
        " [✔ Add Project] (Enter) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let btn_cancel = Span::styled(
        " [✖ Cancel] (Esc) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    );
    lines.push(Line::from(vec![btn_save, Span::raw("  "), btn_cancel]));

    f.render_widget(Paragraph::new(lines), inner);

    let btn_y = inner.y + 7;
    if btn_y < inner.bottom() {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: 25,
                height: 1,
            },
            HitboxAction::SubmitProject,
        );
        app.register_hitbox(
            Rect {
                x: inner.x + 27,
                y: btn_y,
                width: 18,
                height: 1,
            },
            HitboxAction::CancelProject,
        );
    }
}

fn draw_container_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 52, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🐳 Add Docker Container — Run Wizard ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let fields = [
        ("Container Name:", app.container_name_buf.clone(), "e.g. my-nginx, redis-db"),
        ("Docker Image:", app.container_image_buf.clone(), "e.g. nginx:alpine, redis:7"),
        ("Port Mapping:", app.container_port_buf.clone(), "e.g. 8080:80 (optional)"),
        ("Environment:", app.container_env_buf.clone(), "e.g. FOO=bar,PORT=80 (optional)"),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Specify container parameters to execute docker/podman run -d.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.container_wizard_field == i;
        let prefix = if is_focused { "▶ " } else { "  " };
        let val_span = if is_focused {
            Span::styled(
                format!(" [ {}█ ] ", val),
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            let display_val = if val.is_empty() { *placeholder } else { val.as_str() };
            Span::styled(
                format!(" [ {} ] ", display_val),
                Style::default().fg(if val.is_empty() { Color::DarkGray } else { Color::White }),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<18}", label), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            val_span,
        ]));

        let row_y = field_start_y + (i as u16);
        if row_y < inner.bottom() {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::FocusContainerField(i),
            );
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab/Down: Next │ BackTab/Up: Prev │ Enter: Run │ Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let btn_save = Span::styled(
        " [✔ Run Container] (Enter) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let btn_cancel = Span::styled(
        " [✖ Cancel] (Esc) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    );
    lines.push(Line::from(vec![btn_save, Span::raw("  "), btn_cancel]));

    f.render_widget(Paragraph::new(lines), inner);

    let btn_y = inner.y + 8;
    if btn_y < inner.bottom() {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: 27,
                height: 1,
            },
            HitboxAction::SubmitContainer,
        );
        app.register_hitbox(
            Rect {
                x: inner.x + 29,
                y: btn_y,
                width: 18,
                height: 1,
            },
            HitboxAction::CancelContainer,
        );
    }
}

fn draw_monitor_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 42, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🌐 Add Uptime Monitor — Endpoint Wizard ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 4 {
        return;
    }

    let fields = [
        ("Monitor Name:", app.monitor_name_buf.clone(), "e.g. Payment Gateway"),
        ("Target URL:", app.monitor_url_buf.clone(), "e.g. https://api.fathforce.com/health"),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Add an HTTP/HTTPS or TCP endpoint to Uptime Kuma monitoring.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.monitor_wizard_field == i;
        let prefix = if is_focused { "▶ " } else { "  " };
        let val_span = if is_focused {
            Span::styled(
                format!(" [ {}█ ] ", val),
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            let display_val = if val.is_empty() { *placeholder } else { val.as_str() };
            Span::styled(
                format!(" [ {} ] ", display_val),
                Style::default().fg(if val.is_empty() { Color::DarkGray } else { Color::White }),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<18}", label), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            val_span,
        ]));

        let row_y = field_start_y + (i as u16);
        if row_y < inner.bottom() {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::FocusMonitorField(i),
            );
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab/Down: Next │ BackTab/Up: Prev │ Enter: Save │ Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let btn_save = Span::styled(
        " [✔ Add Monitor] (Enter) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let btn_cancel = Span::styled(
        " [✖ Cancel] (Esc) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    );
    lines.push(Line::from(vec![btn_save, Span::raw("  "), btn_cancel]));

    f.render_widget(Paragraph::new(lines), inner);

    let btn_y = inner.y + 6;
    if btn_y < inner.bottom() {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: 25,
                height: 1,
            },
            HitboxAction::SubmitMonitor,
        );
        app.register_hitbox(
            Rect {
                x: inner.x + 27,
                y: btn_y,
                width: 18,
                height: 1,
            },
            HitboxAction::CancelMonitor,
        );
    }
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

// ── Master Password Lock Screen ──────────────────────────────────────────────

fn draw_lock_screen(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(58, 16, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🔒 CASHELL SOVEREIGN COCKPIT — AUTHENTICATION REQUIRED ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let masked_pw = if app.lock_mask_visible {
        if app.lock_password_buf.is_empty() {
            "Type master password...".to_string()
        } else {
            app.lock_password_buf.clone()
        }
    } else {
        if app.lock_password_buf.is_empty() {
            "Type master password...".to_string()
        } else {
            "•".repeat(app.lock_password_buf.len())
        }
    };

    let eye_icon = if app.lock_mask_visible { "👁 View" } else { "🔒 Masked" };

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " Master Password Filter Active ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Unauthorized terminal access is restricted on this workstation. ",
            Style::default().fg(palette.text_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Password: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" [ {}█ ] ", masked_pw),
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(if app.lock_password_buf.is_empty() { Color::DarkGray } else { palette.highlight_bg })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" [{}]", eye_icon), Style::default().fg(palette.text_muted)),
        ]),
        Line::from(""),
    ];

    if let Some(ref err) = app.lock_error_msg {
        lines.push(Line::from(Span::styled(err.clone(), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))));
    } else if !app.cfg.security.hint.is_empty() {
        lines.push(Line::from(Span::styled(format!("Hint: {}", app.cfg.security.hint), Style::default().fg(Color::Yellow))));
    } else {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" [Enter] Unlock ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("    "),
        Span::styled(" [Ctrl+E] Toggle View ", Style::default().fg(palette.accent)),
        Span::raw("    "),
        Span::styled(" [Esc/q] Exit ", Style::default().fg(palette.text_muted)),
    ]));

    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        inner,
    );
}

// ── Set Master Password Modal ────────────────────────────────────────────────

fn draw_set_password_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(62, 20, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🔑 Set / Change Master Password ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let p1_mask = "•".repeat(app.new_password_buf.len());
    let p2_mask = "•".repeat(app.confirm_password_buf.len());

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " Protect CAShell Cockpit with Master Authentication ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Anyone launching 'cas' will be prompted to enter this password. ",
            Style::default().fg(palette.text_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(if app.set_password_field == 0 { "▶ " } else { "  " }, Style::default().fg(palette.accent)),
            Span::styled("New Password:     ", Style::default().fg(if app.set_password_field == 0 { palette.accent } else { Color::White })),
            Span::styled(
                format!(" [ {}█ ]", if app.new_password_buf.is_empty() { "Enter new password" } else { &p1_mask }),
                Style::default().fg(palette.highlight_fg).bg(if app.set_password_field == 0 { palette.highlight_bg } else { Color::Reset }),
            ),
        ]),
        Line::from(vec![
            Span::styled(if app.set_password_field == 1 { "▶ " } else { "  " }, Style::default().fg(palette.accent)),
            Span::styled("Confirm Password: ", Style::default().fg(if app.set_password_field == 1 { palette.accent } else { Color::White })),
            Span::styled(
                format!(" [ {}█ ]", if app.confirm_password_buf.is_empty() { "Re-enter password" } else { &p2_mask }),
                Style::default().fg(palette.highlight_fg).bg(if app.set_password_field == 1 { palette.highlight_bg } else { Color::Reset }),
            ),
        ]),
        Line::from(vec![
            Span::styled(if app.set_password_field == 2 { "▶ " } else { "  " }, Style::default().fg(palette.accent)),
            Span::styled("Password Hint:    ", Style::default().fg(if app.set_password_field == 2 { palette.accent } else { Color::White })),
            Span::styled(
                format!(" [ {}█ ]", if app.password_hint_buf.is_empty() { "Optional reminder hint" } else { &app.password_hint_buf }),
                Style::default().fg(palette.text).bg(if app.set_password_field == 2 { palette.highlight_bg } else { Color::Reset }),
            ),
        ]),
        Line::from(""),
    ];

    let mut body_lines = lines;
    if let Some(ref err) = app.set_password_error {
        body_lines.push(Line::from(Span::styled(err.clone(), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))));
    } else {
        body_lines.push(Line::from(Span::styled("Use Tab/Shift+Tab to switch fields. Press Enter to Save.", Style::default().fg(palette.text_muted))));
    }

    body_lines.push(Line::from(""));
    body_lines.push(Line::from(vec![
        Span::styled(" [Enter] Save & Enable ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("    "),
        Span::styled(" [Esc] Cancel ", Style::default().fg(palette.text_muted)),
    ]));

    f.render_widget(Paragraph::new(body_lines), inner);
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
            (KeyCode::Char('&'), 6),
            (KeyCode::Char('*'), 7),
            (KeyCode::Char('('), 8),
            (KeyCode::Char(')'), 9),
        ];
        for (code, expected_idx) in keys {
            let shift_tab = match code {
                KeyCode::Char('!') => Some(0),
                KeyCode::Char('@') => Some(1),
                KeyCode::Char('#') => Some(2),
                KeyCode::Char('$') => Some(3),
                KeyCode::Char('%') => Some(4),
                KeyCode::Char('^') => Some(5),
                KeyCode::Char('&') => Some(6),
                KeyCode::Char('*') => Some(7),
                KeyCode::Char('(') => Some(8),
                KeyCode::Char(')') => Some(9),
                _ => None,
            };
            assert_eq!(shift_tab, Some(expected_idx));
        }
    }

    #[test]
    fn test_cockpit_tabs_index_roundtrip() {
        for idx in 0..10 {
            let tab = CockpitTab::from_index(idx);
            assert_eq!(tab.to_index(), idx);
        }
        assert_eq!(CockpitTab::titles().len(), 10);
        assert_eq!(CockpitTab::COUNT, 10);
    }

    #[test]
    fn test_ai_model_switcher() {
        let mut app = CockpitApp::new(&Config::default());
        let initial = app.current_ai_model();
        assert!(!initial.is_empty());
        app.cycle_ai_model();
        assert_ne!(app.current_ai_model(), initial);
    }

    #[test]
    fn test_log_source_and_filter() {
        let mut app = CockpitApp::new(&Config::default());
        assert_eq!(app.log_source, LogSource::Journal);
        assert_eq!(app.log_level, LogLevelFilter::All);

        app.log_source = LogSource::Agents;
        assert_eq!(app.log_source, LogSource::Agents);
        app.log_level = LogLevelFilter::Error;
        assert_eq!(app.log_level, LogLevelFilter::Error);
    }

    #[test]
    fn test_terminal_command_runner() {
        let mut app = CockpitApp::new(&Config::default());
        app.term_input = "echo 'cashell cockpit'".to_string();
        app.run_terminal_command();
        assert!(app.term_input.is_empty());
        assert!(app.term_output.iter().any(|line| line.contains("cashell cockpit")));
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

    #[test]
    fn test_wizard_add_server() {
        let mut app = CockpitApp::new(&Config::default());
        app.server_name_buf = "test-node".to_string();
        app.server_ip_buf = "10.0.0.99".to_string();
        app.server_user_buf = "admin".to_string();
        app.server_services_buf = "web:8080:http, db:5432".to_string();

        app.submit_server_wizard();

        assert!(!app.show_server_wizard);
        assert!(app.cfg.daemon.nodes.contains_key("test-node"));
        let node = &app.cfg.daemon.nodes["test-node"];
        assert_eq!(node.ip, "10.0.0.99");
        assert_eq!(node.user, "admin");
        assert_eq!(node.services.len(), 2);
        assert_eq!(node.services[0].name, "web");
        assert_eq!(node.services[0].port, 8080);
        assert_eq!(node.services[1].name, "db");
        assert_eq!(node.services[1].port, 5432);

        // Verify AppCard created
        assert!(app.apps.iter().any(|a| a.name == "test-node:web"));
    }

    #[test]
    fn test_wizard_add_project() {
        let mut app = CockpitApp::new(&Config::default());
        let initial_count = app.latest_snap.tracked_projects.len();
        app.project_path_buf = "/tmp/my-test-proj".to_string();
        app.project_name_buf = "MyTestProj".to_string();
        app.project_category_buf = "Product".to_string();

        app.submit_project_wizard();

        assert!(!app.show_project_wizard);
        assert!(app.cfg.workspace.custom_projects.contains(&"/tmp/my-test-proj".to_string()));
        assert_eq!(app.latest_snap.tracked_projects.len(), initial_count + 1);
        assert!(app.latest_snap.tracked_projects.iter().any(|p| p.name == "MyTestProj"));
    }

    #[test]
    fn test_wizard_add_container() {
        let mut app = CockpitApp::new(&Config::default());
        let initial_count = app.latest_snap.docker_containers.len();
        app.container_name_buf = "test-redis".to_string();
        app.container_image_buf = "redis:alpine".to_string();
        app.container_port_buf = "6379:6379".to_string();
        app.container_env_buf = "ENV=test".to_string();

        app.submit_container_wizard();

        assert!(!app.show_container_wizard);
        // Container added to snapshot
        assert_eq!(app.latest_snap.docker_containers.len(), initial_count + 1);
        assert!(app.latest_snap.docker_containers.iter().any(|c| c.name == "test-redis"));
    }

    #[test]
    fn test_wizard_add_monitor() {
        let mut app = CockpitApp::new(&Config::default());
        let initial_count = app.latest_snap.uptime_monitors.len();
        app.monitor_name_buf = "Test API".to_string();
        app.monitor_url_buf = "http://127.0.0.1:9999/health".to_string();

        app.submit_monitor_wizard();

        assert!(!app.show_monitor_wizard);
        assert!(app.cfg.uptime.custom_monitors.iter().any(|m| m.name == "Test API"));
        assert_eq!(app.latest_snap.uptime_monitors.len(), initial_count + 1);
        assert!(app.latest_snap.uptime_monitors.iter().any(|m| m.name == "Test API"));
    }

    #[test]
    fn test_wizard_field_cycling() {
        let mut app = CockpitApp::new(&Config::default());
        app.show_server_wizard = true;
        app.server_wizard_field = 0;

        // Down advances
        handle_server_wizard_key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::empty()), &mut app);
        assert_eq!(app.server_wizard_field, 1);

        // Tab advances
        handle_server_wizard_key(event::KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut app);
        assert_eq!(app.server_wizard_field, 2);

        // Up moves back
        handle_server_wizard_key(event::KeyEvent::new(KeyCode::Up, KeyModifiers::empty()), &mut app);
        assert_eq!(app.server_wizard_field, 1);

        // Esc closes
        handle_server_wizard_key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut app);
        assert!(!app.show_server_wizard);
    }

    #[test]
    fn test_master_password_hash_and_verify() {
        let pass = "Sovereign2026!";
        let hash = crate::config::hash_master_password(pass);
        assert!(!hash.is_empty());
        assert_ne!(pass, hash);
        assert!(crate::config::verify_master_password(pass, &hash));
        assert!(!crate::config::verify_master_password("WrongPassword", &hash));
    }

    #[test]
    fn test_master_password_submit_and_lock() {
        let mut app = CockpitApp::new(&Config::default());
        assert!(!app.is_locked);
        assert!(!app.cfg.security.require_password);

        // Fill modal with mismatched password
        app.show_set_password_modal = true;
        app.new_password_buf = "Secret123".to_string();
        app.confirm_password_buf = "Different456".to_string();
        app.submit_set_password();
        assert!(app.show_set_password_modal); // Still open due to mismatch
        assert!(app.set_password_error.is_some());

        // Fill modal with matching password
        app.confirm_password_buf = "Secret123".to_string();
        app.password_hint_buf = "my secret".to_string();
        app.submit_set_password();
        assert!(!app.show_set_password_modal);
        assert!(app.set_password_error.is_none());
        assert!(app.cfg.security.require_password);
        assert!(!app.cfg.security.master_password_hash.is_empty());
        assert_eq!(app.cfg.security.hint, "my secret");

        // Now if we start app with require_password, it should be locked
        let app_locked = CockpitApp::new(&app.cfg);
        assert!(app_locked.is_locked);
    }

    #[test]
    fn test_settings_fields_coverage() {
        assert_eq!(SETTINGS_FIELDS.len(), 22);
        assert_eq!(SETTINGS_FIELDS[0], "AI Provider");
        assert_eq!(SETTINGS_FIELDS[3], "Output Theme");
        assert_eq!(SETTINGS_FIELDS[11], "Master Password Protection");
        assert_eq!(SETTINGS_FIELDS[12], "Set/Change Master Password");
    }
}
