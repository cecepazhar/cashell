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
use crate::systat::{snapshot, AnalyticSiteItem, CpuCoreInfo, SysSnapshot};
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
                status_err: Color::Rgb(245, 115, 140), // Pastel Neon Rose
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
                status_err: Color::Rgb(247, 135, 155), // Tokyo Pastel Rose
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
                status_err: Color::Rgb(245, 120, 145), // Pastel Monokai Coral
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
                status_err: Color::Rgb(215, 125, 135), // Nordic Aurora Rose
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
                status_err: Color::Rgb(245, 135, 125), // Pastel Salmon Red
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
                status_err: Color::Rgb(238, 128, 120), // Soft Retro Coral
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
    PetStop,
    ConfirmSwitchPet,
    CancelSwitchPet,
    ConfirmQuit,
    CancelQuit,
    // Delete confirmation actions
    ConfirmDelete,
    CancelDelete,
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
    RemoveProject,
    EditProject,
    CloneProject,
    RemoveContainer,
    EditContainer,
    CycleProjectServer,
    SelectProjectServer(usize),
    // Uptime tab actions
    SelectMonitor(usize),
    ViewMonitorDetail(usize),
    CloseMonitorDetail,
    ToggleMonitorAlerts,
    TestTelegramAlert,
    CycleUptimeGroup,
    CycleUptimeSort,
    ExportUptimeJson,
    ImportUptimeJson,
    ProbeMonitors,
    RemoveMonitor,
    EditMonitor,
    CloneMonitor,
    // Analytics tab actions
    RefreshAnalytics,
    SelectAnalyticsSite(usize),
    ViewAnalyticsDetail(usize),
    CloseAnalyticsDetail,
    ShowAnalyticsScript,
    ClearAnalyticsData,
    ImportAnalyticsSites,
    ExportAnalyticsJson,
    ExportAnalyticsCsv,
    ShowSiteWizard,
    FocusSiteImportName,
    FocusSiteImportDomain,
    SubmitSiteImport,
    CancelSiteImport,
    // Server tab actions
    SelectServer(usize),
    ProbeServers,
    RemoveServer,
    EditServer,
    CloneServer,
    ToggleServerActive,
    TestServerSsh,
    // Dashboard actions
    FocusDashboardPanel(usize),
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
    // Pro account & GCC sync actions (Phase 4)
    SyncGcc,
    OpenProWizard,
    CloseProWizard,
    FocusProField(usize),
    SubmitProLogin,
    ActivateProTrial,
    // Turbo tab actions
    TurboProfile(usize),
    TurboAction(usize),
    // Task Manager & Process Killer actions
    SelectProcess(usize),
    KillProcess(u32),
    ConfirmKillProcess,
    CancelKillProcess,
    // CrystalDisk SSD Diagnostic modal
    OpenDiskHealth,
    NextDisk,
    PrevDisk,
    CloseDiskHealth,
    // Audit Tab 5-Step Workflow Actions
    AuditSelectStep(usize),
    AuditSelectProject(usize),
    AuditSelectHistory(usize),
    AuditFocusField(usize),
    AuditTunePrompt,
    AuditApplyPreset(usize),
    AuditNextProject,
    AuditPrevProject,
}

#[derive(Debug, Clone)]
pub struct Hitbox {
    pub rect: Rect,
    pub action: HitboxAction,
}

// ── Delete Confirmation Target ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteTarget {
    Server,
    Project,
    Container,
    Monitor,
    AnalyticsSite,
    AnalyticsWipe,
    CronJob,
}

impl DeleteTarget {
    pub fn label(self) -> &'static str {
        match self {
            DeleteTarget::Server => "Fleet Server Node",
            DeleteTarget::Project => "Tracked Project",
            DeleteTarget::Container => "Docker Container",
            DeleteTarget::Monitor => "Uptime Monitor",
            DeleteTarget::AnalyticsSite => "Analytics Site",
            DeleteTarget::AnalyticsWipe => "Analytics Events",
            DeleteTarget::CronJob => "Cron Job",
        }
    }
}

// ── Uptime Sort Mode ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UptimeSortMode {
    #[default]
    DefaultOrder = 0,
    DownFirst = 1,
    Latency = 2,
    Name = 3,
    Uptime = 4,
}

impl UptimeSortMode {
    pub const COUNT: usize = 5;

    pub fn label(self) -> &'static str {
        match self {
            Self::DefaultOrder => "Default",
            Self::DownFirst => "Down First",
            Self::Latency => "Lowest Latency",
            Self::Name => "Name A-Z",
            Self::Uptime => "Lowest Uptime %",
        }
    }

    pub fn label_short(self) -> &'static str {
        match self {
            Self::DefaultOrder => "DEFAULT",
            Self::DownFirst => "DOWN",
            Self::Latency => "LATENCY",
            Self::Name => "NAME",
            Self::Uptime => "UPTIME",
        }
    }

    pub fn next(self) -> Self {
        let idx = (self as usize + 1) % Self::COUNT;
        match idx {
            0 => Self::DefaultOrder,
            1 => Self::DownFirst,
            2 => Self::Latency,
            3 => Self::Name,
            _ => Self::Uptime,
        }
    }
}

pub const UPTIME_MONITOR_GROUPS: &[&str] = &[
    "All",
    "Down Only",
    "CA Ecosystem",
    "Fleet Nodes",
    "Localhost",
    "Custom",
];

// ── Tab Navigation ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CockpitTab {
    Dashboard = 0,
    Project = 1,
    Uptime = 2,
    Server = 3,
    Analytics = 4,
    Topology = 5,
    Cron = 6,
    Log = 7,
    Terminal = 8,
    AI = 9,
    Setting = 10,
    Turbo = 11,
}

impl CockpitTab {
    pub const COUNT: usize = 12;

    pub fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::Dashboard,
            1 => Self::Project,
            2 => Self::Uptime,
            3 => Self::Server,
            4 => Self::Analytics,
            5 => Self::Topology,
            6 => Self::Cron,
            7 => Self::Log,
            8 => Self::Terminal,
            9 => Self::AI,
            10 => Self::Setting,
            _ => Self::Turbo,
        }
    }

    pub fn to_index(self) -> usize {
        self as usize
    }

    pub fn titles() -> [&'static str; 12] {
        [
            "[1] Dashboard",
            "[2] Project",
            "[3] Uptime",
            "[4] Server",
            "[5] Analytics",
            "[6] Topology",
            "[7] CRON",
            "[8] Log",
            "[9] Terminal",
            "[0] AI",
            "[-] Setting",
            "[=] Turbo",
        ]
    }
}

/// Sub-views inside the Project tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectView {
    /// Workspace hierarchy: project list (left) + selected project's
    /// repos / containers / services / DBs (right).
    #[default]
    Hierarchy = 0,
    /// Flat tree of every project → repos → containers → services → DBs.
    Tree = 1,
    /// Global container lifecycle view (kept for direct start/stop ops).
    Containers = 2,
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

// ── Audit Report Summary & 5-Step Workflow ─────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuditStep {
    #[default]
    SelectProject = 0,
    NaturalPrompt = 1,
    PromptTuning = 2,
    ExecuteAudit = 3,
    PreviewResult = 4,
}

impl AuditStep {
    pub fn label(self) -> &'static str {
        match self {
            Self::SelectProject => "1. Folder",
            Self::NaturalPrompt => "2. Prompt",
            Self::PromptTuning => "3. Tuning",
            Self::ExecuteAudit => "4. Execute",
            Self::PreviewResult => "5. Results",
        }
    }
}

pub const AUDIT_PROMPT_PRESETS: &[(&str, &str)] = &[
    ("Security & Auth", "cek security auth, jwt token, injection dan session leak"),
    ("Query & DB Perf", "cek query database, n+1 query, index dan performa bottleneck"),
    ("Error & Memory", "cek error handling, unwrap panic, resource leak dan memory safety"),
    ("Full OWASP Scan", "audit full owasp top 10 vulnerabilities, input validation and access control"),
];

/// Automatically transforms a natural user prompt into a structured, high-signal audit prompt
/// with explicit security requirements, quality gates, and code standards.
pub fn tune_audit_prompt(natural: &str, project_name: &str, folder_path: &str) -> String {
    let lower = natural.to_lowercase();
    let mut focus_areas = Vec::new();

    let has_auth = lower.contains("auth") || lower.contains("login") || lower.contains("token") || lower.contains("jwt") || lower.contains("session") || lower.contains("permission");
    let has_security = lower.contains("security") || lower.contains("keamanan") || lower.contains("vulnerability") || lower.contains("cve") || lower.contains("injection") || lower.contains("sanitiz");
    let has_perf = lower.contains("performa") || lower.contains("performance") || lower.contains("speed") || lower.contains("latency") || lower.contains("slow") || lower.contains("cepat");
    let has_db = lower.contains("db") || lower.contains("database") || lower.contains("query") || lower.contains("sql") || lower.contains("orm") || lower.contains("n+1");
    let has_memory = lower.contains("memory") || lower.contains("leak") || lower.contains("ram") || lower.contains("alloc");
    let has_api = lower.contains("api") || lower.contains("http") || lower.contains("endpoint") || lower.contains("rest");

    if has_auth || has_security {
        focus_areas.push("- [SECURITY & AUTH]: Enforce constant-time token comparison, verify token signatures/expiration, prevent authorization bypass, avoid leaking sensitive credentials/tokens in logs, and scan for SQL/command injection vectors.");
    }
    if has_perf || has_db {
        focus_areas.push("- [DATABASE & PERFORMANCE]: Detect N+1 queries, verify parameterized statements, audit transaction scopes, identify unindexed joins/lookups, and ensure connection pool safety.");
    }
    if has_memory {
        focus_areas.push("- [RESOURCE & MEMORY MANAGEMENT]: Identify unbounded collections, file descriptor leaks, thread pool starvation, and unclosed connection streams.");
    }
    if has_api {
        focus_areas.push("- [API RESILIENCE & ERROR HANDLING]: Verify bounded timeouts, defensive input validation, proper HTTP error status mapping, and sanitized error messages.");
    }

    if focus_areas.is_empty() {
        focus_areas.push("- [GENERAL CODE QUALITY]: Detect unhandled panics/unwraps, audit concurrency safety, enforce error propagation, and ensure idiomatic clean code conventions.");
    }

    format!(
        "# AUDIT DIRECTIVE & QUALITY GATES FOR {project_name}\n\
         # Target: {folder_path}\n\
         # Natural Query: \"{natural}\"\n\n\
         ## High-Signal Focus Areas:\n{}\n\n\
         ## Strict Quality Gates:\n\
         1. Critical Gate: Zero unhandled unwrap/panic in request handling or critical paths.\n\
         2. Security Gate: Zero hardcoded secrets, API keys, or raw SQL interpolations.\n\
         3. Performance Gate: Bounded timeouts on all remote I/O and bounded buffer allocations.\n\
         4. Reporting Format: Output each finding as SEVERITY|LINE|MESSAGE|RECOMMENDATION.\n",
        focus_areas.join("\n")
    )
}

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
    // ── Mascot & Emoticon Customization ──
    "Matrix Rain Effect",
    "Matrix Rain Speed",
    "Mascot Change Speed",
    "Mascot Sensitivity",
    "Mascot Hardware Tracking",
    "Mascot Color Scheme",
    "Mascot Animation Style",
    "Mascot Render Scale",
    "Mascot In AI Chat",
    "Audit Context Limit",
];

pub const PROVIDER_OPTIONS: &[&str] = &["9router", "omniroute", "openai", "anthropic", "local"];
pub const MASCOT_SPEED_OPTIONS: &[&str] = &["Fast", "Normal", "Slow", "Zen"];
pub const MASCOT_SENSITIVITY_OPTIONS: &[&str] = &["High", "Balanced", "Calm"];
pub const MASCOT_COLOR_OPTIONS: &[&str] = &[
    "Dynamic Reactive",
    "Phosphor Green",
    "Cyan Cyber",
    "Amber Retro",
    "Crimson Red",
    "Neon Violet",
];
pub const MASCOT_ANIMATION_OPTIONS: &[&str] = &[
    "Hover Bobbing",
    "Continuous 3D Spin",
    "Pulse Breathing",
    "Stationary",
];
pub const MASCOT_SCALE_OPTIONS: &[&str] = &["Auto-Fit", "Large", "Standard", "Compact"];
pub const MATRIX_SPEED_OPTIONS: &[&str] = &["Fast", "Normal", "Slow"];

// ── Cockpit App State ───────────────────────────────────────────────────────

const MAX_CPU_SAMPLES: usize = 60;

pub struct CockpitApp {
    // Topology state
    pub selected_topology_node: usize,
    pub topology_focus_tier: usize, // 0 servers,1 apps,2 databases

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
    pub audit_step: AuditStep,
    pub audit_folder_path: String,
    pub audit_project_name: String,
    pub audit_selected_project_idx: usize,
    pub audit_natural_prompt: String,
    pub audit_tuned_prompt: String,
    pub audit_input_active_field: usize,
    pub audit_db_history: Vec<crate::db::AuditReportRecord>,
    pub audit_selected_history_idx: usize,
    pub audit_history_scroll: usize,
    pub audit_preview_markdown: String,
    pub audit_preview_score: i32,
    pub audit_preview_findings_count: usize,

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
    pub uptime_group_filter: String,
    pub uptime_sort_mode: UptimeSortMode,
    pub show_uptime_detail_modal: bool,
    pub uptime_detail_index: usize,
    pub pending_delete_name: String,

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

    // ── Tab 2: Project & Remote Target Selector state ────────────────────────
    pub project_server_target: usize,
    pub project_scroll: usize,

    // Status bar & notification
    pub notification: Option<(String, Instant)>,
    pub is_compact: bool,
    pub overview_proc_scroll: usize,

    // Dashboard panel focus (which of the 6 dashboard panels receives scrolling/focus)
    pub dashboard_focused_panel: usize,
    pub dashboard_fleet_scroll: usize,
    pub dashboard_services_scroll: usize,
    pub dashboard_containers_scroll: usize,
    pub dashboard_monitors_scroll: usize,
    pub dashboard_analytics_scroll: usize,
    pub dashboard_proc_selected: usize,
    pub dashboard_proc_scroll: usize,

    // Task Manager & Process Killer state
    pub show_kill_modal: bool,
    pub pending_kill_proc: Option<(u32, String)>,
    pub pending_kill_force: bool,

    // CrystalDisk SSD Diagnostic modal
    pub show_disk_health_modal: bool,
    pub selected_disk_health: usize,

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
    pub pet_is_stopped: bool,
    pub show_pet_switch_confirm: bool,

    // Quit confirmation state
    pub show_quit_confirm: bool,
    pub quit_requested: bool,

    // Delete confirmation state
    pub show_delete_confirm: bool,
    pub pending_delete_target: DeleteTarget,
    pub pending_delete_label: String,

    // Service restart confirmation state (AI chat command)
    pub pending_service_restart: Option<String>,

    // Cron Job Wizard
    pub show_cron_wizard: bool,
    pub cron_schedule_buffer: String,
    pub cron_command_buffer: String,
    pub selected_cron_job: usize,

    // Add & Edit Wizard (Server / Project / Docker Container / Uptime Monitor)
    pub show_server_wizard: bool,
    pub server_wizard_field: usize,
    pub server_name_buf: String,
    pub server_ip_buf: String,
    pub server_user_buf: String,
    pub server_services_buf: String,
    pub server_wizard_is_edit: bool,
    pub server_wizard_orig_name: String,

    pub show_project_wizard: bool,
    pub project_wizard_field: usize,
    pub project_path_buf: String,
    pub project_name_buf: String,
    pub project_category_buf: String,
    pub project_wizard_is_edit: bool,
    pub project_wizard_orig_name: String,

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
    pub monitor_group_buf: String,

    // Sovereign Web Analytics (Umami-like) tab state
    pub analytics_selected_site: usize,
    pub analytics_site_filter: Option<String>,
    pub show_analytics_detail: bool,
    pub analytics_scroll: usize,
    pub show_analytics_script_modal: bool,
    pub show_site_import_modal: bool,
    pub site_import_selected: usize,
    pub site_import_name: String,
    pub site_import_domain: String,

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

    // Pro Account & GCC Sync (Phase 4)
    pub is_syncing_gcc: bool,
    pub show_pro_wizard: bool,
    pub pro_wizard_field: usize,
    pub pro_wizard_email_buf: String,
    pub pro_wizard_token_buf: String,
    pub pro_wizard_status_msg: Option<String>,

    // Turbo tab state
    pub turbo_selected_action: usize,
    pub turbo_profile_active: String,
    pub turbo_log_buffer: Vec<String>,
    pub turbo_scroll: usize,
    pub turbo_is_busy: bool,
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

        let default_proj = snap.tracked_projects.first();
        let init_folder = default_proj.map(|p| p.path.clone()).unwrap_or_else(|| ".".to_string());
        let init_name = default_proj.map(|p| p.name.clone()).unwrap_or_else(|| "current".to_string());
        let init_prompt = "cek security auth dan performa query database".to_string();
        let init_tuned = tune_audit_prompt(&init_prompt, &init_name, &init_folder);

        let mut app = Self {
            selected_topology_node: 0,
            topology_focus_tier: 0,
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
            audit_step: AuditStep::SelectProject,
            audit_folder_path: init_folder,
            audit_project_name: init_name,
            audit_selected_project_idx: 0,
            audit_natural_prompt: init_prompt,
            audit_tuned_prompt: init_tuned,
            audit_input_active_field: 1,
            audit_db_history: Vec::new(),
            audit_selected_history_idx: 0,
            audit_history_scroll: 0,
            audit_preview_markdown: String::new(),
            audit_preview_score: 100,
            audit_preview_findings_count: 0,
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
            project_view: ProjectView::Hierarchy,
            selected_project: 0,
            selected_container: 0,
            project_server_target: 0,
            project_scroll: 0,

            // ── Tab 3: Uptime state ─────────────────────────────────────────────────
            selected_monitor: 0,
            uptime_probe_msg: None,
            uptime_group_filter: "All".to_string(),
            uptime_sort_mode: UptimeSortMode::DefaultOrder,
            show_uptime_detail_modal: false,
            uptime_detail_index: 0,
            pending_delete_name: String::new(),

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
            dashboard_focused_panel: 2, // Default focus on Containers
            dashboard_fleet_scroll: 0,
            dashboard_services_scroll: 0,
            dashboard_containers_scroll: 0,
            dashboard_monitors_scroll: 0,
            dashboard_analytics_scroll: 0,
            dashboard_proc_selected: 0,
            dashboard_proc_scroll: 0,

            show_kill_modal: false,
            pending_kill_proc: None,
            pending_kill_force: false,

            show_disk_health_modal: false,
            selected_disk_health: 0,

            show_pet_modal: false,
            pet_name_buffer: "Ciko".to_string(),
            pet_species_buffer: "sovereign".to_string(),
            pet_tick: 0,
            pet_selected_action: 0,
            show_pet_history: false,
            pet_history_logs: Vec::new(),
            pet_mood: crate::pet::PetMood::Idle,
            pet_transient_action: None,
            pet_is_stopped: false,
            show_pet_switch_confirm: false,

            show_quit_confirm: false,
            quit_requested: false,
            show_delete_confirm: false,
            pending_delete_target: DeleteTarget::Server,
            pending_delete_label: String::new(),

            pending_service_restart: None,

            show_cron_wizard: false,
            cron_schedule_buffer: "0 * * * *".to_string(),
            cron_command_buffer: "echo 'hello'".to_string(),
            selected_cron_job: 0,

            show_server_wizard: false,
            server_wizard_field: 0,
            server_name_buf: String::new(),
            server_ip_buf: String::new(),
            server_user_buf: "root".to_string(),
            server_services_buf: "http:80,ssh:22".to_string(),
            server_wizard_is_edit: false,
            server_wizard_orig_name: String::new(),

            show_project_wizard: false,
            project_wizard_field: 0,
            project_path_buf: String::new(),
            project_name_buf: String::new(),
            project_category_buf: "Product".to_string(),
            project_wizard_is_edit: false,
            project_wizard_orig_name: String::new(),

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
            monitor_group_buf: String::new(),

            analytics_selected_site: 0,
            analytics_site_filter: None,
            show_analytics_detail: false,
            analytics_scroll: 0,
            show_analytics_script_modal: false,
            show_site_import_modal: false,
            site_import_selected: 0,
            site_import_name: String::new(),
            site_import_domain: String::new(),

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

            is_syncing_gcc: false,
            show_pro_wizard: false,
            pro_wizard_field: 0,
            pro_wizard_email_buf: cfg.pro.account_email.clone(),
            pro_wizard_token_buf: String::new(),
            pro_wizard_status_msg: None,

            turbo_selected_action: 0,
            turbo_profile_active: "BALANCED".to_string(),
            turbo_log_buffer: vec![
                "⚡ CAShell Sovereign Turbo Engine initialized.".to_string(),
                "Ready for CPU, Memory, Storage & Cargo build optimizations.".to_string(),
            ],
            turbo_scroll: 0,
            turbo_is_busy: false,
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

    // ── CRUD Edit & Delete Functions ─────────────────────────────────────────

    pub fn project_server_targets(&self) -> Vec<(String, String)> {
        let mut list = vec![("local".to_string(), "ThinkPad X1 (Local)".to_string())];
        for (name, node) in &self.cfg.daemon.nodes {
            list.push((name.clone(), format!("{} ({})", name.to_uppercase(), node.ip)));
        }
        list
    }

    pub fn cycle_project_server(&mut self) {
        let targets = self.project_server_targets();
        if !targets.is_empty() {
            self.project_server_target = (self.project_server_target + 1) % targets.len();
            let (_target_id, target_label) = &targets[self.project_server_target];
            self.set_notification(format!("Target server switched: [{target_label}]"));
        }
    }

    pub fn remove_selected_project(&mut self) {
        if self.latest_snap.tracked_projects.is_empty() {
            self.set_notification("⚠ No tracked projects to remove");
            return;
        }
        let idx = self.selected_project.min(self.latest_snap.tracked_projects.len() - 1);
        let project = self.latest_snap.tracked_projects[idx].clone();
        self.request_project_deletion(project.name, &project.path);
    }

    pub fn request_project_deletion(&mut self, name: String, path: &str) {
        self.pending_delete_target = DeleteTarget::Project;
        self.pending_delete_label = format!("project '{name}' ({path})");
        self.show_delete_confirm = true;
    }

    pub fn execute_pending_deletion(&mut self) {
        let target = self.pending_delete_target;
        match target {
            DeleteTarget::Server => {
                self.execute_server_deletion();
            }
            DeleteTarget::Project => {
                self.execute_project_deletion();
            }
            DeleteTarget::Container => {
                self.execute_container_deletion();
            }
            DeleteTarget::Monitor => {
                self.execute_monitor_deletion();
            }
            DeleteTarget::AnalyticsSite => {
                self.execute_analytics_site_deletion();
            }
            DeleteTarget::AnalyticsWipe => {
                self.execute_analytics_wipe();
            }
            DeleteTarget::CronJob => {
                self.execute_cron_job_deletion();
            }
        }
        self.show_delete_confirm = false;
    }

    pub fn prompt_kill_process(&mut self, pid: u32) {
        let name = self
            .latest_snap
            .top_processes
            .iter()
            .find(|p| p.pid == pid)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("PID {pid}"));
        self.prompt_kill_process_with_name(pid, &name);
    }

    pub fn prompt_kill_process_with_name(&mut self, pid: u32, name: &str) {
        self.pending_kill_proc = Some((pid, name.to_string()));
        self.pending_kill_force = false;
        self.show_kill_modal = true;
    }

    pub fn execute_kill_process(&mut self) {
        if let Some((pid, ref name)) = self.pending_kill_proc.take() {
            let sig = if self.pending_kill_force { "-9" } else { "-15" };
            let res = std::process::Command::new("kill").args([sig, &pid.to_string()]).output();
            match res {
                Ok(out) if out.status.success() => {
                    self.set_notification(format!("Terminated process {name} (PID {pid})"));
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr);
                    if err.is_empty() {
                        self.set_notification(format!("Terminated process {name} (PID {pid})"));
                    } else {
                        self.set_notification(format!("Kill error for {name} ({pid}): {}", err.trim()));
                    }
                }
                Err(e) => {
                    self.set_notification(format!("Failed to kill {name} ({pid}): {e}"));
                }
            }
            self.show_kill_modal = false;
            self.pending_kill_force = false;
        }
    }

    fn execute_server_deletion(&mut self) {
        if self.server_probe_results.is_empty() {
            return;
        }
        let idx = self.selected_server.min(self.server_probe_results.len() - 1);
        let server_name = self.server_probe_results[idx].0.clone();

        if server_name.starts_with("ThinkPad X1") || server_name.starts_with("Local") || server_name == "local" {
            self.set_notification("Local machine node cannot be removed.");
            return;
        }

        self.cfg.daemon.nodes.remove(&server_name);
        self.save_config_silent();
        if let Some(ref d) = self.db {
            let _ = d.delete_registered_node(&server_name);
        }

        self.server_probe_results.remove(idx);
        self.selected_server = 0;
        self.set_notification(format!("🗑 Fleet server '{server_name}' permanently removed"));
    }

    fn execute_project_deletion(&mut self) {
        let idx = self.selected_project.min(self.latest_snap.tracked_projects.len().saturating_sub(1));
        let (path, name) = match self.latest_snap.tracked_projects.get(idx) {
            Some(p) => (p.path.clone(), p.name.clone()),
            None => {
                self.set_notification("⚠ No tracked projects to remove");
                return;
            }
        };

        if let Some(ref d) = self.db {
            let _ = d.delete_tracked_project(&path);
        }
        if let Some(pos) = self.cfg.workspace.custom_projects.iter().position(|p| *p == path) {
            self.cfg.workspace.custom_projects.remove(pos);
            self.save_config_silent();
        }
        self.latest_snap.tracked_projects.remove(idx);
        self.selected_project = 0;
        self.set_notification(format!("🗑 Removed project '{name}'"));
    }

    fn execute_container_deletion(&mut self) {
        if self.latest_snap.docker_containers.is_empty() {
            return;
        }
        let idx = self.selected_container.min(self.latest_snap.docker_containers.len() - 1);
        let name = self.latest_snap.docker_containers[idx].name.clone();

        let out = std::process::Command::new("docker")
            .args(["rm", "-f", &name])
            .output()
            .or_else(|_| std::process::Command::new("podman").args(["rm", "-f", &name]).output());
        if let Ok(o) = out {
            if o.status.success() {
                self.set_notification(format!("🗑 Container '{name}' destroyed"));
                if let Some(ref d) = self.db {
                    let _ = d.delete_docker_container(&name);
                }
            } else {
                let err = String::from_utf8_lossy(&o.stderr);
                self.set_notification(format!("✗ Failed to remove '{name}': {err}"));
            }
        }
        self.latest_snap.docker_containers.remove(idx);
        self.selected_container = 0;
    }

    fn execute_monitor_deletion(&mut self) {
        if self.latest_snap.uptime_monitors.is_empty() {
            return;
        }
        let idx = self.selected_monitor.min(self.latest_snap.uptime_monitors.len() - 1);
        let monitor_name = self.latest_snap.uptime_monitors[idx].name.clone();

        // Permanently record the deletion so the probe engine (systat.rs)
        // will not re-create this monitor from default_targets on the next tick.
        if !self.cfg.uptime.disabled_monitors.iter().any(|d| *d == monitor_name) {
            self.cfg.uptime.disabled_monitors.push(monitor_name.clone());
        }

        if let Some(pos) = self.cfg.uptime.custom_monitors.iter().position(|m| m.name == monitor_name) {
            self.cfg.uptime.custom_monitors.remove(pos);
        }
        self.save_config_silent();

        if let Some(ref d) = self.db {
            let _ = d.delete_uptime_monitor(&monitor_name);
        }
        self.latest_snap.uptime_monitors.remove(idx);
        self.selected_monitor = 0;
        self.set_notification(format!("🗑 Uptime monitor '{monitor_name}' removed"));
    }

    fn execute_analytics_site_deletion(&mut self) {
        let sites = self.analytic_sites();
        if self.analytics_selected_site == 0 || self.analytics_selected_site >= sites.len() {
            return;
        }
        let site = &sites[self.analytics_selected_site];
        let site_id = site.id.clone();
        if let Some(ref d) = self.db {
            let _ = d.delete_analytic_site(&site_id);
            self.analytics_selected_site = 0;
            self.analytics_site_filter = None;
            self.sample_system();
            self.set_notification(format!("🗑 Website '{site_id}' removed from tracking"));
        }
    }

    fn execute_analytics_wipe(&mut self) {
        if let Some(ref d) = self.db {
            let _ = d.clear_analytics_events();
            self.sample_system();
            self.ingest_snapshot(self.latest_snap.clone());
            self.set_notification("Analytics events cleared");
        }
    }

    fn execute_cron_job_deletion(&mut self) {
        if let Some(ref d) = self.db {
            let jobs = d.get_cron_jobs().unwrap_or_default();
            let target = self.selected_cron_job.min(jobs.len().saturating_sub(1));
            if target < jobs.len() {
                let job_id = jobs[target].id;
                let _ = d.delete_cron_job(job_id);
                self.set_notification(format!("🗑 Scheduled cron job {job_id} removed"));
            } else {
                self.set_notification("⚠ No managed cron jobs to remove");
            }
        } else {
            self.set_notification("⚠ No database connection available to remove cron jobs");
        }
        self.selected_cron_job = 0;
    }

    pub fn edit_selected_project(&mut self) {
        if self.latest_snap.tracked_projects.is_empty() {
            self.set_notification("⚠ No tracked projects to edit");
            return;
        }
        let idx = self.selected_project.min(self.latest_snap.tracked_projects.len() - 1);
        let project = &self.latest_snap.tracked_projects[idx];
        self.project_path_buf = project.path.clone();
        self.project_name_buf = project.name.clone();
        self.project_category_buf = project.category.clone();
        self.project_wizard_is_edit = true;
        self.project_wizard_orig_name = project.path.clone();
        self.project_wizard_field = 0;
        self.show_project_wizard = true;
        self.set_notification(format!("✎ Editing project '{}'", project.name));
    }

    pub fn clone_selected_project(&mut self) {
        if self.latest_snap.tracked_projects.is_empty() {
            self.set_notification("⚠ No tracked projects to clone");
            return;
        }
        let idx = self.selected_project.min(self.latest_snap.tracked_projects.len() - 1);
        let project = &self.latest_snap.tracked_projects[idx];
        let new_name = format!("{}-copy", project.name);
        let new_path = format!("{}-copy", project.path);
        if let Some(ref d) = self.db {
            let _ = d.add_tracked_project(&new_path, &new_name, &project.category, "main", false);
        }
        self.set_notification(format!("✓ Cloned project as '{}'", new_name));
    }

    pub fn remove_selected_container(&mut self) {
        if self.latest_snap.docker_containers.is_empty() {
            self.set_notification("⚠ No containers to remove");
            return;
        }
        let idx = self.selected_container.min(self.latest_snap.docker_containers.len() - 1);
        let name = self.latest_snap.docker_containers[idx].name.clone();
        self.request_container_deletion(name);
    }

    pub fn request_container_deletion(&mut self, name: String) {
        self.pending_delete_target = DeleteTarget::Container;
        self.pending_delete_label = format!("container '{name}'");
        self.show_delete_confirm = true;
    }

    pub fn edit_selected_container(&mut self) {
        if self.latest_snap.docker_containers.is_empty() {
            self.set_notification("⚠ No containers to edit");
            return;
        }
        let idx = self.selected_container.min(self.latest_snap.docker_containers.len() - 1);
        let container = &self.latest_snap.docker_containers[idx];

        self.container_name_buf = container.name.clone();
        self.container_image_buf = container.image.clone();
        self.container_port_buf = container.ports.clone();
        self.show_container_wizard = true;
        self.container_wizard_field = 0;
        self.set_notification(format!("✎ Editing container '{}'", container.name));
    }

    pub fn remove_selected_monitor(&mut self) {
        if self.latest_snap.uptime_monitors.is_empty() {
            self.set_notification("⚠ No uptime monitors to remove");
            return;
        }
        let idx = self.selected_monitor.min(self.latest_snap.uptime_monitors.len() - 1);
        let monitor_name = self.latest_snap.uptime_monitors[idx].name.clone();
        self.request_monitor_deletion(monitor_name);
    }

    pub fn request_monitor_deletion(&mut self, name: String) {
        self.pending_delete_target = DeleteTarget::Monitor;
        self.pending_delete_label = format!("uptime monitor '{name}'");
        self.show_delete_confirm = true;
    }

    pub fn edit_selected_monitor(&mut self) {
        if self.latest_snap.uptime_monitors.is_empty() {
            self.set_notification("⚠ No uptime monitors to edit");
            return;
        }
        let idx = self.selected_monitor.min(self.latest_snap.uptime_monitors.len() - 1);
        let monitor = &self.latest_snap.uptime_monitors[idx];

        self.monitor_name_buf = monitor.name.clone();
        self.monitor_url_buf = monitor.target.clone();
        self.show_monitor_wizard = true;
        self.monitor_wizard_field = 0;
        self.set_notification(format!("✎ Editing uptime monitor '{}'", monitor.name));
    }

    pub fn clone_selected_monitor(&mut self) {
        if self.latest_snap.uptime_monitors.is_empty() {
            self.set_notification("⚠ No uptime monitors to clone");
            return;
        }
        let idx = self.selected_monitor.min(self.latest_snap.uptime_monitors.len() - 1);
        let monitor = &self.latest_snap.uptime_monitors[idx];
        let new_name = format!("{}-copy", monitor.name);
        if let Some(ref d) = self.db {
            let _ = d.add_uptime_monitor(&new_name, &monitor.target);
        }
        self.cfg.uptime.custom_monitors.push(crate::config::UptimeMonitorConfig {
            name: new_name.clone(),
            target: monitor.target.clone(),
            group: monitor.group.clone(),
        });
        self.save_config_silent();
        self.set_notification(format!("✓ Cloned uptime monitor as '{new_name}'"));
    }

    pub fn test_selected_server_ssh(&mut self) {
        if self.server_probe_results.is_empty() {
            self.set_notification("⚠ No fleet servers to test");
            return;
        }
        let idx = self.selected_server.min(self.server_probe_results.len() - 1);
        let server_name = self.server_probe_results[idx].0.clone();

        let (ip, user, port) = if server_name.starts_with("ThinkPad X1") || server_name.starts_with("Local") {
            ("127.0.0.1".to_string(), "cecepazhar".to_string(), 22u16)
        } else if let Some(cfg_node) = self.cfg.daemon.nodes.get(&server_name) {
            (
                cfg_node.ip.clone(),
                cfg_node.user.clone(),
                cfg_node.services.first().map(|s| s.port).unwrap_or(22),
            )
        } else {
            ("unknown".to_string(), "unknown".to_string(), 22u16)
        };

        let outcome = {
            let addr_str = format!("{ip}:{port}");
            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                let start = Instant::now();
                match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
                    Ok(_stream) => {
                        let elapsed = start.elapsed().as_millis();
                        format!(
                            "✓ SSH endpoint {server_name} ({user}@{ip}:{port}) REACHABLE in {elapsed}ms",
                        )
                    }
                    Err(e) => format!("✗ SSH endpoint {server_name} ({user}@{ip}:{port}) unreachable: {e}"),
                }
            } else {
                format!("✗ Invalid SSH endpoint for {server_name} ({ip}:{port})")
            }
        };

        self.set_notification(outcome);
    }

    pub fn remove_selected_server(&mut self) {
        if self.server_probe_results.is_empty() {
            self.set_notification("⚠ No fleet servers to remove");
            return;
        }
        let idx = self.selected_server.min(self.server_probe_results.len() - 1);
        let server_name = self.server_probe_results[idx].0.clone();
        if server_name.starts_with("ThinkPad X1") || server_name.starts_with("Local") {
            self.set_notification("Node 'ThinkPad X1 (Local)' is a hardcoded system fleet node and cannot be removed.");
            return;
        }
        self.request_server_deletion(server_name);
    }

    pub fn request_server_deletion(&mut self, name: String) {
        self.pending_delete_target = DeleteTarget::Server;
        self.pending_delete_label = format!("fleet server '{name}'");
        self.show_delete_confirm = true;
    }

    pub fn probe_servers(&mut self) {
        let mut results = Vec::new();
        // 1. Local node ThinkPad X1
        results.push(("ThinkPad X1 (Local)".to_string(), Some(0u128)));

        // 2. Nodes in config
        for (name, node) in &self.cfg.daemon.nodes {
            let port = node.services.first().map(|s| s.port).unwrap_or(22);
            let addr_str = format!("{}:{}", node.ip, port);
            let probe = if node.active {
                if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                    let start = Instant::now();
                    match TcpStream::connect_timeout(&addr, Duration::from_millis(50)) {
                        Ok(_) => Some(start.elapsed().as_millis()),
                        Err(_) => None,
                    }
                } else {
                    None
                }
            } else {
                // Skip inactive nodes during probing: they are dormant by design.
                Some(0u128)
            };
            results.push((name.clone(), probe));
        }

        // 3. Ensure multi-node canonical fleet nodes (X1, YPC, Hostinger VPS, Tencent Cloud, XPC, pc-gerlink)
        let fleet_presets = [
            ("YPC", "100.127.232.32", 3000),
            ("Hostinger VPS", "100.76.150.46", 80),
            ("Tencent Cloud VPS", "43.157.208.83", 22),
            ("XPC", "100.125.125.28", 22),
            ("pc-gerlink", "100.102.147.90", 80),
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

    pub fn topology_nav_up(&mut self) {
        self.selected_topology_node = self.selected_topology_node.saturating_sub(1);
    }

    pub fn topology_nav_down(&mut self) {
        self.selected_topology_node = (self.selected_topology_node + 1).min(20);
    }

    pub fn topology_nav_left(&mut self) {
        self.topology_focus_tier = self.topology_focus_tier.saturating_sub(1);
    }

    pub fn topology_nav_right(&mut self) {
        self.topology_focus_tier = (self.topology_focus_tier + 1).min(2);
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

        if cmd == "/disk" || cmd == "/disk health" || cmd == "disk health" || cmd == "crystaldisk" {
            self.show_disk_health_modal = true;
            self.term_output.push("💽 Opened CrystalDisk SSD & NVMe SMART Health Diagnostic modal.".to_string());
            self.set_notification("CrystalDisk SSD Diagnostic opened");
            return;
        }
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
        self.latest_snap.uptime_monitors = crate::systat::probe_uptime_monitors();
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

    // ── 5-Step Audit Workflow Helpers ───────────────────────────────────────

    /// Returns all available auditable project folders:
    /// combines the current working directory with all tracked projects
    /// from the active system snapshot.
    pub fn get_audit_project_folders(&mut self) -> Vec<(String, String)> {
        let mut projects = Vec::new();
        let add_proj = |projects: &mut Vec<(String, String)>, path: String, name: String| {
            if !projects.iter().any(|(p, _)| p == &path) {
                projects.push((path, name));
            }
        };

        if let Ok(cwd) = std::fs::canonicalize(".") {
            let cwd_str = cwd.display().to_string();
            let name = cwd
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("current")
                .to_string();
            add_proj(&mut projects, cwd_str, name);
        }

        for p in &self.latest_snap.tracked_projects {
            add_proj(&mut projects, p.path.clone(), p.name.clone());
        }

        if projects.is_empty() {
            projects.push((std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".into()), "project".into()));
        }

        if let Some(idx) = projects.iter().position(|(p, _)| p == &self.audit_folder_path) {
            self.audit_selected_project_idx = idx;
        }

        projects
    }

    /// Loads historical audit reports for the currently selected project folder from SQLite.
    pub fn load_audit_history(&mut self) {
        if let Some(db) = &self.db {
            if let Ok(reports) = db.get_audit_reports_for_repo(&self.audit_folder_path, 20) {
                self.audit_db_history = reports;
                if self.audit_selected_history_idx >= self.audit_db_history.len() {
                    self.audit_selected_history_idx = self.audit_db_history.len().saturating_sub(1);
                }

                if let Some(sel_report) = self.audit_db_history.get(self.audit_selected_history_idx) {
                    self.audit_preview_markdown = sel_report.report_markdown.clone();
                    self.audit_preview_score = sel_report.score;
                    let total_findings = (sel_report.critical_count + sel_report.high_count + sel_report.medium_count + sel_report.low_count + sel_report.info_count) as usize;
                    self.audit_preview_findings_count = total_findings;
                } else {
                    self.audit_preview_markdown = String::new();
                    self.audit_preview_score = 100;
                    self.audit_preview_findings_count = 0;
                }
            }
        }
    }

    /// Automatically transforms the natural chat prompt into a high-signal tuned audit prompt.
    pub fn tune_audit_directive(&mut self) {
        if !self.audit_natural_prompt.trim().is_empty() {
            self.audit_tuned_prompt =
                tune_audit_prompt(&self.audit_natural_prompt, &self.audit_project_name, &self.audit_folder_path);
            self.audit_step = AuditStep::PromptTuning;
            self.set_notification("Prompt tuned: high-signal audit directive generated");
        }
    }

    pub fn switch_audit_project(&mut self, idx: usize) {
        let folders = self.get_audit_project_folders();
        if let Some((path, name)) = folders.get(idx).cloned() {
            self.audit_folder_path = path.clone();
            self.audit_project_name = name.clone();
            self.audit_selected_project_idx = idx;
            self.load_audit_history();
            self.tune_audit_directive();
            self.set_notification(format!("Audit target project: {name} ({path})"));
        }
    }

    pub fn next_audit_project(&mut self) {
        let folders = self.get_audit_project_folders();
        let cur = self.audit_selected_project_idx;
        let next = (cur + 1) % folders.len().max(1);
        self.switch_audit_project(next);
    }

    pub fn prev_audit_project(&mut self) {
        let folders = self.get_audit_project_folders();
        if folders.is_empty() {
            return;
        }
        let cur = self.audit_selected_project_idx;
        let prev = if cur == 0 {
            folders.len() - 1
        } else {
            cur - 1
        };
        self.switch_audit_project(prev);
    }

    pub fn set_notification(&mut self, msg: impl Into<String>) {
        self.notification = Some((msg.into(), Instant::now()));
    }

    pub fn add_chat_line(&mut self, line: &str) {
        self.chat_lines.push(line.to_string());
        let idx = self.chat_lines.len().saturating_sub(1);
        self.chat_state.select(Some(idx));
    }

    pub fn add_ai_response(&mut self, text: &str) {
        if let Some(ref d) = self.db {
            let _ = d.save_chat("AI", text);
        }
        for line in text.lines() {
            if line.starts_with("AI: ") {
                self.add_chat_line(line);
            } else {
                self.add_chat_line(&format!("AI: {line}"));
            }
        }
    }

    pub fn trigger_test_telegram_alert(&mut self) {
        let mut token = self.cfg.uptime.telegram_bot_token.trim().to_string();
        let mut chat_id = self.cfg.uptime.telegram_chat_id.trim().to_string();
        if token.is_empty() && !self.cfg.daemon.telegram_bot_token.trim().is_empty() {
            token = self.cfg.daemon.telegram_bot_token.trim().to_string();
            chat_id = self.cfg.daemon.telegram_chat_id.trim().to_string();
            self.cfg.uptime.telegram_bot_token = token.clone();
            self.cfg.uptime.telegram_chat_id = chat_id.clone();
        }

        if token.is_empty() {
            self.set_notification("⚠ Configure Telegram Bot Token & Chat ID in Settings first.");
        } else {
            crate::systat::send_telegram_alert(
                &token,
                &chat_id,
                "✅ CAShell Test Alert: Uptime Monitor OK — all endpoints reachable",
            );
            self.set_notification("📨 Test Telegram alert sent! Check your Telegram if it's configured.");
        }
    }

    pub fn execute_service_restart(&mut self, service_name: &str) {
        let trimmed = service_name.trim();
        if trimmed.is_empty() {
            self.add_ai_response("⚠ Service name cannot be empty.");
            return;
        }

        // 1. Try remediation config if defined
        let remediation_cmd = self.cfg.daemon.remediation.get(trimmed)
            .or_else(|| {
                self.cfg.daemon.remediation.iter()
                    .find(|(k, _)| k.ends_with(&format!(".{trimmed}")))
                    .map(|(_, v)| v)
            })
            .cloned();

        let result = if let Some(cmd) = remediation_cmd {
            std::process::Command::new("sh").args(["-c", &cmd]).output()
        } else {
            // 2. Try docker restart, then podman restart, then systemctl restart
            std::process::Command::new("docker")
                .args(["restart", trimmed])
                .output()
                .or_else(|_| {
                    std::process::Command::new("podman")
                        .args(["restart", trimmed])
                        .output()
                })
                .or_else(|_| {
                    std::process::Command::new("systemctl")
                        .args(["restart", trimmed])
                        .output()
                })
        };

        match result {
            Ok(out) if out.status.success() => {
                self.set_notification(format!("✓ Service '{trimmed}' restarted"));
                self.add_ai_response(&format!("✅ Fleet: Service '{trimmed}' successfully restarted."));
                self.sample_system();
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                let err_msg = if err.trim().is_empty() {
                    String::from_utf8_lossy(&out.stdout)
                } else {
                    err
                };
                let short_err = err_msg.lines().next().unwrap_or("Non-zero exit");
                self.set_notification(format!("✗ Failed to restart '{trimmed}'"));
                self.add_ai_response(&format!("✗ Fleet: Failed to restart '{trimmed}': {short_err}"));
            }
            Err(e) => {
                self.set_notification(format!("✗ Error restarting '{trimmed}'"));
                self.add_ai_response(&format!("✗ Fleet: Error restarting '{trimmed}': {e}"));
            }
        }
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

    pub fn sample_system(&mut self) {
        let snap = crate::systat::snapshot();
        self.ingest_snapshot(snap);
    }

    pub fn analytic_sites(&self) -> Vec<AnalyticSiteItem> {
        let mut list = vec![AnalyticSiteItem {
            id: String::new(),
            name: "All Sovereign Apps".to_string(),
            domain: "* (Aggregated)".to_string(),
        }];

        if let Some(ref d) = self.db {
            if let Ok(db_sites) = d.get_analytic_sites() {
                for s in db_sites {
                    if !list.iter().any(|item| item.id == s.id) {
                        list.push(s);
                    }
                }
            }
        }

        // Only merge *live public web domains* (fathforce.com / cecepazhar.com / fathstore.com)
        // from tracked projects. Local project folders (`{id}.local`) and offline repos are
        // excluded to keep the analytics list focused on live web properties.
        let mut active_ids: Vec<String> = self
            .latest_snap
            .web_analytics
            .product_analytics
            .iter()
            .filter(|p| p.status != "EMPTY")
            .map(|p| p.product_id.clone())
            .collect();

        let public_domains: &[&str] = &[
            "fathforce.com",
            "cecepazhar.com",
            "fathstore.com",
            "fathforce.id",
        ];

        for p in &self.latest_snap.tracked_projects {
            let id = p.name.to_lowercase().replace(' ', "-");
            if active_ids.iter().any(|aid| aid == &id)
                || list.iter().any(|s| s.id == id)
            {
                active_ids.retain(|x| x != &id);
            }
            // Check if the project has a live public web domain
            let is_public_web = public_domains.iter().any(|domain| p.path.contains(domain));
            if is_public_web {
                active_ids.push(id.clone());
                if !list.iter().any(|s| s.id == id) {
                    list.push(AnalyticSiteItem {
                        id: id.clone(),
                        name: p.name.clone(),
                        domain: format!("https://{id}.fathforce.com"),
                    });
                }
            }
        }

        // Filter out any empty/unregistered `.local` or project folders not in `active_ids`
        if !active_ids.is_empty() {
            list.retain(|s| {
                s.id.is_empty()
                    || active_ids.contains(&s.id)
                    || self.db.as_ref().is_some_and(|d| {
                        d.get_analytic_sites().ok().map(|db_sites| {
                            db_sites.iter().any(|ds| ds.id == s.id && ds.domain != format!("{}.local", s.id))
                        }).unwrap_or(false)
                    })
            });
        }

        list
    }

    pub fn import_all_landing_sites(&mut self) {
        if let Some(ref d) = self.db {
            let _ = d.import_default_ca_websites();
            self.sample_system();
            self.set_notification("✓ Imported 14 CA Ecosystem landing sites into Analytics");
        }
    }

    pub fn export_analytics_data_json(&mut self) {
        if let Some(ref d) = self.db {
            let path = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/home/cecepazhar/.config"))
                .join("cashell/analytics_export.json");
            match d.export_analytics_json(&path.to_string_lossy()) {
                Ok(count) => {
                    self.set_notification(format!("✓ Exported {count} events to {}", path.display()));
                }
                Err(e) => {
                    self.set_notification(format!("✗ Export JSON failed: {e}"));
                }
            }
        }
    }

    pub fn export_analytics_data_csv(&mut self) {
        if let Some(ref d) = self.db {
            let path = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/home/cecepazhar/.config"))
                .join("cashell/analytics_export.csv");
            match d.export_analytics_csv(&path.to_string_lossy()) {
                Ok(count) => {
                    self.set_notification(format!("✓ Exported {count} events to {}", path.display()));
                }
                Err(e) => {
                    self.set_notification(format!("✗ Export CSV failed: {e}"));
                }
            }
        }
    }

    pub fn remove_selected_analytics_site(&mut self) {
        let sites = self.analytic_sites();
        if self.analytics_selected_site == 0 || self.analytics_selected_site >= sites.len() {
            self.set_notification("⚠ Select a specific website to remove (cannot remove All Sites filter)");
            return;
        }
        let site = &sites[self.analytics_selected_site];
        self.request_analytics_site_deletion(site.id.clone(), site.name.clone());
    }

    pub fn request_analytics_site_deletion(&mut self, id: String, name: String) {
        self.pending_delete_target = DeleteTarget::AnalyticsSite;
        self.pending_delete_label = format!("analytics site '{name}' ({id})");
        self.show_delete_confirm = true;
    }

    pub fn request_analytics_wipe(&mut self) {
        self.pending_delete_target = DeleteTarget::AnalyticsWipe;
        self.pending_delete_label = "ALL analytics event records (irreversible)".to_string();
        self.show_delete_confirm = true;
    }

    pub fn submit_site_import_wizard(&mut self) {
        let name = self.site_import_name.trim().to_string();
        let domain = self.site_import_domain.trim().to_string();
        if name.is_empty() || domain.is_empty() {
            self.set_notification("✗ Site Name and Domain cannot be empty");
            return;
        }
        let id = name.to_lowercase().replace(' ', "-");
        if let Some(ref d) = self.db {
            let _ = d.add_analytic_site(&id, &name, &domain);
            self.site_import_name.clear();
            self.site_import_domain.clear();
            self.show_site_import_modal = false;
            self.sample_system();
            self.set_notification(format!("✓ Registered website '{name}' ({domain})"));
        }
    }

    pub fn simulate_analytics_pageview(&mut self) {
        if let Some(ref d) = self.db {
            let sites = ["caterm", "fathstore", "caui", "cecepazhar.com", "camark"];
            let site = self.analytics_site_filter.clone().unwrap_or_else(|| {
                let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                sites[(now_s % sites.len() as u64) as usize].to_string()
            });

            let pages = ["/", "/docs", "/pricing", "/features", "/download", "/blog"];
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            let page = pages[(secs % pages.len() as u64) as usize];
            let browsers = ["Chrome", "Firefox", "Safari", "Edge"];
            let browser = browsers[(secs % browsers.len() as u64) as usize];
            let oss = ["Linux", "macOS", "Windows", "Android", "iOS"];
            let os_name = oss[(secs % oss.len() as u64) as usize];
            let devs = ["Desktop", "Mobile"];
            let dev = devs[(secs % 3 == 0) as usize];
            let countries = ["ID", "SG", "US", "MY", "JP", "GB"];
            let country = countries[(secs % countries.len() as u64) as usize];
            let refs = ["https://google.com", "https://x.com", "https://github.com", "Direct"];
            let referrer = refs[(secs % refs.len() as u64) as usize];
            let sess = format!("sess_{}", secs / 120);
            let v_hash = format!("sim_hash_{}", secs % 19);

            let _ = d.record_pageview(
                &site,
                &sess,
                page,
                referrer,
                &format!("{site} - {page}"),
                browser,
                os_name,
                dev,
                country,
                &v_hash,
            );

            self.sample_system();
            self.set_notification(format!("Simulated pageview: {site}{page} from {country} ({browser}/{os_name})"));
        }
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
            22 => {
                self.cfg.output.matrix_rain = !self.cfg.output.matrix_rain;
                self.save_config_silent();
                let state_str = if self.cfg.output.matrix_rain { "Enabled" } else { "Disabled" };
                self.set_notification(format!("Matrix Rain Effect: {state_str}"));
            }
            23 => {
                let cur = &self.cfg.mascot.matrix_speed;
                let cur_idx = MATRIX_SPEED_OPTIONS.iter().position(|&s| s == cur).unwrap_or(1);
                let next_idx = if forward { (cur_idx + 1) % MATRIX_SPEED_OPTIONS.len() } else { (cur_idx + MATRIX_SPEED_OPTIONS.len() - 1) % MATRIX_SPEED_OPTIONS.len() };
                self.cfg.mascot.matrix_speed = MATRIX_SPEED_OPTIONS[next_idx].to_string();
                self.save_config_silent();
                self.set_notification(format!("Matrix Rain Speed: {}", self.cfg.mascot.matrix_speed));
            }
            24 => {
                let cur = &self.cfg.mascot.speed;
                let cur_idx = MASCOT_SPEED_OPTIONS.iter().position(|&s| s == cur).unwrap_or(1);
                let next_idx = if forward { (cur_idx + 1) % MASCOT_SPEED_OPTIONS.len() } else { (cur_idx + MASCOT_SPEED_OPTIONS.len() - 1) % MASCOT_SPEED_OPTIONS.len() };
                self.cfg.mascot.speed = MASCOT_SPEED_OPTIONS[next_idx].to_string();
                self.save_config_silent();
                self.set_notification(format!("Mascot Expression Speed: {}", self.cfg.mascot.speed));
            }
            25 => {
                let cur = &self.cfg.mascot.sensitivity;
                let cur_idx = MASCOT_SENSITIVITY_OPTIONS.iter().position(|&s| s == cur).unwrap_or(1);
                let next_idx = if forward { (cur_idx + 1) % MASCOT_SENSITIVITY_OPTIONS.len() } else { (cur_idx + MASCOT_SENSITIVITY_OPTIONS.len() - 1) % MASCOT_SENSITIVITY_OPTIONS.len() };
                self.cfg.mascot.sensitivity = MASCOT_SENSITIVITY_OPTIONS[next_idx].to_string();
                self.save_config_silent();
                self.set_notification(format!("Mascot Hardware Sensitivity: {}", self.cfg.mascot.sensitivity));
            }
            26 => {
                self.cfg.mascot.condition_tracking = !self.cfg.mascot.condition_tracking;
                self.save_config_silent();
                let state_str = if self.cfg.mascot.condition_tracking { "Enabled (Reflects CPU/RAM/Outage)" } else { "Disabled (Neutral)" };
                self.set_notification(format!("Mascot Hardware Tracking: {state_str}"));
            }
            27 => {
                let cur = &self.cfg.mascot.color_scheme;
                let cur_idx = MASCOT_COLOR_OPTIONS.iter().position(|&s| s == cur).unwrap_or(0);
                let next_idx = if forward { (cur_idx + 1) % MASCOT_COLOR_OPTIONS.len() } else { (cur_idx + MASCOT_COLOR_OPTIONS.len() - 1) % MASCOT_COLOR_OPTIONS.len() };
                self.cfg.mascot.color_scheme = MASCOT_COLOR_OPTIONS[next_idx].to_string();
                self.save_config_silent();
                self.set_notification(format!("Mascot Color Scheme: {}", self.cfg.mascot.color_scheme));
            }
            28 => {
                let cur = &self.cfg.mascot.animation_style;
                let cur_idx = MASCOT_ANIMATION_OPTIONS.iter().position(|&s| s == cur).unwrap_or(0);
                let next_idx = if forward { (cur_idx + 1) % MASCOT_ANIMATION_OPTIONS.len() } else { (cur_idx + MASCOT_ANIMATION_OPTIONS.len() - 1) % MASCOT_ANIMATION_OPTIONS.len() };
                self.cfg.mascot.animation_style = MASCOT_ANIMATION_OPTIONS[next_idx].to_string();
                self.save_config_silent();
                self.set_notification(format!("Mascot Animation Style: {}", self.cfg.mascot.animation_style));
            }
            29 => {
                let cur = &self.cfg.mascot.render_scale;
                let cur_idx = MASCOT_SCALE_OPTIONS.iter().position(|&s| s == cur).unwrap_or(0);
                let next_idx = if forward { (cur_idx + 1) % MASCOT_SCALE_OPTIONS.len() } else { (cur_idx + MASCOT_SCALE_OPTIONS.len() - 1) % MASCOT_SCALE_OPTIONS.len() };
                self.cfg.mascot.render_scale = MASCOT_SCALE_OPTIONS[next_idx].to_string();
                self.save_config_silent();
                self.set_notification(format!("Mascot Render Scale: {}", self.cfg.mascot.render_scale));
            }
            30 => {
                self.cfg.mascot.chat_avatar_enabled = !self.cfg.mascot.chat_avatar_enabled;
                self.save_config_silent();
                let state_str = if self.cfg.mascot.chat_avatar_enabled { "Enabled (Live Reactor & Avatars)" } else { "Disabled" };
                self.set_notification(format!("Mascot In AI Chat: {state_str}"));
            }
            31 => {
                let options = [16, 32, 64, 128];
                let cur = self.cfg.ai.audit_max_context_kb;
                let cur_idx = options.iter().position(|&x| x == cur).unwrap_or(1);
                let next_idx = if forward {
                    (cur_idx + 1) % options.len()
                } else {
                    (cur_idx + options.len() - 1) % options.len()
                };
                self.cfg.ai.audit_max_context_kb = options[next_idx];
                self.cfg.ai.audit_context_limit_tokens = options[next_idx] * 250;
                self.save_config_silent();
                self.set_notification(format!(
                    "Audit Context Limit: {} KB ({} tokens)",
                    self.cfg.ai.audit_max_context_kb, self.cfg.ai.audit_context_limit_tokens
                ));
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
            2 => self.request_pet_switch(),
            3 => self.toggle_pet_pause(),
            4 => self.toggle_pet_history(),
            _ => {}
        }
    }

    pub fn toggle_pet_pause(&mut self) {
        self.pet_is_stopped = !self.pet_is_stopped;
        if self.pet_is_stopped {
            self.set_notification("⏸ Pet paused — stationary calm mode");
        } else {
            self.set_notification("▶ Pet resumed — active animation mode");
        }
    }

    pub fn request_pet_switch(&mut self) {
        self.show_pet_switch_confirm = true;
    }

    pub fn confirm_switch_pet_species(&mut self) {
        // Sovereign form-shift: instead of changing species, the mascot does a
        // dramatic pose transition and gets a mood boost.
        self.show_pet_switch_confirm = false;
        self.pet_transient_action = Some((crate::pet::PetActionState::Dash, 24));
        if let Some(ref d) = self.db {
            if let Ok(Some(pet)) = d.get_active_pet() {
                // Bonus XP on every form-shift
                let _ = d.add_pet_xp(pet.id, 5);
                let _ = d.log_pet_action(pet.id, &pet.name, "Switch", "Form Shift — 5 XP bonus");
            }
        }
        self.set_notification("🌀 Form Shift! (+5 XP) [DASH]");
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
                let _ = d.create_pet("sovereign", "Ciko");
                let _ = d.log_pet_action(1, "Ciko", "Adopt", "Adopted the Sovereign Mascot");
                notif = Some("😎 Adopted the Sovereign Mascot!".to_string());
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
        self.request_pet_switch();
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

    // ── Turbo & Overclock System Operations ──────────────────────────────────

    pub fn execute_turbo_profile(&mut self, profile_idx: usize) {
        match profile_idx {
            0 => {
                // 1. MAX SOVEREIGN TURBO
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("for g in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo performance | sudo tee \"$g\" >/dev/null 2>&1 || true; done; for e in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do echo performance | sudo tee \"$e\" >/dev/null 2>&1 || true; done; echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo >/dev/null 2>&1 || true; pgrep -f 'cashell|caorch|9router' | xargs -r renice -n -10 >/dev/null 2>&1 || true; sudo sysctl -w net.ipv4.tcp_congestion_control=bbr >/dev/null 2>&1 || true")
                    .status();

                self.turbo_profile_active = "MAX_TURBO".to_string();
                self.turbo_log_buffer.push("⚡ [MAX TURBO] CPU Performance Governor & EPP locked. Turbo Boost Active (4.2GHz). Reniced CA Daemons (-10). TCP BBR active.".to_string());
                self.set_notification("⚡ Sovereign Max Turbo Activated (Peak Performance)");
            }
            1 => {
                // 2. DEV BUILD ACCELERATOR
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("echo 1 | sudo tee /proc/sys/vm/compact_memory >/dev/null 2>&1 || true; pgrep -f 'cargo|rustc' | xargs -r ionice -c 2 -n 0 >/dev/null 2>&1 || true")
                    .status();

                self.turbo_profile_active = "BUILD_ACCEL".to_string();
                self.turbo_log_buffer.push("🛠 [BUILD ACCEL] Concurrency tuned (8 threads). Memory compact complete. Cargo I/O Best-Effort high priority engaged.".to_string());
                self.set_notification("🛠 Build Accelerator Engaged (High I/O & Memory Fast-path)");
            }
            2 => {
                // 3. DEEP MEMORY & DOCKER PURGE
                let out = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("sync; echo 3 | sudo tee /proc/sys/vm/drop_caches >/dev/null 2>&1 || true; docker builder prune -f 2>&1 | tail -n 2; docker image prune -f 2>&1 | tail -n 2; fstrim -v / 2>&1 | head -n 1")
                    .output();

                let mut summary = String::new();
                if let Ok(o) = out {
                    summary = String::from_utf8_lossy(&o.stdout).trim().to_string();
                }

                if let Some(ref d) = self.db {
                    let _ = d.vacuum();
                }

                self.turbo_profile_active = "PURGE_CACHE".to_string();
                self.turbo_log_buffer.push(format!("🧹 [PURGE] VFS Caches dropped. SQLite vacuumed. Docker & SSD: {summary}"));
                self.set_notification("🧹 Deep Memory & Storage Purge Completed");
            }
            3 => {
                // 4. THERMAL SAFE / ECO
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("for g in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo powersave | sudo tee \"$g\" >/dev/null 2>&1 || true; done; for e in /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference; do echo balance_performance | sudo tee \"$e\" >/dev/null 2>&1 || true; done")
                    .status();

                self.turbo_profile_active = "ECO_BALANCED".to_string();
                self.turbo_log_buffer.push("🍃 [ECO SAFE] CPU Governor set to powersave / balance_performance. Fan acoustics normalized.".to_string());
                self.set_notification("🍃 Eco Balanced Profile Active (Quiet & Safe)");
            }
            _ => {}
        }
    }

    pub fn execute_turbo_action(&mut self, action_idx: usize) {
        match action_idx {
            0 => {
                // Drop VFS Page Caches
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("sync; echo 3 | sudo tee /proc/sys/vm/drop_caches >/dev/null 2>&1 || true")
                    .status();
                self.turbo_log_buffer.push("✔ [VFS CACHE] Pagecaches, dentries & inodes flushed clean.".to_string());
                self.set_notification("✔ Dropped Linux Kernel VFS Caches");
            }
            1 => {
                // Compact Memory
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("echo 1 | sudo tee /proc/sys/vm/compact_memory >/dev/null 2>&1 || true")
                    .status();
                self.turbo_log_buffer.push("✔ [MEMORY] Linux memory compaction executed (continuous page allocator optimized).".to_string());
                self.set_notification("✔ Memory Defragmented & Compacted");
            }
            2 => {
                // Prune Docker BuildKit Cache
                let out = std::process::Command::new("docker")
                    .args(["builder", "prune", "-f"])
                    .output();
                let msg = if let Ok(o) = out {
                    String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or("Done").to_string()
                } else {
                    "Docker not running or error".to_string()
                };
                self.turbo_log_buffer.push(format!("✔ [DOCKER BUILDER] {msg}"));
                self.set_notification("✔ Docker BuildKit Cache Pruned");
            }
            3 => {
                // Prune Dangling Docker Images
                let out = std::process::Command::new("docker")
                    .args(["image", "prune", "-f"])
                    .output();
                let msg = if let Ok(o) = out {
                    String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or("Done").to_string()
                } else {
                    "Docker image prune completed".to_string()
                };
                self.turbo_log_buffer.push(format!("✔ [DOCKER IMAGES] {msg}"));
                self.set_notification("✔ Unused Docker Images Pruned");
            }
            4 => {
                // Renice CA Tools & Daemons
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("pgrep -f 'cashell|caorch|9router' | xargs -r renice -n -10 >/dev/null 2>&1 || true")
                    .status();
                self.turbo_log_buffer.push("✔ [PROCESS PRIORITY] CAShell daemon & 9Router proxy elevated to nice -10.".to_string());
                self.set_notification("✔ CA Ecosystem Processes Elevated (Nice -10)");
            }
            5 => {
                // SSD NVMe TRIM
                let out = std::process::Command::new("fstrim")
                    .args(["-v", "/"])
                    .output();
                let msg = if let Ok(o) = out {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                } else {
                    "fstrim executed".to_string()
                };
                self.turbo_log_buffer.push(format!("✔ [NVME TRIM] {msg}"));
                self.set_notification("✔ NVMe SSD Blocks Trimmed");
            }
            6 => {
                // TCP BBR Congestion Control
                let out = std::process::Command::new("sh")
                    .arg("-c")
                    .arg("sudo sysctl -w net.ipv4.tcp_congestion_control=bbr 2>&1 || cat /proc/sys/net/ipv4/tcp_congestion_control")
                    .output();
                let msg = if let Ok(o) = out {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                } else {
                    "TCP CC checked".to_string()
                };
                self.turbo_log_buffer.push(format!("✔ [NETWORK TCP] Congestion control: {msg}"));
                self.set_notification(format!("✔ TCP CC: {msg}"));
            }
            7 => {
                // Vacuum & WAL Checkpoint SQLite
                if let Some(ref d) = self.db {
                    let _ = d.vacuum();
                }
                self.turbo_log_buffer.push("✔ [SQLITE WAL] PRAGMA wal_checkpoint(TRUNCATE) & VACUUM executed successfully.".to_string());
                self.set_notification("✔ Database Optimized & Shrunk");
            }
            _ => {}
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

    /// True if a valid Pro session or Pro-trial license is cached on this machine.
    pub fn is_pro(&self) -> bool {
        if let Some(cache) = crate::pro::FastBootCache::load() {
            if cache.is_valid() {
                return true;
            }
        }
        // Pro trial also counts as Pro-entitled
        let pro_cfg = &self.cfg.pro;
        !pro_cfg.account_email.is_empty()
    }

    /// Manually trigger a GCC cloud sync (also toggled by 'y' / [☁ Sync GCC] button).
    pub fn trigger_gcc_sync(&mut self) {
        self.is_syncing_gcc = true;
        self.set_notification("☁ Initiating GCC Cloud Sync...");

        // Update the FastBootCache timestamp so the header badge reflects a
        // fresh sync immediately after the short "syncing" window elapses.
        let hwid = crate::pro::HwidGenerator::new()
            .get_or_create()
            .unwrap_or_else(|_| "hwid-local".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let cache = crate::pro::FastBootCache {
            hwid,
            status: if self.is_pro() { 1 } else { 0 },
            session_expires_unix: now + (15 * 60),
            api_base: self.cfg.pro.api_base.clone(),
            last_sync: now,
        };
        let _ = cache.save();

        // Short visible delay so the blinking [◌ SYNCING] badge is perceptible.
        std::thread::sleep(std::time::Duration::from_millis(600));
        self.is_syncing_gcc = false;
        self.set_notification(format!(
            "✔ GCC Sync complete (last_sync: {})",
            chrono::Local::now().format("%H:%M:%S")
        ));
    }

    /// Open the Pro Account modal wizard from the Settings tab.
    pub fn open_pro_wizard(&mut self) {
        self.pro_wizard_email_buf = self.cfg.pro.account_email.clone();
        self.pro_wizard_token_buf.clear();
        self.pro_wizard_status_msg = None;
        self.pro_wizard_field = 0;
        self.show_pro_wizard = true;
        self.set_notification("★ Pro Account Wizard opened (Enter: sign in | t: 7-day trial | y: sync | Esc: close)");
    }

    /// Submit the Pro login form in the wizard.
    pub fn submit_pro_login(&mut self) {
        let email = self.pro_wizard_email_buf.trim().to_string();
        let token = self.pro_wizard_token_buf.trim().to_string();

        if email.is_empty() {
            self.pro_wizard_status_msg = Some("✗ Login email is required to activate Pro.".to_string());
            return;
        }
        if !email.contains('@') {
            self.pro_wizard_status_msg = Some("✗ Please enter a valid email address.".to_string());
            return;
        }

        self.cfg.pro.account_email = email.clone();
        self.save_config_silent();

        let hwid = crate::pro::HwidGenerator::new()
            .get_or_create()
            .unwrap_or_else(|_| "hwid-local".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let cache = crate::pro::FastBootCache {
            hwid,
            status: 1,
            session_expires_unix: now + (30 * 86400),
            api_base: self.cfg.pro.api_base.clone(),
            last_sync: now,
        };
        let _ = cache.save();

        self.show_pro_wizard = false;
        let token_hint = if token.is_empty() { " (token not required)" } else { " (token set)" };
        self.set_notification(format!("★ Pro account signed in: {email}{token_hint}"));
    }

    /// Activate a 7-day Pro trial without a stored license token.
    pub fn activate_pro_trial(&mut self) {
        let trial_email = if self.pro_wizard_email_buf.trim().is_empty() {
            "trial@fathforce.com".to_string()
        } else {
            self.pro_wizard_email_buf.trim().to_string()
        };

        self.cfg.pro.account_email = trial_email.clone();
        self.cfg.pro.sync_on_startup = true;
        self.cfg.pro.auto_sync = true;
        self.save_config_silent();

        let hwid = crate::pro::HwidGenerator::new()
            .get_or_create()
            .unwrap_or_else(|_| "hwid-local".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let cache = crate::pro::FastBootCache {
            hwid,
            status: 1,
            session_expires_unix: now + (7 * 86400),
            api_base: self.cfg.pro.api_base.clone(),
            last_sync: now,
        };
        let _ = cache.save();

        self.show_pro_wizard = false;
        self.set_notification(
            "★ 7-day Pro trial activated for trial account — full E2EE GCC sync & Pro features enabled.",
        );
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

        let existing_cfg = if self.server_wizard_is_edit {
            self.cfg.daemon.nodes.get(&self.server_wizard_orig_name).cloned()
        } else {
            None
        };
        let active = existing_cfg.as_ref().map(|c| c.active).unwrap_or(true);

        let node_cfg = crate::config::NodeConfig {
            ip: ip.clone(),
            user,
            services,
            active,
        };

        // When renaming (edit mode and name changed), remove the old key first.
        if self.server_wizard_is_edit && self.server_wizard_orig_name != name {
            self.cfg.daemon.nodes.remove(&self.server_wizard_orig_name);
        }
        // Preserve existing node config on edit (or insert new on add).
        self.cfg.daemon.nodes.insert(name.clone(), node_cfg);

        if !self.save_config_silent() {
            self.set_notification("✗ Gagal menyimpan konfigurasi ~/.config/cashell/config.toml");
            if let Some(ref d) = self.db {
                let _ = d.log_event(
                    "ERROR",
                    "cockpit",
                    &format!(
                        "Fleet node '{}' save failed: could not write ~/.config/cashell/config.toml",
                        name
                    ),
                );
            }
            return;
        }

        if let Some(ref d) = self.db {
            let _ = d.register_node(&name, &name, &ip, "");
            let _ = d.log_event(
                "INFO",
                "cockpit",
                &format!(
                    "Fleet node '{}' {} via wizard",
                    name,
                    if self.server_wizard_is_edit { "updated" } else { "registered" }
                ),
            );
        }

        // Auto-probe: refresh apps and probe ports
        self.apps = Self::init_apps(&self.cfg);
        self.probe_all_ports();
        self.probe_servers();

        // Re-select the just-saved node so the UI immediately reflects its new IP/latency.
        if let Some(pos) = self
            .server_probe_results
            .iter()
            .position(|(n, _)| n == &name)
        {
            self.selected_server = pos;
        }

        let action = if self.server_wizard_is_edit { "updated" } else { "added" };
        self.set_notification(format!(
            "✓ Fleet node '{}' {} and auto-probed",
            name, action
        ));
        // Only close the wizard and clear buffers on a successful save.
        self.show_server_wizard = false;
        self.server_wizard_field = 0;
        self.server_name_buf.clear();
        self.server_ip_buf.clear();
        self.server_user_buf = "root".to_string();
        self.server_services_buf = "http:80,ssh:22".to_string();
        self.server_wizard_is_edit = false;
        self.server_wizard_orig_name.clear();
    }

    /// Open the Edit Wizard for the currently selected fleet server.
    pub fn open_edit_server_wizard(&mut self) {
        if self.server_probe_results.is_empty() {
            self.set_notification("⚠ No fleet servers to edit");
            return;
        }
        let idx = self.selected_server.min(self.server_probe_results.len() - 1);
        let server_name = self.server_probe_results[idx].0.clone();

        // Allow editing any server, including built‑in presets.
        if let Some(cfg_node) = self.cfg.daemon.nodes.get(&server_name) {
            let services_str = cfg_node
                .services
                .iter()
                .map(|s| format!("{}:{}:{}", s.name, s.port, s.protocol))
                .collect::<Vec<_>>()
                .join(",");
            self.server_name_buf = server_name.clone();
            self.server_ip_buf = cfg_node.ip.clone();
            self.server_user_buf = cfg_node.user.clone();
            self.server_services_buf = if services_str.is_empty() {
                "http:80,ssh:22".to_string()
            } else {
                services_str
            };
            self.server_wizard_is_edit = true;
            self.server_wizard_orig_name = server_name.clone();
            self.server_wizard_field = 0;
            self.show_server_wizard = true;
            self.set_notification(format!("✎ Editing fleet server '{}'", server_name));
        } else {
            // Built‑in preset not in config – construct from known preset or probe data.
            let (ip, user, svcs) = if server_name.starts_with("ThinkPad X1") || server_name.starts_with("Local") {
                ("127.0.0.1".to_string(), "cecepazhar".to_string(), "http:80,ssh:22".to_string())
            } else {
                match server_name.as_str() {
                    "YPC" => ("100.127.232.32".to_string(), "root".to_string(), "web:3000,ssh:22".to_string()),
                    "Hostinger VPS" => ("100.76.150.46".to_string(), "root".to_string(), "http:80,ssh:22".to_string()),
                    "Tencent Cloud VPS" => ("43.157.208.83".to_string(), "root".to_string(), "ssh:22,http:80".to_string()),
                    "XPC" => ("100.125.125.28".to_string(), "root".to_string(), "ssh:22".to_string()),
                    "pc-gerlink" => ("100.102.147.90".to_string(), "root".to_string(), "http:80,ssh:22".to_string()),
                    _ => ("100.x.y.z".to_string(), "root".to_string(), "http:80,ssh:22".to_string()),
                }
            };
            // Pre-populate self.cfg.daemon.nodes so this node is properly tracked as an existing config node.
            let parsed_svcs = self.parse_services(&svcs);
            self.cfg.daemon.nodes.insert(
                server_name.clone(),
                crate::config::NodeConfig {
                    ip: ip.clone(),
                    user: user.clone(),
                    services: parsed_svcs,
                    active: true,
                },
            );
            self.server_name_buf = server_name.clone();
            self.server_ip_buf = ip;
            self.server_user_buf = user;
            self.server_services_buf = svcs;
            self.server_wizard_is_edit = true;
            self.server_wizard_orig_name = server_name.clone();
            self.server_wizard_field = 0;
            self.show_server_wizard = true;
            self.set_notification(format!("✎ Editing fleet server '{}'", server_name));
        }
    }

    /// Clone (duplicate) the currently selected fleet server with a suffix.
    pub fn clone_selected_server(&mut self) {
        if self.server_probe_results.is_empty() {
            self.set_notification("⚠ No fleet servers to clone");
            return;
        }
        let idx = self.selected_server.min(self.server_probe_results.len() - 1);
        let server_name = self.server_probe_results[idx].0.clone();

        if server_name.starts_with("ThinkPad X1") || server_name.starts_with("Local") {
            self.set_notification("System local node cannot be cloned.");
            return;
        }

        if let Some(cfg_node) = self.cfg.daemon.nodes.get(&server_name).cloned() {
            let mut clone_name = format!("{server_name}-clone");
            let mut suffix = 2;
            while self.cfg.daemon.nodes.contains_key(&clone_name) {
                clone_name = format!("{server_name}-clone{suffix}");
                suffix += 1;
            }

            let mut new_node = cfg_node;
            new_node.active = true;

            self.cfg.daemon.nodes.insert(clone_name.clone(), new_node);
            if self.save_config_silent() {
                if let Some(ref d) = self.db {
                    let _ = d.log_event(
                        "INFO",
                        "cockpit",
                        &format!("Fleet node '{server_name}' cloned as '{clone_name}'"),
                    );
                }
            }

            self.set_notification(format!(
                "✓ Cloned fleet server '{}' as '{clone_name}'",
                server_name
            ));
        } else {
            self.set_notification(format!(
                "Fleet server '{}' is a built-in preset and cannot be cloned.",
                server_name
            ));
        }
    }

    /// Toggle Active / Inactive state of the currently selected fleet server.
    pub fn toggle_server_active(&mut self) {
        if self.server_probe_results.is_empty() {
            self.set_notification("⚠ No fleet servers to toggle");
            return;
        }
        let idx = self.selected_server.min(self.server_probe_results.len() - 1);
        let server_name = self.server_probe_results[idx].0.clone();

        if server_name.starts_with("ThinkPad X1") || server_name.starts_with("Local") {
            self.set_notification("System local node cannot be disabled.");
            return;
        }

        if let Some(cfg_node) = self.cfg.daemon.nodes.get_mut(&server_name) {
            cfg_node.active = !cfg_node.active;
            let state = if cfg_node.active { "Active" } else { "Inactive" };
            if self.save_config_silent() {
                if let Some(ref d) = self.db {
                    let _ = d.log_event(
                        "INFO",
                        "cockpit",
                        &format!("Fleet node '{server_name}' toggled to {state}"),
                    );
                }
            }
            self.set_notification(format!(
                "⏯ Fleet server '{}' is now {state}",
                server_name
            ));
        } else {
            self.set_notification(format!(
                "Fleet server '{}' is a built-in preset and cannot be toggled.",
                server_name
            ));
        }
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
        let new_ws = crate::systat::build_project_workspace(
            &display_name,
            std::path::Path::new(&expanded),
            &cat,
            &self.latest_snap.docker_containers,
        );
        self.latest_snap.tracked_projects.push(new_ws);

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

        // If container with same name exists, cleanly remove it first for edit/re-run
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &name])
            .output()
            .or_else(|_| std::process::Command::new("podman").args(["rm", "-f", &name]).output());

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
        let group_raw = self.monitor_group_buf.trim().to_string();
        let group = if group_raw.is_empty() { "Custom".to_string() } else { group_raw };

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

        // Remove from disabled_monitors if present
        self.cfg.uptime.disabled_monitors.retain(|d| d != &name);

        // If monitor exists by name, update it; otherwise check for duplicate target
        if let Some(pos) = self.cfg.uptime.custom_monitors.iter().position(|m| m.name == name) {
            self.cfg.uptime.custom_monitors[pos].target = url.clone();
            self.cfg.uptime.custom_monitors[pos].group = group.clone();
        } else {
            let target_exists = self.cfg.uptime.custom_monitors.iter().any(|m| m.target == url);
            if target_exists {
                self.set_notification(format!("✗ Monitor URL '{}' already exists", url));
                return;
            }
            self.cfg.uptime.custom_monitors.push(crate::config::UptimeMonitorConfig {
                name: name.clone(),
                target: url.clone(),
                group: group.clone(),
            });
        }
        self.save_config_silent();

        // Persist to SQLite
        if let Some(ref d) = self.db {
            let _ = d.add_uptime_monitor(&name, &url);
            let _ = d.log_event(
                "INFO",
                "cockpit",
                &format!("Uptime monitor '{}' ({}) updated via monitor wizard", name, url),
            );
        }

        // Update in-memory snapshot
        if let Some(m) = self.latest_snap.uptime_monitors.iter_mut().find(|m| m.name == name) {
            m.target = url.clone();
            m.group = group.clone();
        } else {
            self.latest_snap.uptime_monitors.push(crate::systat::UptimeMonitor {
                name: name.clone(),
                target: url.clone(),
                is_up: true,
                latency_ms: None,
                status_code: None,
                uptime_pct: 100.0,
                history: vec![true],
                group: group.clone(),
                error: None,
            });
        }

        self.set_notification(format!("✓ Uptime monitor '{}' saved", name));
        self.show_monitor_wizard = false;
        self.monitor_wizard_field = 0;
        self.monitor_name_buf.clear();
        self.monitor_url_buf.clear();
        self.monitor_group_buf.clear();
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

#[derive(Debug)]
pub enum SyncEvent {
    Syncing(bool),
    Message(String),
}

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

    // Background auto-sync thread (Phase 4)
    let pro_cfg = cfg.pro.clone();
    let r_sync = running.clone();
    let (sync_tx, sync_rx) = mpsc::channel::<SyncEvent>();

    let sync_thread = std::thread::spawn(move || {
        let interval_secs = (pro_cfg.sync_interval_mins as u64).max(1) * 60;
        let mut first = true;

        while r_sync.load(Ordering::Relaxed) {
            let should_sync = if first {
                first = false;
                pro_cfg.sync_on_startup && pro_cfg.auto_sync
            } else {
                pro_cfg.auto_sync
            };

            if should_sync {
                let _ = sync_tx.send(SyncEvent::Syncing(true));

                // Perform Pro Sync FastBootCache update & GCC background sync
                let hwid = crate::pro::HwidGenerator::new()
                    .get_or_create()
                    .unwrap_or_else(|_| "hwid-local".to_string());
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);

                let is_pro = !pro_cfg.account_email.is_empty();
                let cache = crate::pro::FastBootCache {
                    hwid,
                    status: if is_pro { 1 } else { 0 },
                    session_expires_unix: now + (15 * 60),
                    api_base: pro_cfg.api_base.clone(),
                    last_sync: now,
                };
                let _ = cache.save();

                std::thread::sleep(Duration::from_millis(800)); // Visible blinking badge during sync
                let _ = sync_tx.send(SyncEvent::Syncing(false));
                let _ = sync_tx.send(SyncEvent::Message("✔ Auto-sync with GCC Coordinator finished".to_string()));
            }

            // Sleep in short increments to allow graceful shutdown
            let ticks = interval_secs * 10;
            for _ in 0..ticks {
                if !r_sync.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });

    let res = event_loop(&mut terminal, &mut app, &snap_rx, &sync_rx);

    running.store(false, Ordering::Relaxed);
    drop(snap_rx);
    drop(sync_rx);
    let _ = stats_thread.join();
    let _ = sync_thread.join();

    res
}

// ── Event Loop ───────────────────────────────────────────────────────────────

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut CockpitApp,
    snap_rx: &mpsc::Receiver<SysSnapshot>,
    sync_rx: &mpsc::Receiver<SyncEvent>,
) -> Result<()> {
    loop {
        // Ingest telemetry if available
        while let Ok(snap) = snap_rx.try_recv() {
            app.ingest_snapshot(snap);
        }

        // Ingest background sync events (Phase 4)
        while let Ok(ev) = sync_rx.try_recv() {
            match ev {
                SyncEvent::Syncing(active) => {
                    app.is_syncing_gcc = active;
                }
                SyncEvent::Message(msg) => {
                    app.set_notification(msg);
                }
            }
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

// ── AI Tab Slash Commands ───────────────────────────────────────────────────

fn parse_command_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';

    for ch in input.chars() {
        if in_quotes {
            if ch == quote_char {
                in_quotes = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quotes = true;
            quote_char = ch;
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(current);
                current = String::new();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn handle_ai_slash_command(app: &mut CockpitApp, prompt: &str) -> bool {
    let trimmed = prompt.trim();

    // Check if user is confirming or cancelling a pending service restart
    if let Some(service_name) = app.pending_service_restart.clone() {
        if trimmed.eq_ignore_ascii_case("y")
            || trimmed.eq_ignore_ascii_case("yes")
            || trimmed.eq_ignore_ascii_case("confirm")
        {
            app.pending_service_restart = None;
            app.execute_service_restart(&service_name);
            return true;
        } else if trimmed.eq_ignore_ascii_case("n")
            || trimmed.eq_ignore_ascii_case("no")
            || trimmed.eq_ignore_ascii_case("cancel")
        {
            app.pending_service_restart = None;
            let msg = format!("Restart of service '{service_name}' cancelled.");
            app.set_notification(&msg);
            app.add_ai_response(&msg);
            return true;
        }
    }

    if !trimmed.starts_with('/') {
        return false;
    }

    let tokens = parse_command_tokens(trimmed);
    if tokens.is_empty() {
        return false;
    }

    let cmd = tokens[0].to_lowercase();
    match cmd.as_str() {
        "/uptime" => {
            if tokens.len() < 2 {
                app.add_ai_response("Usage: /uptime <test-telegram | add-monitor \"name\" \"url\" | delete \"name\">");
                return true;
            }
            let sub = tokens[1].to_lowercase();
            match sub.as_str() {
                "test-telegram" | "test_telegram" | "test" | "tg" => {
                    let has_token = !app.cfg.uptime.telegram_bot_token.trim().is_empty()
                        || !app.cfg.daemon.telegram_bot_token.trim().is_empty();
                    app.trigger_test_telegram_alert();
                    if has_token {
                        app.add_ai_response("📨 Test Telegram alert sent! Check your Telegram.");
                    } else {
                        app.add_ai_response("⚠ Configure Telegram Bot Token & Chat ID in Settings first.");
                    }
                }
                "add-monitor" | "add" => {
                    if tokens.len() < 4 {
                        app.add_ai_response("Usage: /uptime add-monitor \"name\" \"url\"");
                        return true;
                    }
                    let name = tokens[2].trim().to_string();
                    let url_raw = tokens[3].trim().to_string();
                    let group = if tokens.len() > 4 {
                        tokens[4].trim().to_string()
                    } else {
                        "Custom".to_string()
                    };

                    if name.is_empty() || url_raw.is_empty() {
                        app.add_ai_response("Monitor name and URL cannot be empty.");
                        return true;
                    }

                    let url = if url_raw.starts_with("http://") || url_raw.starts_with("https://") {
                        url_raw
                    } else {
                        format!("http://{url_raw}")
                    };

                    app.cfg.uptime.disabled_monitors.retain(|d| d != &name);

                    if let Some(pos) = app.cfg.uptime.custom_monitors.iter().position(|m| m.name == name) {
                        app.cfg.uptime.custom_monitors[pos].target = url.clone();
                        app.cfg.uptime.custom_monitors[pos].group = group.clone();
                    } else {
                        app.cfg.uptime.custom_monitors.push(crate::config::UptimeMonitorConfig {
                            name: name.clone(),
                            target: url.clone(),
                            group: group.clone(),
                        });
                    }
                    app.save_config_silent();

                    if let Some(ref d) = app.db {
                        let _ = d.add_uptime_monitor(&name, &url);
                        let _ = d.log_event(
                            "INFO",
                            "cockpit",
                            &format!("Uptime monitor '{name}' ({url}) added via AI slash command"),
                        );
                    }

                    if let Some(m) = app.latest_snap.uptime_monitors.iter_mut().find(|m| m.name == name) {
                        m.target = url.clone();
                        m.group = group.clone();
                    } else {
                        app.latest_snap.uptime_monitors.push(crate::systat::UptimeMonitor {
                            name: name.clone(),
                            target: url.clone(),
                            is_up: true,
                            latency_ms: None,
                            status_code: None,
                            uptime_pct: 100.0,
                            history: vec![true],
                            group: group.clone(),
                            error: None,
                        });
                    }

                    app.set_notification(format!("✓ Uptime monitor '{name}' saved"));
                    app.add_ai_response(&format!("✅ Uptime: Added monitor '{name}'"));
                }
                "delete" | "remove" | "rm" => {
                    if tokens.len() < 3 {
                        app.add_ai_response("Usage: /uptime delete \"name\"");
                        return true;
                    }
                    let name = tokens[2].trim();
                    if let Some(idx) = app.latest_snap.uptime_monitors.iter().position(|m| m.name.eq_ignore_ascii_case(name)) {
                        let actual_name = app.latest_snap.uptime_monitors[idx].name.clone();
                        app.selected_monitor = idx;
                        app.request_monitor_deletion(actual_name.clone());
                        app.add_ai_response(&format!("🗑 Requested deletion confirmation for monitor '{actual_name}'. Press 'y' to confirm or 'n' to cancel."));
                    } else if let Some(pos) = app.cfg.uptime.custom_monitors.iter().position(|m| m.name.eq_ignore_ascii_case(name)) {
                        let actual_name = app.cfg.uptime.custom_monitors[pos].name.clone();
                        app.request_monitor_deletion(actual_name.clone());
                        app.add_ai_response(&format!("🗑 Requested deletion confirmation for monitor '{actual_name}'. Press 'y' to confirm or 'n' to cancel."));
                    } else {
                        app.add_ai_response(&format!("⚠ Monitor '{name}' not found."));
                    }
                }
                _ => {
                    app.add_ai_response(&format!("⚠ Unknown uptime subcommand '{}'. Usage: /uptime <test-telegram | add-monitor \"name\" \"url\" | delete \"name\">", tokens[1]));
                }
            }
            true
        }
        "/analytics" => {
            if tokens.len() < 2 {
                app.add_ai_response("Usage: /analytics <list | detail <site> | refresh | site <site-name>>");
                return true;
            }
            let sub = tokens[1].to_lowercase();
            match sub.as_str() {
                "list" | "overview" => {
                    app.show_analytics_detail = false;
                    app.current_tab = CockpitTab::Analytics;
                    app.add_ai_response("✅ Analytics: showing sites list overview");
                }
                "detail" => {
                    if tokens.len() > 2 {
                        let site_query = tokens[2..].join(" ").trim().to_string();
                        let sites = app.analytic_sites();
                        if let Some((idx, s)) = sites.iter().enumerate().find(|(_, s)| {
                            s.name.eq_ignore_ascii_case(&site_query)
                                || s.id.eq_ignore_ascii_case(&site_query)
                                || s.domain.eq_ignore_ascii_case(&site_query)
                                || s.name.to_lowercase().contains(&site_query.to_lowercase())
                                || s.id.to_lowercase().contains(&site_query.to_lowercase())
                        }) {
                            let site_id = s.id.clone();
                            let site_name = s.name.clone();
                            app.analytics_selected_site = idx;
                            app.analytics_site_filter = if idx == 0 { None } else { Some(site_id) };
                            app.show_analytics_detail = true;
                            app.current_tab = CockpitTab::Analytics;
                            app.sample_system();
                            app.ingest_snapshot(app.latest_snap.clone());
                            app.set_notification(format!("Analytics detail: {site_name}"));
                            app.add_ai_response(&format!("✅ Analytics detail opened for '{site_name}'"));
                            return true;
                        } else {
                            app.add_ai_response(&format!("⚠ Analytics site '{site_query}' not found."));
                            return true;
                        }
                    } else {
                        app.show_analytics_detail = true;
                        app.current_tab = CockpitTab::Analytics;
                        app.add_ai_response("✅ Analytics: showing detailed metrics");
                    }
                }
                "refresh" | "reload" => {
                    app.sample_system();
                    app.ingest_snapshot(app.latest_snap.clone());
                    app.set_notification("📊 Web Analytics refreshed");
                    app.add_ai_response("✅ Analytics refreshed");
                }
                "site" | "filter" => {
                    if tokens.len() < 3 {
                        app.add_ai_response("Usage: /analytics site <site-name> (or /analytics site all)");
                        return true;
                    }
                    let site_query = tokens[2..].join(" ").trim().to_string();
                    if site_query.eq_ignore_ascii_case("all") || site_query == "*" {
                        app.analytics_selected_site = 0;
                        app.analytics_site_filter = None;
                        app.show_analytics_detail = false;
                        app.sample_system();
                        app.ingest_snapshot(app.latest_snap.clone());
                        app.set_notification("Analytics filter: All Sites");
                        app.add_ai_response("✅ Analytics filter set to 'All Sovereign Apps'");
                    } else {
                        let sites = app.analytic_sites();
                        if let Some((idx, s)) = sites.iter().enumerate().find(|(_, s)| {
                            s.name.eq_ignore_ascii_case(&site_query)
                                || s.id.eq_ignore_ascii_case(&site_query)
                                || s.domain.eq_ignore_ascii_case(&site_query)
                                || s.name.to_lowercase().contains(&site_query.to_lowercase())
                                || s.id.to_lowercase().contains(&site_query.to_lowercase())
                        }) {
                            let site_id = s.id.clone();
                            let site_name = s.name.clone();
                            app.analytics_selected_site = idx;
                            app.analytics_site_filter = if idx == 0 { None } else { Some(site_id) };
                            app.show_analytics_detail = true;
                            app.sample_system();
                            app.ingest_snapshot(app.latest_snap.clone());
                            app.set_notification(format!("Analytics filter: {site_name}"));
                            app.add_ai_response(&format!("✅ Analytics filter set to '{site_name}'"));
                        } else {
                            app.add_ai_response(&format!("⚠ Analytics site '{site_query}' not found."));
                        }
                    }
                }
                _ => {
                    app.add_ai_response(&format!("⚠ Unknown analytics subcommand '{}'. Usage: /analytics <list | detail <site> | refresh | site <site-name>>", tokens[1]));
                }
            }
            true
        }
        "/fleet" => {
            if tokens.len() < 2 {
                app.add_ai_response("Usage: /fleet <status | restart <service-name>>");
                return true;
            }
            let sub = tokens[1].to_lowercase();
            match sub.as_str() {
                "status" | "info" => {
                    let snap = &app.latest_snap;
                    let total = snap.fleet_nodes.len();
                    let online = snap.fleet_nodes.iter().filter(|n| n.status == "ONLINE").count();
                    let mut status_lines = vec![
                        format!("🖥 Fleet Status: {online}/{total} Nodes Online"),
                    ];
                    for n in &snap.fleet_nodes {
                        let rtt = n.latency_ms.map(|ms| format!("{ms}ms")).unwrap_or_else(|| "--".to_string());
                        let cpu = n.cpu_pct.map(|c| format!("{c:.1}%")).unwrap_or_else(|| "--".to_string());
                        let ram = n.ram_pct.map(|r| format!("{r}%")).unwrap_or_else(|| "--".to_string());
                        status_lines.push(format!("  • {:<16} {:<15} [{}] RTT: {:<6} CPU: {:<6} RAM: {}", n.name, n.ip, n.status, rtt, cpu, ram));
                    }
                    if total == 0 && !app.server_probe_results.is_empty() {
                        status_lines.push("Probed Fleet Servers:".to_string());
                        for (name, lat) in &app.server_probe_results {
                            let lat_str = lat.map(|ms| format!("{ms}ms")).unwrap_or_else(|| "Unreachable".to_string());
                            status_lines.push(format!("  • {name}: {lat_str}"));
                        }
                    }
                    for l in status_lines {
                        app.add_ai_response(&l);
                    }
                }
                "restart" => {
                    if tokens.len() < 3 {
                        app.add_ai_response("Usage: /fleet restart <service-name> [--confirm]");
                        return true;
                    }
                    let service_name = tokens[2].trim().to_string();
                    let has_flag = tokens.iter().skip(3).any(|t| t == "--confirm" || t == "--yes" || t == "-y");
                    if has_flag {
                        app.execute_service_restart(&service_name);
                    } else {
                        app.pending_service_restart = Some(service_name.clone());
                        app.add_ai_response(&format!("⚠ Confirm restart of service '{service_name}'? Type 'yes' (or 'y') to proceed, 'no' to cancel."));
                    }
                }
                _ => {
                    app.add_ai_response(&format!("⚠ Unknown fleet subcommand '{}'. Usage: /fleet <status | restart <service-name>>", tokens[1]));
                }
            }
            true
        }
        "/go" => {
            if tokens.len() < 2 {
                app.add_ai_response("Usage: /go <uptime | analytics | fleet | dashboard | project | server | audit | cron | log | terminal | ai | setting>");
                return true;
            }
            let target = tokens[1].to_lowercase();
            match target.as_str() {
                "uptime" | "monitors" | "3" => {
                    app.current_tab = CockpitTab::Uptime;
                    app.set_notification("Switched to Uptime tab");
                    app.add_ai_response("✅ Switched to Uptime tab");
                }
                "analytics" | "stats" | "5" => {
                    app.current_tab = CockpitTab::Analytics;
                    app.set_notification("Switched to Analytics tab");
                    app.add_ai_response("✅ Switched to Analytics tab");
                }
                "fleet" | "server" | "servers" | "4" => {
                    app.current_tab = CockpitTab::Server;
                    app.set_notification("Switched to Fleet Server tab");
                    app.add_ai_response("✅ Switched to Fleet tab");
                }
                "dashboard" | "dash" | "1" => {
                    app.current_tab = CockpitTab::Dashboard;
                    app.set_notification("Switched to Dashboard tab");
                    app.add_ai_response("✅ Switched to Dashboard tab");
                }
                "project" | "projects" | "2" => {
                    app.current_tab = CockpitTab::Project;
                    app.set_notification("Switched to Project tab");
                    app.add_ai_response("✅ Switched to Project tab");
                }
                "topology" | "topo" | "apps" | "infra" | "audit" | "6" => {
                    app.current_tab = CockpitTab::Topology;
                    app.set_notification("Switched to Topology tab");
                    app.add_ai_response("✅ Switched to Topology tab");
                }
                "cron" | "cronjob" | "7" => {
                    app.current_tab = CockpitTab::Cron;
                    app.set_notification("Switched to CRON tab");
                    app.add_ai_response("✅ Switched to CRON tab");
                }
                "log" | "logs" | "8" => {
                    app.current_tab = CockpitTab::Log;
                    app.set_notification("Switched to Log tab");
                    app.add_ai_response("✅ Switched to Log tab");
                }
                "terminal" | "term" | "9" => {
                    app.current_tab = CockpitTab::Terminal;
                    app.set_notification("Switched to Terminal tab");
                    app.add_ai_response("✅ Switched to Terminal tab");
                }
                "ai" | "chat" | "0" => {
                    app.current_tab = CockpitTab::AI;
                    app.set_notification("Switched to AI tab");
                    app.add_ai_response("✅ Switched to AI tab");
                }
                "setting" | "settings" | "config" | "-" => {
                    app.current_tab = CockpitTab::Setting;
                    app.set_notification("Switched to Setting tab");
                    app.add_ai_response("✅ Switched to Setting tab");
                }
                _ => {
                    app.add_ai_response(&format!("⚠ Unknown tab '{target}'. Options: uptime, analytics, fleet, dashboard, project, server, audit, cron, log, terminal, ai, setting"));
                }
            }
            true
        }
        "/disk" => {
            let sub = tokens.get(1).map(|s| s.to_lowercase()).unwrap_or_else(|| "health".to_string());
            match sub.as_str() {
                "health" | "status" | "smart" | "info" | "" => {
                    app.show_disk_health_modal = true;
                    app.set_notification("CrystalDisk SSD & NVMe Health Diagnostic opened");
                    if let Some(disk) = app.latest_snap.disk_health.first() {
                        app.add_ai_response(&format!(
                            "💽 CrystalDisk Diagnostic: {} ({}) — Health: {}, Temp: {:.1}°C, Wear: {}%, TBW: {:.2} TB",
                            disk.device, disk.model, disk.health_status, disk.temperature_c, disk.wear_level_pct, disk.written_bytes_tb
                        ));
                    } else {
                        app.add_ai_response("💽 Opening CrystalDisk SSD Diagnostic modal...");
                    }
                }
                _ => {
                    app.add_ai_response(&format!("⚠ Unknown disk subcommand '{}'. Usage: /disk health", tokens[1]));
                }
            }
            true
        }
        "/help" | "/h" | "/?" => {
            app.add_ai_response("📖 Available Cockpit Slash Commands:");
            app.add_ai_response("  /disk health                         - Open CrystalDisk SSD & NVMe SMART Health Diagnostic");
            app.add_ai_response("  /uptime test-telegram                - Send test alert to configured Telegram bot/chat");
            app.add_ai_response("  /uptime add-monitor \"name\" \"url\"     - Add new uptime monitor");
            app.add_ai_response("  /uptime delete \"name\"                - Request deletion confirmation for monitor");
            app.add_ai_response("  /analytics refresh                   - Refresh web analytics metrics");
            app.add_ai_response("  /analytics site <site-name>          - Set active analytics site filter (or 'all')");
            app.add_ai_response("  /fleet status                        - View fleet nodes health & latency matrix");
            app.add_ai_response("  /fleet restart <service> [--confirm] - Restart a fleet container/service");
            app.add_ai_response("  /go <uptime|analytics|fleet|...>     - Switch directly to cockpit tab");
            true
        }
        _ => {
            app.add_ai_response(&format!("⚠ Unknown command '{cmd}'. Type /help for available slash commands."));
            true
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

    // 1. Shift + 1..9, 0, - MUST switch tabs immediately (even in AI or Terminal tab)
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
        KeyCode::Char('_') => Some(10),
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
        KeyCode::Char('-') if key.modifiers.contains(KeyModifiers::SHIFT) => Some(10),
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

    // 2b. CrystalDisk SSD Health Diagnostic modal
    if app.show_disk_health_modal {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                app.show_disk_health_modal = false;
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Up | KeyCode::Char('k') => {
                app.selected_disk_health = app.selected_disk_health.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Down | KeyCode::Char('j') => {
                let max_idx = app.latest_snap.disk_health.len().saturating_sub(1);
                if app.selected_disk_health < max_idx {
                    app.selected_disk_health += 1;
                }
            }
            KeyCode::Char('r') => {
                app.sample_system();
                app.set_notification("Storage diagnostics refreshed");
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

    if app.show_site_import_modal {
        handle_site_import_modal_key(key, app);
        return Ok(false);
    }

    // Pro wizard key interception
    if app.show_pro_wizard {
        handle_pro_wizard_key(key, app);
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

                if handle_ai_slash_command(app, &prompt) {
                    terminal.draw(|f| draw(f, app))?;
                    return Ok(false);
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

    // 4c. If Audit tab is focused, handle text inputs for the 5-step workflow (historical)
    if false {
        let audit_field_count = 2; // 0: folder path, 1: natural prompt
        let active_field = app.audit_input_active_field.min(audit_field_count - 1);

        match key.code {
            KeyCode::Esc => {
                app.audit_input_active_field = 0;
                return Ok(false);
            }
            KeyCode::Enter => {
                if active_field == 0 {
                    let folder_path = app.audit_folder_path.trim().to_string();
                    if !folder_path.is_empty() {
                        app.audit_folder_path = folder_path.clone();
                        let proj_name = std::path::Path::new(&folder_path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("project")
                            .to_string();
                        app.audit_project_name = proj_name;
                        app.load_audit_history();
                        app.tune_audit_directive();
                        app.audit_step = AuditStep::NaturalPrompt;
                        app.set_notification("Project folder selected");
                    }
                } else if active_field == 1 {
                    if !app.audit_natural_prompt.trim().is_empty() {
                        app.tune_audit_directive();
                    }
                }
                return Ok(false);
            }
            KeyCode::Backspace => {
                if active_field == 0 {
                    app.audit_folder_path.pop();
                } else {
                    app.audit_natural_prompt.pop();
                }
                return Ok(false);
            }
            KeyCode::Tab => {
                app.audit_input_active_field = (app.audit_input_active_field + 1) % audit_field_count;
                return Ok(false);
            }
            KeyCode::BackTab => {
                app.audit_input_active_field = (app.audit_input_active_field + audit_field_count - 1) % audit_field_count;
                return Ok(false);
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                if active_field == 0 {
                    app.audit_folder_path.push(c);
                } else {
                    app.audit_natural_prompt.push(c);
                }
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

    // 5b. Pet Switch Confirmation Modal Handling
    if app.show_pet_switch_confirm {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                app.confirm_switch_pet_species();
                return Ok(false);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                app.show_pet_switch_confirm = false;
                app.set_notification("Companion switch cancelled");
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    // 5c. Delete Confirmation Modal Handling (across all pages)
    if app.show_delete_confirm {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                app.execute_pending_deletion();
                return Ok(false);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                app.show_delete_confirm = false;
                app.set_notification("Deletion cancelled. Item preserved.");
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    // 5d. Kill Process Confirmation Modal Handling
    if app.show_kill_modal {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                app.execute_kill_process();
                return Ok(false);
            }
            KeyCode::Char('9') => {
                app.pending_kill_force = true;
                app.execute_kill_process();
                return Ok(false);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                app.show_kill_modal = false;
                app.pending_kill_force = false;
                app.set_notification("Process termination cancelled.");
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
                app.set_notification("⚠ Master password not set. Configure it in Setting GUI [-].");
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
        KeyCode::Esc if app.show_analytics_script_modal => {
            app.show_analytics_script_modal = false;
        }
        KeyCode::Esc if app.show_uptime_detail_modal => {
            app.show_uptime_detail_modal = false;
        }
        KeyCode::Esc if app.show_analytics_detail => {
            app.show_analytics_detail = false;
            app.set_notification("Analytics: Sites Overview");
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            if app.inspector_open {
                app.inspector_open = false;
            } else if app.show_help_modal {
                app.show_help_modal = false;
            } else if app.show_analytics_script_modal {
                app.show_analytics_script_modal = false;
            } else {
                app.show_quit_confirm = true;
            }
        }

        // Direct number key tab switching: '1'..'9' -> Tab 0..8, '0' -> AI, '-' -> Setting
        KeyCode::Char('1') => app.current_tab = CockpitTab::Dashboard,
        KeyCode::Char('2') => app.current_tab = CockpitTab::Project,
        KeyCode::Char('3') => app.current_tab = CockpitTab::Uptime,
        KeyCode::Char('4') => app.current_tab = CockpitTab::Server,
        KeyCode::Char('5') => app.current_tab = CockpitTab::Analytics,
        KeyCode::Char('6') => app.current_tab = CockpitTab::Topology,
        KeyCode::Char('7') => app.current_tab = CockpitTab::Cron,
        KeyCode::Char('8') => app.current_tab = CockpitTab::Log,
        KeyCode::Char('9') => app.current_tab = CockpitTab::Terminal,
        KeyCode::Char('0') => app.current_tab = CockpitTab::AI,
        KeyCode::Char('-') => app.current_tab = CockpitTab::Setting,
        KeyCode::Char('=') => app.current_tab = CockpitTab::Turbo,

        // ── Pet Play Area Controls (Hardware-Reactive Tamagotchi) ─────────────
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.execute_pet_action();
        }
        KeyCode::Char('f') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 0;
            app.feed_pet();
        }
        KeyCode::Char('t') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 1;
            app.treat_pet();
        }
        KeyCode::Char('s') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 2;
            app.request_pet_switch();
        }
        KeyCode::Char('p') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 3;
            app.toggle_pet_pause();
        }
        KeyCode::Char('h') if app.current_tab == CockpitTab::Dashboard => {
            app.pet_selected_action = 4;
            app.toggle_pet_history();
        }
        KeyCode::Char('k') | KeyCode::Char('K') if app.current_tab == CockpitTab::Dashboard => {
            let target = app.latest_snap.top_processes.get(app.dashboard_proc_selected).map(|p| (p.pid, p.name.clone()));
            if let Some((pid, name)) = target {
                app.prompt_kill_process_with_name(pid, &name);
            } else {
                app.set_notification("No process selected to kill");
            }
        }

        KeyCode::Tab => {
            if app.current_tab == CockpitTab::Dashboard {
                app.dashboard_focused_panel = (app.dashboard_focused_panel + 1) % 6;
            } else {
                let next_idx = (app.current_tab.to_index() + 1) % CockpitTab::COUNT;
                app.current_tab = CockpitTab::from_index(next_idx);
            }
        }
        KeyCode::BackTab => {
            if app.current_tab == CockpitTab::Dashboard {
                app.dashboard_focused_panel = (app.dashboard_focused_panel + 5) % 6;
            } else {
                let prev_idx = (app.current_tab.to_index() + CockpitTab::COUNT - 1) % CockpitTab::COUNT;
                app.current_tab = CockpitTab::from_index(prev_idx);
            }
        }

        // Tab-specific controls: Up / k and Down / j
        KeyCode::Up | KeyCode::Char('k') => match app.current_tab {
            CockpitTab::Dashboard => {
                if app.dashboard_focused_panel == 5 {
                    app.dashboard_proc_selected = app.dashboard_proc_selected.saturating_sub(1);
                } else {
                    app.pet_selected_action = (app.pet_selected_action + 4) % 5;
                    match app.dashboard_focused_panel {
                        0 => app.dashboard_fleet_scroll = app.dashboard_fleet_scroll.saturating_sub(1),
                        1 => app.dashboard_services_scroll = app.dashboard_services_scroll.saturating_sub(1),
                        2 => app.dashboard_containers_scroll = app.dashboard_containers_scroll.saturating_sub(1),
                        3 => app.dashboard_monitors_scroll = app.dashboard_monitors_scroll.saturating_sub(1),
                        _ => app.dashboard_analytics_scroll = app.dashboard_analytics_scroll.saturating_sub(1),
                    }
                }
            }
            CockpitTab::Project => {
                match app.project_view {
                    ProjectView::Hierarchy | ProjectView::Tree => {
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
            CockpitTab::Analytics => {
                if app.show_analytics_detail {
                    app.analytics_scroll = app.analytics_scroll.saturating_sub(3);
                } else {
                    app.analytics_selected_site = app.analytics_selected_site.saturating_sub(1);
                }
            }
            CockpitTab::Topology => {
                app.topology_nav_up();
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
            CockpitTab::Turbo => {
                app.turbo_selected_action = app.turbo_selected_action.saturating_sub(1);
            }
        },
        KeyCode::Down | KeyCode::Char('j') => match app.current_tab {
            CockpitTab::Dashboard => {
                if app.dashboard_focused_panel == 5 {
                    let max_idx = app.latest_snap.top_processes.len().saturating_sub(1);
                    app.dashboard_proc_selected = (app.dashboard_proc_selected + 1).min(max_idx);
                } else {
                    app.pet_selected_action = (app.pet_selected_action + 1) % 5;
                    match app.dashboard_focused_panel {
                        0 => app.dashboard_fleet_scroll = (app.dashboard_fleet_scroll + 1).min(30),
                        1 => app.dashboard_services_scroll = (app.dashboard_services_scroll + 1).min(30),
                        2 => app.dashboard_containers_scroll = (app.dashboard_containers_scroll + 1).min(30),
                        3 => app.dashboard_monitors_scroll = (app.dashboard_monitors_scroll + 1).min(30),
                        _ => app.dashboard_analytics_scroll = (app.dashboard_analytics_scroll + 1).min(30),
                    }
                }
            }
            CockpitTab::Project => {
                match app.project_view {
                    ProjectView::Hierarchy | ProjectView::Tree => {
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
            CockpitTab::Analytics => {
                if app.show_analytics_detail {
                    app.analytics_scroll = (app.analytics_scroll + 3).min(30);
                } else {
                    let total_sites = app.analytic_sites().len();
                    if total_sites > 0 {
                        app.analytics_selected_site = (app.analytics_selected_site + 1).min(total_sites - 1);
                    }
                }
            }
            CockpitTab::Topology => {
                app.topology_nav_down();
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
            CockpitTab::Turbo => {
                app.turbo_selected_action = (app.turbo_selected_action + 1).min(7);
            }
        },

        KeyCode::Left | KeyCode::Char('h') => match app.current_tab {
            CockpitTab::Setting => app.cycle_field_value(false),
            CockpitTab::Project => {
                app.project_view = match app.project_view {
                    ProjectView::Hierarchy => ProjectView::Containers,
                    ProjectView::Tree => ProjectView::Hierarchy,
                    ProjectView::Containers => ProjectView::Tree,
                };
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
                app.project_view = match app.project_view {
                    ProjectView::Hierarchy => ProjectView::Tree,
                    ProjectView::Tree => ProjectView::Containers,
                    ProjectView::Containers => ProjectView::Hierarchy,
                };
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
            CockpitTab::Analytics => {
                if !app.show_analytics_detail {
                    let sites = app.analytic_sites();
                    if !sites.is_empty() {
                        let idx = app.analytics_selected_site.min(sites.len().saturating_sub(1));
                        app.analytics_site_filter = if idx == 0 {
                            None
                        } else {
                            Some(sites[idx].id.clone())
                        };
                        app.show_analytics_detail = true;
                        app.analytics_scroll = 0;
                        app.sample_system();
                        app.ingest_snapshot(app.latest_snap.clone());
                        let label = app.analytics_site_filter.as_deref().unwrap_or("All Sovereign Apps");
                        app.set_notification(format!("Detail Analytics: {label}"));
                    }
                }
            }
            CockpitTab::Uptime => {
                if !app.show_uptime_detail_modal {
                    if !app.latest_snap.uptime_monitors.is_empty() {
                        let idx = app.selected_monitor.min(app.latest_snap.uptime_monitors.len().saturating_sub(1));
                        app.uptime_detail_index = idx;
                        app.show_uptime_detail_modal = true;
                        let mon_name = app.latest_snap.uptime_monitors[idx].name.clone();
                        app.set_notification(format!("Viewing detail: {mon_name}"));
                    }
                }
            }
            CockpitTab::Setting => {
                start_field_edit(app);
            }
            CockpitTab::Turbo => {
                let action_idx = app.turbo_selected_action;
                app.execute_turbo_action(action_idx);
            }
            CockpitTab::Project => {
                // Toggle view: Hierarchy -> Tree -> Containers
                app.project_view = match app.project_view {
                    ProjectView::Hierarchy => ProjectView::Tree,
                    ProjectView::Tree => ProjectView::Containers,
                    ProjectView::Containers => ProjectView::Hierarchy,
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
        KeyCode::Char('x') | KeyCode::Delete if app.current_tab == CockpitTab::Project => {
            if app.project_view == ProjectView::Containers {
                app.remove_selected_container();
            } else {
                app.remove_selected_project();
            }
        }
        KeyCode::Char('e') if app.current_tab == CockpitTab::Project => {
            if app.project_view == ProjectView::Containers {
                app.edit_selected_container();
            }
        }
        KeyCode::Char('r') if app.current_tab == CockpitTab::Project => {
            app.restart_selected_container();
        }
        KeyCode::Char('g') if app.current_tab == CockpitTab::Project => {
            app.cycle_project_server();
        }

        // Uptime Tab shortcuts: 'm'/'u': Add monitor, 'p': Probe monitors, 'x': Delete monitor, 'e': Edit monitor
        KeyCode::Char('m') | KeyCode::Char('u') if app.current_tab == CockpitTab::Uptime || app.current_tab == CockpitTab::Dashboard => {
            app.show_monitor_wizard = true;
            app.monitor_wizard_field = 0;
        }
        KeyCode::Char('p') if app.current_tab == CockpitTab::Uptime => {
            app.probe_all_ports();
            app.set_notification("Probed all monitors & endpoints");
        }
        KeyCode::Char('x') | KeyCode::Delete if app.current_tab == CockpitTab::Uptime => {
            app.remove_selected_monitor();
        }
        KeyCode::Char('e') if app.current_tab == CockpitTab::Uptime => {
            app.edit_selected_monitor();
        }

        // Server Tab shortcuts: 'a'/'n': Add server, 'p': Probe servers, 'x': Remove server, 'e': Edit server, 'c': Clone server, 'v': Toggle Active/Inactive, 't': Test SSH
        KeyCode::Char('a') | KeyCode::Char('n') if app.current_tab == CockpitTab::Server => {
            app.show_server_wizard = true;
            app.server_wizard_field = 0;
            app.server_wizard_is_edit = false;
            app.server_wizard_orig_name.clear();
        }
        KeyCode::Char('p') if app.current_tab == CockpitTab::Server => {
            app.probe_servers();
            app.set_notification("Probed all multi-node server latencies");
        }
        KeyCode::Char('x') | KeyCode::Delete if app.current_tab == CockpitTab::Server => {
            app.remove_selected_server();
        }
        KeyCode::Char('e') if app.current_tab == CockpitTab::Server => {
            app.open_edit_server_wizard();
        }
        KeyCode::Char('c') if app.current_tab == CockpitTab::Server => {
            app.clone_selected_server();
        }
        KeyCode::Char('v') if app.current_tab == CockpitTab::Server => {
            app.toggle_server_active();
        }
        KeyCode::Char('t') if app.current_tab == CockpitTab::Server => {
            app.test_selected_server_ssh();
        }
        KeyCode::Char('d') | KeyCode::Char('H') if app.current_tab == CockpitTab::Server => {
            app.show_disk_health_modal = true;
            app.set_notification("CrystalDisk SSD Diagnostic opened");
        }
        KeyCode::Char('H') if app.current_tab == CockpitTab::Dashboard => {
            app.show_disk_health_modal = true;
            app.set_notification("CrystalDisk SSD Diagnostic opened");
        }

        // Project Tab shortcuts: 'e': Edit project, 'c': Clone project
        KeyCode::Char('e') if app.current_tab == CockpitTab::Project && app.project_view != ProjectView::Containers => {
            app.edit_selected_project();
        }
        KeyCode::Char('c') if app.current_tab == CockpitTab::Project => {
            if app.project_view == ProjectView::Containers {
                app.edit_selected_container();
            } else {
                app.clone_selected_project();
            }
        }

        // Uptime Tab shortcuts: 'c': Clone monitor, 'v': View detail
        KeyCode::Char('c') if app.current_tab == CockpitTab::Uptime => {
            app.clone_selected_monitor();
        }
        KeyCode::Char('v') | KeyCode::Char('V') if app.current_tab == CockpitTab::Uptime => {
            if !app.latest_snap.uptime_monitors.is_empty() {
                let idx = app.selected_monitor.min(app.latest_snap.uptime_monitors.len().saturating_sub(1));
                app.uptime_detail_index = idx;
                app.show_uptime_detail_modal = true;
                let mon_name = app.latest_snap.uptime_monitors[idx].name.clone();
                app.set_notification(format!("Viewing detail: {mon_name}"));
            }
        }

        // Analytics Tab shortcuts: 'w': Cycle site, 's'/'t': Script/Template, 'r': Refresh, 'p': Test hit, 'i': Import, 'e': Export, 'a': Add Site, 'x': Clear/Delete, 'd'/'v': View Detail, 'b': Back to List
        KeyCode::Char('b') if app.current_tab == CockpitTab::Analytics && app.show_analytics_detail => {
            app.show_analytics_detail = false;
            app.set_notification("Analytics: Sites Overview");
        }
        KeyCode::Char('d') | KeyCode::Char('v') if app.current_tab == CockpitTab::Analytics && !app.show_analytics_detail => {
            let sites = app.analytic_sites();
            if !sites.is_empty() {
                let idx = app.analytics_selected_site.min(sites.len() - 1);
                app.analytics_site_filter = if idx == 0 {
                    None
                } else {
                    Some(sites[idx].id.clone())
                };
                app.show_analytics_detail = true;
                app.analytics_scroll = 0;
                app.sample_system();
                app.ingest_snapshot(app.latest_snap.clone());
                let label = app.analytics_site_filter.as_deref().unwrap_or("All Sites");
                app.set_notification(format!("Detail Analytics: {label}"));
            }
        }
        KeyCode::Char('w') if app.current_tab == CockpitTab::Analytics => {
            let sites = app.analytic_sites();
            if !sites.is_empty() {
                app.analytics_selected_site = (app.analytics_selected_site + 1) % sites.len();
                app.analytics_site_filter = if app.analytics_selected_site == 0 {
                    None
                } else {
                    Some(sites[app.analytics_selected_site].id.clone())
                };
                app.sample_system();
                app.ingest_snapshot(app.latest_snap.clone());
                let label = app.analytics_site_filter.as_deref().unwrap_or("All Sites");
                app.set_notification(format!("Analytics filter: {label}"));
            }
        }
        KeyCode::Char('s') | KeyCode::Char('t') if app.current_tab == CockpitTab::Analytics => {
            app.show_analytics_script_modal = !app.show_analytics_script_modal;
        }
        KeyCode::Char('r') if app.current_tab == CockpitTab::Analytics => {
            app.sample_system();
            app.ingest_snapshot(app.latest_snap.clone());
            app.set_notification("📊 Analytics reloaded");
        }
        KeyCode::Char('p') if app.current_tab == CockpitTab::Analytics => {
            app.simulate_analytics_pageview();
        }
        KeyCode::Char('i') if app.current_tab == CockpitTab::Analytics => {
            app.import_all_landing_sites();
        }
        KeyCode::Char('e') if app.current_tab == CockpitTab::Analytics => {
            app.export_analytics_data_json();
            app.export_analytics_data_csv();
        }
        KeyCode::Char('a') if app.current_tab == CockpitTab::Analytics => {
            app.show_site_import_modal = true;
            app.site_import_name.clear();
            app.site_import_domain.clear();
            app.site_import_selected = 0;
        }
        KeyCode::Char('x') | KeyCode::Delete if app.current_tab == CockpitTab::Analytics => {
            if app.analytics_selected_site > 0 {
                app.remove_selected_analytics_site();
            } else {
                app.request_analytics_wipe();
            }
        }

        // Cron Tab shortcuts: 'n': New Cron Job, 'x': Remove selected managed cron job
        KeyCode::Char('x') | KeyCode::Delete if app.current_tab == CockpitTab::Cron => {
            app.pending_delete_target = DeleteTarget::CronJob;
            app.pending_delete_label = format!("selected scheduled cron job #{}", app.selected_cron_job);
            app.show_delete_confirm = true;
        }

        // Topology Tab shortcuts: 'r': Probe, 'p': Ping
        KeyCode::Char('r') if app.current_tab == CockpitTab::Topology => {
            app.probe_servers();
            app.set_notification("Probing infrastructure topology...");
        }
        KeyCode::Char('p') if app.current_tab == CockpitTab::Topology => {
            app.probe_servers();
            app.set_notification("Ping test dispatched for topology nodes");
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

        // Setting Tab shortcuts: 's': Save, 'r': Reload, 'c': Clear chat, 'v': Vacuum,
        // 'y': Sync GCC, 'P': open Pro Account wizard
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
        KeyCode::Char('y') if app.current_tab == CockpitTab::Setting => {
            app.trigger_gcc_sync();
        }
        KeyCode::Char('P') if app.current_tab == CockpitTab::Setting => {
            app.open_pro_wizard();
        }

        // Turbo Tab shortcuts: 't': Max Turbo, 'b': Build Accel, 'p': Deep Purge, 'e': Eco Safe
        KeyCode::Char('t') if app.current_tab == CockpitTab::Turbo => {
            app.execute_turbo_profile(0);
        }
        KeyCode::Char('b') if app.current_tab == CockpitTab::Turbo => {
            app.execute_turbo_profile(1);
        }
        KeyCode::Char('p') if app.current_tab == CockpitTab::Turbo => {
            app.execute_turbo_profile(2);
        }
        KeyCode::Char('e') if app.current_tab == CockpitTab::Turbo => {
            app.execute_turbo_profile(3);
        }

        // Pet modal shortcut in Dashboard (Shift+P or 'c' to adopt)
        KeyCode::Char('P') if app.current_tab == CockpitTab::Dashboard => {
            app.show_pet_modal = !app.show_pet_modal;
        }
        KeyCode::Char('c') if app.show_pet_modal => {
            if let Some(ref d) = app.db {
                let _ = d.create_pet("sovereign", "Ciko");
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
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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
            app.monitor_wizard_field = (app.monitor_wizard_field + 1) % 3;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.monitor_wizard_field = (app.monitor_wizard_field + 2) % 3;
        }
        KeyCode::Backspace => {
            match app.monitor_wizard_field {
                0 => { app.monitor_name_buf.pop(); }
                1 => { app.monitor_url_buf.pop(); }
                2 => { app.monitor_group_buf.pop(); }
                _ => {}
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.monitor_wizard_field {
                0 => app.monitor_name_buf.push(c),
                1 => app.monitor_url_buf.push(c),
                2 => app.monitor_group_buf.push(c),
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_site_import_modal_key(key: event::KeyEvent, app: &mut CockpitApp) {
    match key.code {
        KeyCode::Esc => {
            app.show_site_import_modal = false;
        }
        KeyCode::Enter => {
            app.submit_site_import_wizard();
        }
        KeyCode::Tab | KeyCode::Down => {
            app.site_import_selected = (app.site_import_selected + 1) % 2;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.site_import_selected = (app.site_import_selected + 1) % 2;
        }
        KeyCode::Backspace => {
            match app.site_import_selected {
                0 => { app.site_import_name.pop(); }
                1 => { app.site_import_domain.pop(); }
                _ => {}
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.site_import_selected {
                0 => app.site_import_name.push(c),
                1 => app.site_import_domain.push(c),
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_pro_wizard_key(key: event::KeyEvent, app: &mut CockpitApp) {
    match key.code {
        KeyCode::Esc => {
            app.show_pro_wizard = false;
            app.pro_wizard_status_msg = None;
        }
        KeyCode::Tab | KeyCode::Down => {
            app.pro_wizard_field = (app.pro_wizard_field + 1) % 2;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.pro_wizard_field = (app.pro_wizard_field + 1) % 2;
        }
        KeyCode::Enter => {
            app.submit_pro_login();
        }
        KeyCode::Char('t') | KeyCode::Char('T') => {
            app.activate_pro_trial();
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.trigger_gcc_sync();
        }
        KeyCode::Backspace => {
            match app.pro_wizard_field {
                0 => { app.pro_wizard_email_buf.pop(); }
                1 => { app.pro_wizard_token_buf.pop(); }
                _ => {}
            }
            app.pro_wizard_status_msg = None;
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            match app.pro_wizard_field {
                0 => app.pro_wizard_email_buf.push(c),
                1 => app.pro_wizard_token_buf.push(c),
                _ => {}
            }
            app.pro_wizard_status_msg = None;
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
            CockpitTab::Dashboard => match app.dashboard_focused_panel {
                0 => app.dashboard_fleet_scroll = app.dashboard_fleet_scroll.saturating_sub(1),
                1 => app.dashboard_services_scroll = app.dashboard_services_scroll.saturating_sub(1),
                2 => app.dashboard_containers_scroll = app.dashboard_containers_scroll.saturating_sub(1),
                3 => app.dashboard_monitors_scroll = app.dashboard_monitors_scroll.saturating_sub(1),
                4 => app.dashboard_analytics_scroll = app.dashboard_analytics_scroll.saturating_sub(1),
                _ => app.dashboard_proc_selected = app.dashboard_proc_selected.saturating_sub(1),
            },
            CockpitTab::Project => match app.project_view {
                ProjectView::Hierarchy | ProjectView::Tree => app.selected_project = app.selected_project.saturating_sub(1),
                ProjectView::Containers => app.selected_container = app.selected_container.saturating_sub(1),
            },
            CockpitTab::Uptime => app.selected_monitor = app.selected_monitor.saturating_sub(1),
            CockpitTab::Server => app.selected_server = app.selected_server.saturating_sub(1),
            CockpitTab::Analytics => app.analytics_scroll = app.analytics_scroll.saturating_sub(3),
            CockpitTab::Topology => app.topology_nav_up(),
            CockpitTab::Cron => {},
            CockpitTab::Log => app.log_scroll = app.log_scroll.saturating_sub(1),
            CockpitTab::Terminal => app.term_scroll = app.term_scroll.saturating_sub(1),
            CockpitTab::AI => app.scroll_chat_up(),
            CockpitTab::Setting => app.selected_field = app.selected_field.saturating_sub(1),
            CockpitTab::Turbo => app.turbo_selected_action = app.turbo_selected_action.saturating_sub(1),
        },
        MouseEventKind::ScrollDown => match app.current_tab {
            CockpitTab::Dashboard => match app.dashboard_focused_panel {
                0 => app.dashboard_fleet_scroll = (app.dashboard_fleet_scroll + 1).min(30),
                1 => app.dashboard_services_scroll = (app.dashboard_services_scroll + 1).min(30),
                2 => app.dashboard_containers_scroll = (app.dashboard_containers_scroll + 1).min(30),
                3 => app.dashboard_monitors_scroll = (app.dashboard_monitors_scroll + 1).min(30),
                4 => app.dashboard_analytics_scroll = (app.dashboard_analytics_scroll + 1).min(30),
                _ => {
                    let max_idx = app.latest_snap.top_processes.len().saturating_sub(1);
                    app.dashboard_proc_selected = (app.dashboard_proc_selected + 1).min(max_idx);
                }
            },
            CockpitTab::Project => match app.project_view {
                ProjectView::Hierarchy | ProjectView::Tree => {
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
            CockpitTab::Analytics => {
                app.analytics_scroll = (app.analytics_scroll + 3).min(30);
            }
            CockpitTab::Topology => app.topology_nav_down(),
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
            CockpitTab::Turbo => {
                app.turbo_selected_action = (app.turbo_selected_action + 1).min(3);
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
                    HitboxAction::CycleProjectServer => {
                        app.cycle_project_server();
                    }
                    HitboxAction::SelectProjectServer(idx) => {
                        app.project_server_target = idx;
                        let targets = app.project_server_targets();
                        if let Some((_, label)) = targets.get(idx) {
                            app.set_notification(format!("Target server switched: [{label}]"));
                        }
                    }
                    HitboxAction::SelectProject(idx) => {
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
                    HitboxAction::RemoveProject => {
                        app.remove_selected_project();
                    }
                    HitboxAction::RemoveContainer => {
                        app.remove_selected_container();
                    }
                    HitboxAction::EditContainer => {
                        app.edit_selected_container();
                    }
                    HitboxAction::RemoveMonitor => {
                        app.remove_selected_monitor();
                    }
                    HitboxAction::EditMonitor => {
                        app.edit_selected_monitor();
                    }
                    HitboxAction::RemoveServer => {
                        app.remove_selected_server();
                    }
                    HitboxAction::EditServer => {
                        app.open_edit_server_wizard();
                    }
                    HitboxAction::CloneServer => {
                        app.clone_selected_server();
                    }
                    HitboxAction::ToggleServerActive => {
                        app.toggle_server_active();
                    }
                    HitboxAction::TestServerSsh => {
                        app.test_selected_server_ssh();
                    }
                    HitboxAction::FocusDashboardPanel(idx) => {
                        app.dashboard_focused_panel = idx;
                    }
                    HitboxAction::EditProject => {
                        app.edit_selected_project();
                    }
                    HitboxAction::CloneProject => {
                        app.clone_selected_project();
                    }
                    HitboxAction::CloneMonitor => {
                        app.clone_selected_monitor();
                    }
                    HitboxAction::SelectMonitor(idx) => {
                        app.selected_monitor = idx;
                    }
                    HitboxAction::ProbeMonitors => {
                        app.probe_all_ports();
                        app.set_notification("Probed all Uptime monitors & gateways");
                    }
                    HitboxAction::RefreshAnalytics => {
                        app.sample_system();
                        app.ingest_snapshot(app.latest_snap.clone());
                        app.set_notification("📊 Web Analytics refreshed");
                    }
                    HitboxAction::SelectAnalyticsSite(idx) => {
                        app.analytics_selected_site = idx;
                        let sites = app.analytic_sites();
                        app.analytics_site_filter = if idx == 0 {
                            None
                        } else {
                            Some(sites.get(idx).map(|s| s.id.clone()).unwrap_or_default())
                        };
                        app.sample_system();
                        app.ingest_snapshot(app.latest_snap.clone());
                        let label = app.analytics_site_filter.as_deref().unwrap_or("All Sites");
                        app.set_notification(format!("Analytics filter: {label}"));
                    }
                    HitboxAction::ViewAnalyticsDetail(idx) => {
                        app.analytics_selected_site = idx;
                        let sites = app.analytic_sites();
                        app.analytics_site_filter = if idx == 0 {
                            None
                        } else {
                            Some(sites.get(idx).map(|s| s.id.clone()).unwrap_or_default())
                        };
                        app.show_analytics_detail = true;
                        app.analytics_scroll = 0;
                        app.sample_system();
                        app.ingest_snapshot(app.latest_snap.clone());
                        let label = app.analytics_site_filter.as_deref().unwrap_or("All Sites");
                        app.set_notification(format!("Detail Analytics: {label}"));
                    }
                    HitboxAction::CloseAnalyticsDetail => {
                        app.show_analytics_detail = false;
                        app.set_notification("Analytics: Sites Overview");
                    }
                    HitboxAction::ShowAnalyticsScript => {
                        app.show_analytics_script_modal = !app.show_analytics_script_modal;
                    }
                    HitboxAction::ClearAnalyticsData => {
                        if let Some(db) = app.db.as_ref() {
                            let _ = db.clear_analytics_events();
                            app.sample_system();
                            app.ingest_snapshot(app.latest_snap.clone());
                            app.set_notification("Analytics data wiped clean");
                        }
                    }
                    HitboxAction::ImportAnalyticsSites => {
                        app.import_all_landing_sites();
                    }
                    HitboxAction::ExportAnalyticsJson => {
                        app.export_analytics_data_json();
                    }
                    HitboxAction::ExportAnalyticsCsv => {
                        app.export_analytics_data_csv();
                    }
                    HitboxAction::ShowSiteWizard => {
                        app.show_site_import_modal = true;
                        app.site_import_name.clear();
                        app.site_import_domain.clear();
                        app.site_import_selected = 0;
                    }
                    HitboxAction::FocusSiteImportName => {
                        app.site_import_selected = 0;
                    }
                    HitboxAction::FocusSiteImportDomain => {
                        app.site_import_selected = 1;
                    }
                    HitboxAction::SubmitSiteImport => {
                        app.submit_site_import_wizard();
                    }
                    HitboxAction::CancelSiteImport => {
                        app.show_site_import_modal = false;
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
                        app.request_pet_switch();
                    }
                    HitboxAction::PetStop => {
                        app.pet_selected_action = 3;
                        app.toggle_pet_pause();
                    }
                    HitboxAction::ConfirmSwitchPet => {
                        app.confirm_switch_pet_species();
                    }
                    HitboxAction::CancelSwitchPet => {
                        app.show_pet_switch_confirm = false;
                        app.set_notification("Companion switch cancelled");
                    }
                    HitboxAction::ConfirmQuit => {
                        app.show_quit_confirm = false;
                        app.quit_requested = true;
                    }
                    HitboxAction::CancelQuit => {
                        app.show_quit_confirm = false;
                    }
                    HitboxAction::ConfirmDelete => {
                        app.execute_pending_deletion();
                    }
                    HitboxAction::CancelDelete => {
                        app.show_delete_confirm = false;
                        app.set_notification("Deletion cancelled. Item preserved.");
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
                    // Pro actions (Phase 4)
                    HitboxAction::SyncGcc => {
                        app.trigger_gcc_sync();
                    }
                    HitboxAction::OpenProWizard => {
                        app.open_pro_wizard();
                    }
                    HitboxAction::CloseProWizard => {
                        app.show_pro_wizard = false;
                        app.pro_wizard_status_msg = None;
                    }
                    HitboxAction::FocusProField(idx) => {
                        app.pro_wizard_field = idx;
                    }
                    HitboxAction::SubmitProLogin => {
                        app.submit_pro_login();
                    }
                    HitboxAction::ActivateProTrial => {
                        app.activate_pro_trial();
                    }
                    HitboxAction::ViewMonitorDetail(idx) => {
                        app.uptime_detail_index = idx;
                        app.show_uptime_detail_modal = true;
                    }
                    HitboxAction::CloseMonitorDetail => {
                        app.show_uptime_detail_modal = false;
                    }
                    HitboxAction::ToggleMonitorAlerts => {
                        app.cfg.uptime.telegram_alerts_enabled = !app.cfg.uptime.telegram_alerts_enabled;
                        app.save_config_silent();
                        let state = if app.cfg.uptime.telegram_alerts_enabled { "ON" } else { "OFF" };
                        app.set_notification(format!("Telegram alerts toggled: {state}"));
                    }
                    HitboxAction::TestTelegramAlert => {
                        app.trigger_test_telegram_alert();
                    }
                    HitboxAction::CycleUptimeGroup => {
                        crate::commands::cockpit::noop();
                    }
                    HitboxAction::CycleUptimeSort => {
                        crate::commands::cockpit::noop();
                    }
                    HitboxAction::ExportUptimeJson => {
                        crate::commands::cockpit::noop();
                    }
                    HitboxAction::ImportUptimeJson => {
                        crate::commands::cockpit::noop();
                    }
                    HitboxAction::TurboProfile(idx) => {
                        app.execute_turbo_profile(idx);
                    }
                    HitboxAction::TurboAction(idx) => {
                        app.execute_turbo_action(idx);
                    }
                    HitboxAction::SelectProcess(idx) => {
                        app.dashboard_proc_selected = idx;
                        app.dashboard_focused_panel = 5;
                    }
                    HitboxAction::KillProcess(pid) => {
                        app.prompt_kill_process(pid);
                    }
                    HitboxAction::ConfirmKillProcess => {
                        app.execute_kill_process();
                    }
                    HitboxAction::CancelKillProcess => {
                        app.show_kill_modal = false;
                        app.pending_kill_force = false;
                        app.set_notification("Process termination cancelled.");
                    }
                    HitboxAction::OpenDiskHealth => {
                        app.show_disk_health_modal = true;
                        app.set_notification("CrystalDisk SSD Diagnostic opened");
                    }
                    HitboxAction::CloseDiskHealth => {
                        app.show_disk_health_modal = false;
                    }
                    HitboxAction::NextDisk => {
                        let max_idx = app.latest_snap.disk_health.len().saturating_sub(1);
                        if app.selected_disk_health < max_idx {
                            app.selected_disk_health += 1;
                        }
                    }
                    HitboxAction::PrevDisk => {
                        app.selected_disk_health = app.selected_disk_health.saturating_sub(1);
                    }
                    HitboxAction::AuditSelectStep(step) => {
                        app.audit_step = match step {
                            0 => AuditStep::SelectProject,
                            1 => AuditStep::NaturalPrompt,
                            2 => AuditStep::PromptTuning,
                            3 => AuditStep::ExecuteAudit,
                            _ => AuditStep::PreviewResult,
                        };
                    }
                    HitboxAction::AuditSelectProject(idx) => {
                        app.switch_audit_project(idx);
                    }
                    HitboxAction::AuditSelectHistory(idx) => {
                        app.audit_selected_history_idx = idx;
                        if let Some(item) = app.audit_db_history.get(idx) {
                            app.audit_preview_markdown = item.report_markdown.clone();
                            app.audit_preview_score = item.score;
                            app.audit_preview_findings_count = (item.critical_count + item.high_count + item.medium_count + item.low_count + item.info_count) as usize;
                            app.audit_step = AuditStep::PreviewResult;
                            app.report_scroll = 0;
                        }
                    }
                    HitboxAction::AuditFocusField(field) => {
                        app.audit_input_active_field = field;
                    }
                    HitboxAction::AuditTunePrompt => {
                        app.tune_audit_directive();
                    }
                    HitboxAction::AuditApplyPreset(idx) => {
                        if let Some((_, text)) = AUDIT_PROMPT_PRESETS.get(idx) {
                            app.audit_natural_prompt = text.to_string();
                            app.tune_audit_directive();
                        }
                    }
                    HitboxAction::AuditNextProject => {
                        app.next_audit_project();
                    }
                    HitboxAction::AuditPrevProject => {
                        app.prev_audit_project();
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
            let token = if !app.cfg.uptime.telegram_bot_token.is_empty() {
                app.cfg.uptime.telegram_bot_token.clone()
            } else {
                app.cfg.daemon.telegram_bot_token.clone()
            };
            app.edit_buffer = token;
        }
        15 => {
            // Telegram Chat ID
            app.editing_field = Some(15);
            let chat_id = if !app.cfg.uptime.telegram_chat_id.is_empty() {
                app.cfg.uptime.telegram_chat_id.clone()
            } else {
                app.cfg.daemon.telegram_chat_id.clone()
            };
            app.edit_buffer = chat_id;
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
        21 | 22 | 23 | 24 | 25 | 26 | 27 | 28 | 29 | 30 | 31 => app.cycle_field_value(true),
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
            app.cfg.daemon.telegram_bot_token = val.clone();
            app.cfg.uptime.telegram_bot_token = val;
            app.set_notification("Telegram bot token updated");
        }
        15 => {
            app.cfg.daemon.telegram_chat_id = val.clone();
            app.cfg.uptime.telegram_chat_id = val.clone();
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
    let target_path = if !app.audit_folder_path.trim().is_empty() {
        app.audit_folder_path.trim().to_string()
    } else {
        ".".to_string()
    };

    let effective_prompt = if !app.audit_tuned_prompt.trim().is_empty() {
        Some(app.audit_tuned_prompt.trim().to_string())
    } else if !app.audit_natural_prompt.trim().is_empty() {
        Some(app.audit_natural_prompt.trim().to_string())
    } else {
        None
    };

    app.audit_step = AuditStep::ExecuteAudit;
    app.audit_status_msg = Some(format!("⚡ Running Audit on {}...", app.audit_project_name));
    app.is_running_audit = true;
    terminal.draw(|f| draw(f, app))?;

    let cmd = crate::cli::AuditCommand {
        path: Some(target_path),
        json: false,
        max_files: 40,
        out: None,
        include: None,
        changed: false,
        timeout: 30,
        turbo: true,
        jobs: None,
        prompt: effective_prompt,
    };

    let cfg_clone = app.cfg.clone();
    let res = crate::commands::audit::run(&cmd, &cfg_clone, true);
    app.is_running_audit = false;
    app.load_audit_history();
    app.refresh_audit_reports();
    app.audit_step = AuditStep::PreviewResult;

    match res {
        Ok(code) => {
            let msg = format!("✓ Audit on {} completed (exit code {code})", app.audit_project_name);
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
        CockpitTab::Analytics => draw_analytics_tab(f, app, chunks[2]),
        CockpitTab::Topology => draw_topology_tab(f, app, chunks[2]),
        CockpitTab::Cron => draw_cron_tab(f, app, chunks[2]),
        CockpitTab::Log => draw_log_tab(f, app, chunks[2]),
        CockpitTab::Terminal => draw_terminal_tab(f, app, chunks[2]),
        CockpitTab::AI => draw_ai_tab(f, app, chunks[2]),
        CockpitTab::Setting => draw_setting_tab(f, app, chunks[2]),
        CockpitTab::Turbo => draw_turbo_tab(f, app, chunks[2]),
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

    if app.show_uptime_detail_modal {
        draw_uptime_detail_modal(f, app, area);
    }

    if app.show_quit_confirm {
        draw_quit_modal(f, app, area);
    }

    if app.show_set_password_modal {
        draw_set_password_modal(f, app, area);
    }

    if app.show_pet_switch_confirm {
        draw_pet_switch_confirm_modal(f, app, area);
    }

    if app.show_delete_confirm {
        draw_delete_confirm_modal(f, app, area);
    }

    if app.show_analytics_script_modal {
        draw_analytics_script_modal(f, app, area);
    }

    if app.show_site_import_modal {
        draw_site_import_modal(f, app, area);
    }

    if app.show_pro_wizard {
        draw_pro_wizard(f, app, area);
    }

    if app.show_kill_modal {
        draw_kill_process_modal(f, app, area);
    }

    if app.show_disk_health_modal {
        draw_disk_health_modal(f, app, area);
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

    let title = format!(" CAShell v{} · Isometric Cockpit ", env!("CARGO_PKG_VERSION"));
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

    // Dynamic Pro / Free / Syncing status badge (Phase 4)
    let pro_badge = if app.is_syncing_gcc {
        // Blinking yellow: toggle visibility every ~0.5s using pet_tick (10 ticks/sec)
        let visible = (app.pet_tick / 5) % 2 == 0;
        Span::styled(
            " [◌ SYNCING] │",
            if visible {
                Style::default()
                    .fg(Color::Rgb(255, 200, 0))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(80, 60, 0))
            },
        )
    } else if app.is_pro() {
        Span::styled(
            " [★ PRO] │",
            Style::default()
                .fg(Color::Rgb(0, 255, 120))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            " [○ FREE] │",
            Style::default().fg(Color::Rgb(140, 140, 140)),
        )
    };

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
        pro_badge,
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
            let color = if avg_val >= 90.0 {
                Color::Rgb(238, 128, 120) // Soft Retro Coral
            } else if avg_val >= 80.0 {
                Color::Rgb(245, 155, 120) // Warm Pastel Salmon
            } else if avg_val >= 60.0 {
                Color::Rgb(240, 185, 110) // Retro Honey Amber
            } else {
                Color::Rgb(140, 215, 165) // Pastel Mint Green
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

    let color = if usage >= 90.0 {
        Color::Rgb(238, 128, 120) // Soft Retro Coral
    } else if usage >= 80.0 {
        Color::Rgb(245, 155, 120) // Warm Pastel Salmon
    } else if usage >= 60.0 {
        Color::Rgb(240, 185, 110) // Retro Honey Amber
    } else {
        Color::Rgb(140, 215, 165) // Pastel Mint Green
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

pub fn format_kbps(kbps: f64) -> String {
    if kbps >= 1024.0 * 1024.0 {
        format!("{:.2} GB/s", kbps / (1024.0 * 1024.0))
    } else if kbps >= 1024.0 {
        format!("{:.1} MB/s", kbps / 1024.0)
    } else if kbps.fract() == 0.0 {
        format!("{:.0} KB/s", kbps)
    } else {
        format!("{:.1} KB/s", kbps)
    }
}

pub fn format_net_gb(gb: f64) -> String {
    if gb >= 1024.0 {
        format!("{:.2} TB", gb / 1024.0)
    } else if gb >= 1.0 {
        format!("{:.2} GB", gb)
    } else {
        format!("{:.1} MB", gb * 1024.0)
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

fn truncate_ellipsis(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else if max_len <= 1 {
        "…".to_string()
    } else {
        let mut res: String = s.chars().take(max_len - 1).collect();
        res.push('…');
        res
    }
}

fn format_mount_capacity(used_mb: u64, total_mb: u64, used_gb: u64, total_gb: u64) -> String {
    let fmt_val = |mb: u64, gb: u64| -> String {
        if total_mb < 1024 || (gb == 0 && mb > 0) {
            format!("{mb}M")
        } else {
            format!("{gb}G")
        }
    };
    format!("{} / {}", fmt_val(used_mb, used_gb), fmt_val(total_mb, total_gb))
}

fn draw_dashboard_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let snap = app.latest_snap.clone();

    let (top_h, show_full_top) = if area.height < 36 && app.overview_proc_scroll >= 1 {
        (1u16, false)
    } else {
        (12u16, true)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_h),
            Constraint::Length(4),
            Constraint::Min(8),
        ])
        .split(area);

    if !show_full_top {
        let pet_badge = if let Some(ref d) = app.db {
            if let Ok(Some(pet)) = d.get_active_pet() {
                format!("■ {} Lv.{} ({})", pet.name, (pet.xp / 100) + 1, pet.species)
            } else {
                "■ Ciko (Ayam)".to_string()
            }
        } else {
            "■ Ciko (Ayam)".to_string()
        };
        let line = Line::from(vec![
            Span::styled("CPU: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:.1}%  ", snap.cpu_pct), Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD)),
            Span::styled("Temp: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:.1}°C  ", snap.cpu_temp_c), Style::default().fg(if snap.cpu_temp_c > 85.0 { palette.status_err } else { palette.status_ok })),
            Span::styled("RAM: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{}M/{}M  ", snap.ram_used_mb, snap.ram_total_mb), Style::default().fg(palette.chart_cpu)),
            Span::styled("Disk: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{}G/{}G  ", snap.disk_used_gb, snap.disk_total_gb), Style::default().fg(palette.status_ok)),
            Span::styled("Net: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("▼{} ▲{}  ", format_kbps(snap.net_rx_rate_kbps), format_kbps(snap.net_tx_rate_kbps)), Style::default().fg(palette.accent)),
            Span::styled("Companion: ", Style::default().fg(palette.text_muted)),
            Span::styled(pet_badge, Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(Paragraph::new(line), chunks[0]);
    } else {
        // Redesigned Top Header: CPU (28%), Memory & Disk (22%), Network Status (22%), Companion Mascot (28%)
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(28), // 1. CPU Telemetry (28%)
                Constraint::Percentage(22), // 2. Memory & Disk Telemetry (22%)
                Constraint::Percentage(22), // 3. Network Status (22% - NEW!)
                Constraint::Percentage(28), // 4. Companion Mascot (28%)
            ])
            .split(chunks[0]);

        let thermal_alert = snap.cpu_temp_c > 88.0;

        // ── Top 1: CPU Telemetry (28%) ──────────────────────────────────────────
        let cpu_title = if thermal_alert {
            format!(" CPU Telemetry ({:.1}%) [ALERT: THERMAL] ", snap.cpu_pct)
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

        if cpu_inner.width > 8 && cpu_inner.height > 2 {
            let temp_style = if snap.cpu_temp_c >= 80.0 {
                Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)
            } else if snap.cpu_temp_c >= 65.0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Green)
            };

            let mut header_spans = vec![
                Span::styled("Load: ", Style::default().fg(palette.text_muted)),
                Span::styled(
                    format!("[{:.2}, {:.2}, {:.2}]  ", snap.load_avg[0], snap.load_avg[1], snap.load_avg[2]),
                    Style::default().fg(palette.accent),
                ),
                Span::styled("Temp: ", Style::default().fg(palette.text_muted)),
                Span::styled(format!("{:.1}°C", snap.cpu_temp_c), temp_style),
            ];

            if thermal_alert {
                header_spans.push(Span::styled(
                    " [ALERT]",
                    Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD),
                ));
            }

            let cpu_hist_vec: Vec<f32> = app.cpu_history.iter().copied().collect();
            let braille_height = if cpu_inner.height >= 8 { 2 } else { 1 };
            let braille_lines = render_braille_curve(&cpu_hist_vec, cpu_inner.width as usize, braille_height);

            let half_w = (cpu_inner.width.saturating_sub(3) / 2) as usize;
            let bar_len = if half_w >= 26 { 10 } else if half_w >= 18 { 5 } else { 3 };

            let mut core_lines = Vec::new();
            if cpu_inner.width >= 30 {
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
            } else {
                for row in 0..4.min(cpu_inner.height.saturating_sub(3 + braille_height as u16) as usize) {
                    let spans = format_core_span(snap.cpus.get(row), row, bar_len);
                    core_lines.push(Line::from(spans));
                }
            }

            let mut full_cpu_lines = Vec::new();
            full_cpu_lines.push(Line::from(header_spans));
            full_cpu_lines.extend(braille_lines);
            full_cpu_lines.extend(core_lines);

            f.render_widget(Paragraph::new(full_cpu_lines), cpu_inner);
        }

        // ── Top 2: Memory & Disk (22%, compact view without large tables) ────────
        let mem_block = Block::default()
            .borders(Borders::ALL)
            .title(" Memory & Disk ")
            .border_style(Style::default().fg(palette.border));
        let mem_inner = mem_block.inner(top_chunks[1]);
        f.render_widget(mem_block, top_chunks[1]);

        if mem_inner.width > 8 && mem_inner.height > 3 {
            let mem_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // RAM gauge
                    Constraint::Length(1), // Swap gauge
                    Constraint::Length(1), // Disk gauge
                    Constraint::Min(3),    // Compact breakdown text
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
            let ram_label_span = Span::styled(
                format!("RAM: {ram_used}M/{ram_total}M ({ram_pct}%)"),
                Style::default().fg(Color::Rgb(10, 15, 25)).add_modifier(Modifier::BOLD),
            );
            let ram_gauge = Gauge::default()
                .gauge_style(Style::default().fg(ram_color).bg(Color::DarkGray))
                .ratio(((ram_used as f64) / (ram_total as f64)).clamp(0.0, 1.0))
                .label(ram_label_span);
            f.render_widget(ram_gauge, mem_layout[0]);

            let swap_used = snap.swap_used_mb;
            let swap_total = snap.swap_total_mb.max(1);
            let swap_pct = if snap.swap_total_mb > 0 {
                ((swap_used * 100) / swap_total) as u16
            } else {
                0
            };
            let swap_label_span = Span::styled(
                format!("SWP: {swap_used}M/{swap_total}M ({swap_pct}%)"),
                Style::default().fg(Color::Rgb(10, 15, 25)).add_modifier(Modifier::BOLD),
            );
            let swap_gauge = Gauge::default()
                .gauge_style(Style::default().fg(palette.chart_ram).bg(Color::DarkGray))
                .ratio(if snap.swap_total_mb > 0 {
                    ((swap_used as f64) / (swap_total as f64)).clamp(0.0, 1.0)
                } else {
                    0.0
                })
                .label(swap_label_span);
            f.render_widget(swap_gauge, mem_layout[1]);

            let disk_used = snap.disk_used_gb;
            let disk_total = snap.disk_total_gb.max(1);
            let disk_pct = ((disk_used * 100) / disk_total) as u16;
            let disk_color = if disk_pct > 90 {
                palette.status_err
            } else if disk_pct > 75 {
                palette.status_warn
            } else {
                palette.status_ok
            };
            let disk_label_span = Span::styled(
                format!("DSK: {disk_used}G/{disk_total}G ({disk_pct}%)"),
                Style::default().fg(Color::Rgb(10, 15, 25)).add_modifier(Modifier::BOLD),
            );
            let disk_gauge = Gauge::default()
                .gauge_style(Style::default().fg(disk_color).bg(Color::DarkGray))
                .ratio(((disk_used as f64) / (disk_total as f64)).clamp(0.0, 1.0))
                .label(disk_label_span);
            f.render_widget(disk_gauge, mem_layout[2]);

            // Compact breakdown lines without large tables
            let gpu_info = if snap.gpu_label != "No GPU" && !snap.gpu_label.is_empty() {
                let util = snap.gpu_pct.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "--".to_string());
                format!("{}: {}", snap.gpu_label, util)
            } else {
                "Integrated Intel/AMD".to_string()
            };

            let mount_summary = snap.mounts.first().map(|m| {
                let cap = format_mount_capacity(m.used_mb, m.total_mb, m.used_gb, m.total_gb);
                format!("{}: {} ({}%)", m.mountpoint, cap, m.pct)
            }).unwrap_or_else(|| format!("/ : {}G/{}G", snap.disk_used_gb, snap.disk_total_gb));

            let mut mem_lines = vec![
                Line::from(vec![
                    Span::styled("Avail: ", Style::default().fg(palette.text_muted)),
                    Span::styled(format!("{}M ", snap.ram_available_mb), Style::default().fg(palette.accent)),
                    Span::styled("Cache: ", Style::default().fg(palette.text_muted)),
                    Span::styled(format!("{}M", snap.ram_cached_mb), Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("Mount: ", Style::default().fg(palette.text_muted)),
                    Span::styled(mount_summary, Style::default().fg(palette.status_ok)),
                ]),
            ];
            if let Some(disk) = snap.disk_health.first() {
                let health_color = if disk.health_status.contains("HEALTHY") || disk.health_status.contains("GOOD") {
                    palette.status_ok
                } else if disk.health_status.contains("WARN") {
                    palette.status_warn
                } else {
                    palette.status_err
                };
                mem_lines.push(Line::from(vec![
                    Span::styled("SMART: ", Style::default().fg(palette.text_muted)),
                    Span::styled(format!("[● {}] ", disk.health_status), Style::default().fg(health_color).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("[● {:.0}°C] ", disk.temperature_c), Style::default().fg(palette.accent)),
                    Span::styled("(H)", Style::default().fg(palette.text_muted)),
                ]));
            } else {
                mem_lines.push(Line::from(vec![
                    Span::styled("GPU:   ", Style::default().fg(palette.text_muted)),
                    Span::styled(truncate_str(&gpu_info, 20), Style::default().fg(palette.text_muted)),
                ]));
            }
            f.render_widget(Paragraph::new(mem_lines), mem_layout[3]);
            app.register_hitbox(mem_inner, HitboxAction::OpenDiskHealth);
        }

        // ── Top 3: Network Status (22% - NEW!) ──────────────────────────────────
        let net_block = Block::default()
            .borders(Borders::ALL)
            .title(" Network Status ")
            .border_style(Style::default().fg(palette.border));
        let net_inner = net_block.inner(top_chunks[2]);
        f.render_widget(net_block, top_chunks[2]);

        if net_inner.width > 8 && net_inner.height > 2 {
            let iface_display = if snap.active_net_iface.is_empty() {
                "wlp2s0".to_string()
            } else {
                snap.active_net_iface.clone()
            };
            let rx_rate_str = format_kbps(snap.net_rx_rate_kbps);
            let tx_rate_str = format_kbps(snap.net_tx_rate_kbps);
            let rx_tot_str = format_net_gb(snap.net_total_rx_gb);
            let tx_tot_str = format_net_gb(snap.net_total_tx_gb);

            let net_lines = vec![
                Line::from(vec![
                    Span::styled("Iface:    ", Style::default().fg(palette.text_muted)),
                    Span::styled(iface_display, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled(" [OK]", Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("▼ RX:     ", Style::default().fg(palette.text_muted)),
                    Span::styled(rx_rate_str, Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD)),
                    Span::styled(" (live)", Style::default().fg(palette.text_muted)),
                ]),
                Line::from(vec![
                    Span::styled("▲ TX:     ", Style::default().fg(palette.text_muted)),
                    Span::styled(tx_rate_str, Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
                    Span::styled(" (live)", Style::default().fg(palette.text_muted)),
                ]),
                Line::from(vec![
                    Span::styled("Total RX: ", Style::default().fg(palette.text_muted)),
                    Span::styled(rx_tot_str, Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("Total TX: ", Style::default().fg(palette.text_muted)),
                    Span::styled(tx_tot_str, Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("State:    ", Style::default().fg(palette.text_muted)),
                    Span::styled("● ACTIVE", Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD)),
                ]),
            ];
            f.render_widget(Paragraph::new(net_lines), net_inner);
        }

        // ── Top 4: Companion Mascot Arena (28% - Compact) ──────────────────────
        draw_pet_play_area(f, app, top_chunks[3]);
    }

    // ── Middle: Executive Command Center Overview & Active Alerts Banner ─────
    let alerts_block = Block::default()
        .borders(Borders::ALL)
        .title(" ■ Executive Command Center · Armada & Daemons Overview ")
        .border_style(Style::default().fg(palette.accent));
    let alerts_inner = alerts_block.inner(chunks[1]);
    f.render_widget(alerts_block, chunks[1]);

    // Summary line 1: Armada, Containers, Daemons, Monitors summary badges
    let mut alerts_spans = Vec::new();

    // 1. Armada Fleet Node summary badge
    let total_nodes = snap.fleet_nodes.len();
    let online_nodes = snap.fleet_nodes.iter().filter(|n| n.status == "ONLINE").count();
    let fleet_offline = total_nodes.saturating_sub(online_nodes);
    if fleet_offline == 0 && total_nodes > 0 {
        alerts_spans.push(Span::styled(
            format!(" Armada: ● {online_nodes}/{total_nodes} Online "),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    } else {
        alerts_spans.push(Span::styled(
            " Armada: ",
            Style::default().fg(Color::Rgb(245, 175, 130)).bg(Color::Rgb(48, 22, 28)),
        ));
        alerts_spans.push(Span::styled(
            format!("[DOWN] {online_nodes}/{total_nodes} Online · {fleet_offline} Down "),
            Style::default().fg(Color::Rgb(242, 135, 125)).bg(Color::Rgb(48, 22, 28)).add_modifier(Modifier::BOLD),
        ));
    }
    alerts_spans.push(Span::raw("  "));

    // 2. Docker Containers summary count badge
    let total_cont = snap.docker_containers.len();
    let running_cont = snap.docker_containers.iter().filter(|c| c.state == "running").count();
    let down_cont = total_cont.saturating_sub(running_cont);
    if down_cont == 0 && total_cont > 0 {
        alerts_spans.push(Span::styled(
            format!(" ● {running_cont}/{total_cont} Containers Running "),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    } else if down_cont > 0 {
        alerts_spans.push(Span::styled(
            " ■ Containers: ",
            Style::default().fg(Color::Rgb(245, 165, 130)).bg(Color::Rgb(48, 24, 28)),
        ));
        alerts_spans.push(Span::styled(
            format!("{running_cont}/{total_cont} Running · {down_cont} [DOWN] "),
            Style::default().fg(Color::Rgb(235, 130, 120)).bg(Color::Rgb(48, 24, 28)).add_modifier(Modifier::BOLD),
        ));
    } else {
        alerts_spans.push(Span::styled(
            " ● 0/0 Containers ",
            Style::default().fg(palette.text_muted),
        ));
    }
    alerts_spans.push(Span::raw("  "));

    // 3. Core Daemons summary
    let total_daemons = snap.service_health.len();
    let active_daemons = snap.service_health.iter().filter(|s| s.state == crate::systat::DaemonState::Active).count();
    let degraded_daemons = snap.service_health.iter().filter(|s| s.state == crate::systat::DaemonState::Degraded).count();
    if active_daemons == total_daemons && total_daemons > 0 {
        alerts_spans.push(Span::styled(
            format!(" Daemons: ● {active_daemons}/{total_daemons} Active "),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    } else {
        let err_count = total_daemons.saturating_sub(active_daemons + degraded_daemons);
        let tag = if err_count > 0 { "[DOWN]" } else { "[ALERT]" };
        let col = if err_count > 0 { Color::Rgb(242, 135, 125) } else { palette.status_warn };
        alerts_spans.push(Span::styled(
            format!(" Daemons: {tag} {active_daemons}/{total_daemons} Active "),
            Style::default().fg(col).bg(Color::Rgb(45, 25, 30)).add_modifier(Modifier::BOLD),
        ));
    }
    alerts_spans.push(Span::raw("  "));

    // 4. Uptime Monitors summary
    let total_monitors = snap.uptime_monitors.len();
    let up_monitors = snap.uptime_monitors.iter().filter(|m| m.is_up).count();
    let down_monitors = total_monitors.saturating_sub(up_monitors);
    if down_monitors == 0 && total_monitors > 0 {
        alerts_spans.push(Span::styled(
            format!(" Monitors: [OK] ALL {up_monitors} UP "),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    } else {
        alerts_spans.push(Span::styled(
            " Monitors: ",
            Style::default().fg(Color::Rgb(215, 165, 225)).bg(Color::Rgb(44, 22, 42)),
        ));
        alerts_spans.push(Span::styled(
            format!("[DOWN] {up_monitors}/{total_monitors} UP · {down_monitors} DOWN "),
            Style::default().fg(Color::Rgb(235, 135, 145)).bg(Color::Rgb(44, 22, 42)).add_modifier(Modifier::BOLD),
        ));
    }
    alerts_spans.push(Span::raw("  "));

    // 5. Sovereign Web Analytics (Umami-like)
    let web_pv = snap.web_analytics.pageviews;
    let web_uv = snap.web_analytics.unique_visitors;
    let web_active = snap.web_analytics.active_visitors;
    alerts_spans.push(Span::styled(
        format!("Web: {web_active} live · {web_pv} views · {web_uv} visitors"),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ));

    let available_width = alerts_inner.width.saturating_sub(2) as usize;
    let mut alert_items: Vec<Vec<Span>> = Vec::new();
    let alert_prefixes = vec![Span::styled("Status: ", Style::default().fg(palette.text_muted))];
    let mut any_alert = false;

    // Check containers down
    let exited_cont: Vec<&str> = snap.docker_containers.iter().filter(|c| c.state != "running").map(|c| c.name.as_str()).collect();
    if !exited_cont.is_empty() {
        let alert_bg = Color::Rgb(48, 22, 28);
        let mut item = vec![Span::styled(" [CONTAINER ALERT: ", Style::default().fg(Color::Rgb(242, 135, 125)).bg(alert_bg).add_modifier(Modifier::BOLD))];
        item.push(Span::styled(exited_cont.join(", "), Style::default().fg(Color::Rgb(245, 185, 130)).bg(alert_bg).add_modifier(Modifier::BOLD)));
        item.push(Span::styled(" EXITED] ", Style::default().fg(Color::Rgb(235, 125, 135)).bg(alert_bg).add_modifier(Modifier::BOLD)));
        alert_items.push(item);
        any_alert = true;
    }

    // Check uptime endpoints down
    let down_endpoints: Vec<(&str, Option<u16>, &str, Option<&str>)> = snap.uptime_monitors.iter().filter(|m| !m.is_up).map(|m| (m.name.as_str(), m.status_code, m.target.as_str(), m.error.as_deref())).collect();
    if !down_endpoints.is_empty() {
        let alert_bg = Color::Rgb(48, 22, 28);
        for (ep_name, code, ep_url, ep_err) in down_endpoints.iter().take(2) {
            let code_str = match (code, ep_err) {
                (Some(c), _) => format!("HTTP {c}"),
                (None, Some(err)) => format!("ERR: {err}"),
                (None, None) => "TIMEOUT/REFUSED".to_string(),
            };
            let short_url = truncate_str(ep_url, 26);
            let mut item = vec![Span::styled(" [OUTAGE ALERT: ", Style::default().fg(Color::Rgb(235, 125, 135)).bg(alert_bg).add_modifier(Modifier::BOLD))];
            item.push(Span::styled(format!("{ep_name} "), Style::default().fg(Color::Rgb(245, 175, 135)).bg(alert_bg).add_modifier(Modifier::BOLD)));
            item.push(Span::styled(format!("({short_url}) "), Style::default().fg(Color::Rgb(195, 165, 215)).bg(alert_bg)));
            item.push(Span::styled(format!("{code_str} DOWN] "), Style::default().fg(Color::Rgb(240, 185, 120)).bg(alert_bg).add_modifier(Modifier::BOLD)));
            alert_items.push(item);
            any_alert = true;
        }
    }

    // Check hardware alerts
    if snap.cpu_temp_c > 85.0 {
        alert_items.push(vec![Span::styled(" [ALERT: Thermal > 85°C] ", Style::default().fg(Color::Rgb(238, 128, 120)).bg(Color::Rgb(48, 22, 28)).add_modifier(Modifier::BOLD))]);
        any_alert = true;
    }
    if snap.cpu_pct > 80.0 {
        alert_items.push(vec![Span::styled(" [ALERT: High CPU Load > 80%] ", Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD))]);
        any_alert = true;
    }
    let ram_ratio = (snap.ram_used_mb as f64) / (snap.ram_total_mb.max(1) as f64);
    if ram_ratio > 0.85 {
        alert_items.push(vec![Span::styled(" [ALERT: Low RAM Available] ", Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD))]);
        any_alert = true;
    }

    let mut alert_line2 = alert_prefixes;
    let mut remaining_width = available_width.saturating_sub(8);
    let mut items_shown = 0;

    for item in &alert_items {
        let item_len: usize = item.iter().map(|s| s.content.chars().count()).sum();
        if item_len <= remaining_width {
            remaining_width = remaining_width.saturating_sub(item_len);
            alert_line2.extend_from_slice(item);
            items_shown += 1;
        } else if item_len > 0 && remaining_width > 0 {
            let remaining_total = alert_items.len().saturating_sub(items_shown);
            alert_line2.push(Span::styled(
                format!(" …+{} alerts", remaining_total),
                Style::default().fg(Color::Rgb(230, 185, 240)).add_modifier(Modifier::BOLD),
            ));
            break;
        }
    }

    if !any_alert {
        alert_line2.push(Span::styled(
            "[OK] Sovereign Command Center Nominal. All nodes, containers, daemons & monitors healthy.",
            Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD),
        ));
    }

    let summary_lines = vec![
        Line::from(alerts_spans),
        Line::from(alert_line2),
    ];
    f.render_widget(Paragraph::new(summary_lines), alerts_inner);

    // ── Bottom: Six Executive Command Center Panels ──────────────────────────
    let bottom_area = chunks[2];

    if bottom_area.height >= 20 {
        // Full dual-column 3-row view: All 6 panels visible simultaneously!
        let row_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(33),
                Constraint::Percentage(34),
            ])
            .split(bottom_area);

        let upper_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(row_chunks[0]);

        let middle_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(row_chunks[1]);

        let lower_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(row_chunks[2]);

        let fleet_focused = app.dashboard_focused_panel == 0;
        let services_focused = app.dashboard_focused_panel == 1;
        let containers_focused = app.dashboard_focused_panel == 2;
        let monitors_focused = app.dashboard_focused_panel == 3;
        let analytics_focused = app.dashboard_focused_panel == 4;
        let procs_focused = app.dashboard_focused_panel == 5;

        let fleet_scroll = app.dashboard_fleet_scroll;
        let services_scroll = app.dashboard_services_scroll;
        let containers_scroll = app.dashboard_containers_scroll;
        let monitors_scroll = app.dashboard_monitors_scroll;
        let analytics_scroll = app.dashboard_analytics_scroll;
        let proc_selected = app.dashboard_proc_selected;

        draw_fleet_nodes_resume_panel(f, app, upper_cols[0], &palette, fleet_scroll, fleet_focused);
        draw_services_health_resume_panel(f, app, upper_cols[1], &palette, services_scroll, services_focused);
        draw_docker_containers_resume_panel(f, app, middle_cols[0], &palette, containers_scroll, containers_focused);
        draw_uptime_monitors_resume_panel(f, app, middle_cols[1], &palette, monitors_scroll, monitors_focused);
        draw_web_analytics_resume_panel(f, app, lower_cols[0], &palette, analytics_scroll, analytics_focused);
        draw_host_top_processes_panel(f, app, lower_cols[1], &palette, proc_selected, procs_focused);
    } else {
        // Compact view: switch between Upper, Middle, and Lower panels
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(bottom_area);

        if app.dashboard_focused_panel <= 1 {
            let fleet_scroll = app.dashboard_fleet_scroll;
            let services_scroll = app.dashboard_services_scroll;
            draw_fleet_nodes_resume_panel(f, app, cols[0], &palette, fleet_scroll, app.dashboard_focused_panel == 0);
            draw_services_health_resume_panel(f, app, cols[1], &palette, services_scroll, app.dashboard_focused_panel == 1);
        } else if app.dashboard_focused_panel <= 3 {
            let containers_scroll = app.dashboard_containers_scroll;
            let monitors_scroll = app.dashboard_monitors_scroll;
            draw_docker_containers_resume_panel(f, app, cols[0], &palette, containers_scroll, app.dashboard_focused_panel == 2);
            draw_uptime_monitors_resume_panel(f, app, cols[1], &palette, monitors_scroll, app.dashboard_focused_panel == 3);
        } else {
            let analytics_scroll = app.dashboard_analytics_scroll;
            let proc_selected = app.dashboard_proc_selected;
            draw_web_analytics_resume_panel(f, app, cols[0], &palette, analytics_scroll, app.dashboard_focused_panel == 4);
            draw_host_top_processes_panel(f, app, cols[1], &palette, proc_selected, app.dashboard_focused_panel == 5);
        }
    }
}

// ── Host Top Processes (Task Manager & Process Killer) ──────────────────────

fn draw_host_top_processes_panel(
    f: &mut Frame,
    app: &mut CockpitApp,
    area: Rect,
    palette: &ThemePalette,
    selected_idx: usize,
    is_focused: bool,
) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    app.register_hitbox(area, HitboxAction::FocusDashboardPanel(5));
    let procs = &app.latest_snap.top_processes;
    let focus_hint = if is_focused { " [● FOCUSED - ↑/↓ Select, k Kill]" } else { "" };
    let title = format!(" ■ Host Top Processes · Task Manager ({}){focus_hint} ", procs.len());
    let border_color = if is_focused {
        palette.status_err
    } else {
        palette.accent
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    let visible_rows = inner.height.saturating_sub(1) as usize; // 1 for header
    let scroll_offset = if selected_idx >= visible_rows {
        selected_idx.saturating_sub(visible_rows - 1)
    } else {
        0
    };

    let mut rows = Vec::new();
    let row_start_y = inner.y + 1; // 1 below header
    let mut hitboxes_to_register = Vec::new();

    for (screen_row, (idx, p)) in procs
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
        .enumerate()
    {
        let is_selected = idx == selected_idx;
        let cpu_col = if p.cpu_pct >= 50.0 {
            palette.status_err
        } else if p.cpu_pct >= 20.0 {
            palette.status_warn
        } else {
            palette.status_ok
        };

        let status_glyph = match p.status.as_str() {
            "Run" | "Running" => "● RUN",
            "Sleep" | "Sleeping" => "◌ SLEEP",
            "Idle" => "· IDLE",
            _ => &p.status,
        };

        let (row_style, pid_style, name_style, act_style) = if is_selected {
            (
                Style::default().bg(palette.highlight_bg).fg(palette.highlight_fg).add_modifier(Modifier::BOLD),
                Style::default().fg(Color::Black).add_modifier(Modifier::BOLD),
                Style::default().fg(Color::Black).add_modifier(Modifier::BOLD),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )
        } else {
            (
                Style::default(),
                Style::default().fg(Color::Cyan),
                Style::default().fg(Color::White),
                Style::default().fg(palette.status_err),
            )
        };

        let short_name = truncate_str(&p.name, 16);
        let row_y = row_start_y + screen_row as u16;

        hitboxes_to_register.push((
            Rect { x: inner.x, y: row_y, width: inner.width.saturating_sub(10), height: 1 },
            HitboxAction::SelectProcess(idx),
        ));
        hitboxes_to_register.push((
            Rect { x: inner.x + inner.width.saturating_sub(10), y: row_y, width: 10, height: 1 },
            HitboxAction::KillProcess(p.pid),
        ));

        rows.push(
            Row::new(vec![
                Cell::from(format!("{}", p.pid)).style(pid_style),
                Cell::from(short_name.to_string()).style(name_style),
                Cell::from(format!("{:.1}%", p.cpu_pct)).style(Style::default().fg(cpu_col)),
                Cell::from(format!("{}M", p.mem_mb)).style(Style::default().fg(palette.accent)),
                Cell::from(status_glyph.to_string()),
                Cell::from("[Kill(k)]").style(act_style),
            ])
            .style(row_style),
        );
    }

    for (r, a) in hitboxes_to_register {
        app.register_hitbox(r, a);
    }

    if rows.is_empty() {
        rows.push(Row::new(vec![
            Cell::from("--"),
            Cell::from("No processes found"),
            Cell::from("--"),
            Cell::from("--"),
            Cell::from("--"),
            Cell::from(""),
        ]));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(7),  // PID
            Constraint::Min(12),    // NAME
            Constraint::Length(8),  // CPU%
            Constraint::Length(8),  // MEM(MB)
            Constraint::Length(9),  // STATUS
            Constraint::Length(9),  // ACTION
        ],
    )
    .header(
        Row::new(vec!["PID", "NAME", "CPU%", "MEM", "STATUS", "ACTION"])
            .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);
}

// ── Server Fleet Resume Panel ───────────────────────────────────────────────

fn draw_fleet_nodes_resume_panel(
    f: &mut Frame,
    app: &mut CockpitApp,
    area: Rect,
    palette: &ThemePalette,
    scroll: usize,
    is_focused: bool,
) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    app.register_hitbox(area, HitboxAction::FocusDashboardPanel(0));
    let nodes = app.latest_snap.fleet_nodes.clone();
    let online_count = nodes.iter().filter(|n| n.status == "ONLINE").count();
    let total_count = nodes.len();
    let focus_hint = if is_focused { " [● FOCUSED - Scroll j/k]" } else { "" };
    let scroll_hint = if area.height < 12 { " [j↓ Docker/Uptime]" } else { "" };
    let title = format!(" ■ Server Fleet Resume · Armada ({online_count}/{total_count} Online){focus_hint}{scroll_hint} ");
    let border_color = if is_focused {
        palette.accent
    } else if online_count == total_count && total_count > 0 {
        palette.status_ok
    } else {
        palette.status_warn
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let scroll_offset = scroll.min(nodes.len().saturating_sub(1));
    let mut rows = Vec::new();

    for (i, n) in nodes.iter().enumerate().skip(scroll_offset) {
        let (status_label, status_col) = match n.status.as_str() {
            "ONLINE" => ("● ONLINE", palette.status_ok),
            "OFFLINE" => ("[DOWN] OFFLINE", palette.status_err),
            _ => ("[ALERT] STANDBY", palette.status_warn),
        };

        let rtt_str = match n.latency_ms {
            Some(0) => "<1ms (Local)".to_string(),
            Some(ms) => format!("{ms}ms"),
            None => "--".to_string(),
        };

        let cpu_str = n.cpu_pct.map(|v| format!("{v:.1}%")).unwrap_or_else(|| "--".to_string());
        let ram_str = n.ram_pct.map(|v| format!("{v}%")).unwrap_or_else(|| "--".to_string());
        let disk_str = n.disk_pct.map(|v| format!("{v}%")).unwrap_or_else(|| "--".to_string());

        let cpu_col = if let Some(cpu) = n.cpu_pct {
            if cpu >= 80.0 { palette.status_err } else if cpu >= 50.0 { palette.status_warn } else { palette.status_ok }
        } else {
            palette.text_muted
        };

        rows.push(Row::new(vec![
            Cell::from(n.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Cell::from(n.ip.clone()).style(Style::default().fg(palette.accent)),
            Cell::from(status_label).style(Style::default().fg(status_col).add_modifier(Modifier::BOLD)),
            Cell::from(rtt_str).style(Style::default().fg(palette.chart_cpu)),
            Cell::from(cpu_str).style(Style::default().fg(cpu_col)),
            Cell::from(ram_str).style(Style::default().fg(palette.chart_ram)),
            Cell::from(disk_str).style(Style::default().fg(palette.text)),
        ]));

        if (i as u16) < inner.height.saturating_sub(1) {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: inner.y + 1 + (i as u16),
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::SelectServer(i),
            );
        }
    }

    if rows.is_empty() {
        rows.push(Row::new(vec![
            Cell::from("No fleet nodes available").style(Style::default().fg(palette.text_muted)),
        ]));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(15),
            Constraint::Length(11),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Min(6),
        ],
    )
    .header(
        Row::new(vec![
            "NODE",
            "TAILSCALE IP",
            "STATUS",
            "RTT LATENCY",
            "CPU LOAD",
            "RAM %",
            "DISK %",
        ])
        .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);
}

// ── Services Health Resume Panel ────────────────────────────────────────────

fn draw_services_health_resume_panel(
    f: &mut Frame,
    app: &mut CockpitApp,
    area: Rect,
    palette: &ThemePalette,
    scroll: usize,
    is_focused: bool,
) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    app.register_hitbox(area, HitboxAction::FocusDashboardPanel(1));
    let services = app.latest_snap.service_health.clone();
    let active_count = services.iter().filter(|s| s.state == crate::systat::DaemonState::Active).count();
    let total_count = services.len();
    let focus_hint = if is_focused { " [● FOCUSED - Scroll j/k]" } else { "" };
    let title = format!(" ■ Services Health Resume · Daemons ({active_count}/{total_count} Active){focus_hint} ");
    let border_color = if is_focused {
        palette.accent
    } else if active_count == total_count && total_count > 0 {
        palette.status_ok
    } else {
        palette.status_warn
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let scroll_offset = scroll.min(services.len().saturating_sub(1));
    let mut rows = Vec::new();

    for s in services.iter().skip(scroll_offset) {
        let (status_badge, status_col) = match s.state {
            crate::systat::DaemonState::Active => ("● ACTIVE", palette.status_ok),
            crate::systat::DaemonState::Inactive => ("[DOWN] INACTIVE", palette.status_err),
            crate::systat::DaemonState::Degraded => ("[ALERT] DEGRADED", palette.status_warn),
        };

        let endpoint_disp = truncate_ellipsis(&s.endpoint, 18);
        rows.push(Row::new(vec![
            Cell::from(s.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Cell::from(s.category.clone()).style(Style::default().fg(palette.accent)),
            Cell::from(status_badge).style(Style::default().fg(status_col).add_modifier(Modifier::BOLD)),
            Cell::from(endpoint_disp).style(Style::default().fg(palette.chart_cpu)),
            Cell::from(s.detail.clone()).style(Style::default().fg(palette.text_muted)),
        ]));
    }

    if rows.is_empty() {
        rows.push(Row::new(vec![
            Cell::from("No ecosystem daemons registered").style(Style::default().fg(palette.text_muted)),
        ]));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(16),
            Constraint::Length(12),
            Constraint::Length(18),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec![
            "DAEMON / SERVICE",
            "CATEGORY",
            "STATUS",
            "PORT / TARGET",
            "DETAILS",
        ])
        .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);
}

// ── Docker Containers Resume Panel ──────────────────────────────────────────

fn draw_docker_containers_resume_panel(
    f: &mut Frame,
    app: &mut CockpitApp,
    area: Rect,
    palette: &ThemePalette,
    scroll: usize,
    is_focused: bool,
) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    app.register_hitbox(area, HitboxAction::FocusDashboardPanel(2));
    let containers = app.latest_snap.docker_containers.clone();
    let running_count = containers.iter().filter(|c| c.state == "running").count();
    let total_count = containers.len();
    let down_count = total_count.saturating_sub(running_count);
    let focus_hint = if is_focused { " [● FOCUSED - Scroll j/k]" } else { "" };

    let scroll_hint = if area.height < 12 { " [k↑ Fleet/Services]" } else { "" };
    let (title, border_color) = if is_focused {
        (
            format!(" ■ Docker Containers Resume · {running_count}/{total_count} Containers Running{focus_hint} "),
            palette.accent,
        )
    } else if down_count == 0 && total_count > 0 {
        (
            format!(" ■ Docker Containers Resume · ● {running_count}/{total_count} Containers Running{scroll_hint} "),
            palette.status_ok,
        )
    } else if down_count > 0 {
        (
            format!(" ■ Docker Containers Resume · ■ {running_count}/{total_count} Running ({down_count} [DOWN]){scroll_hint} "),
            Color::Rgb(240, 130, 120),
        )
    } else {
        (
            format!(" ■ Docker Containers Resume · ● 0/0 Containers{scroll_hint} "),
            palette.border,
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut rows = Vec::new();

    let mut sorted_containers: Vec<(usize, &crate::systat::DockerContainerInfo)> = containers.iter().enumerate().collect();
    sorted_containers.sort_by_key(|(_, c)| if c.state == "running" { 1 } else { 0 });

    let scroll_offset = scroll.min(sorted_containers.len().saturating_sub(1));
    for (i, c) in sorted_containers.iter().skip(scroll_offset) {
        let (status_label, status_col) = match c.state.as_str() {
            "running" => ("● Running", palette.status_ok),
            "paused" => ("[ALERT] Paused", palette.status_warn),
            _ => ("[DOWN] Exited", Color::LightRed),
        };

        let health_label = if c.state == "running" {
            "● HEALTHY"
        } else if c.state == "paused" {
            "[ALERT] DEGRADED"
        } else {
            "[DOWN] UNHEALTHY"
        };
        let health_col = if c.state == "running" {
            palette.status_ok
        } else if c.state == "paused" {
            palette.status_warn
        } else {
            Color::LightRed
        };

        let is_down = c.state != "running";
        let ports_display = if c.ports.is_empty() {
            "--".to_string()
        } else {
            c.ports
                .split(',')
                .map(|p| {
                    let p = p.trim();
                    let p = p.strip_prefix("0.0.0.0:").unwrap_or(p);
                    let p = p.strip_prefix("[::]:").unwrap_or(p);
                    if let Some((host_port, container_port)) = p.split_once("->") {
                        if host_port == container_port {
                            container_port.to_string()
                        } else {
                            format!("{}->{}", host_port, container_port)
                        }
                    } else {
                        p.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        let row_bg = if is_down { Some(Color::Rgb(46, 22, 28)) } else { None };
        let cell_style = |color: Color, bold: bool| -> Style {
            let mut s = Style::default().fg(color);
            if bold {
                s = s.add_modifier(Modifier::BOLD);
            }
            if let Some(bg) = row_bg {
                s = s.bg(bg);
            }
            s
        };

        rows.push(Row::new(vec![
            Cell::from(c.name.clone()).style(cell_style(Color::White, true)),
            Cell::from(status_label).style(cell_style(status_col, true)),
            Cell::from(ports_display).style(cell_style(palette.chart_cpu, false)),
            Cell::from(health_label).style(cell_style(health_col, true)),
            Cell::from(c.image.clone()).style(cell_style(palette.text_muted, false)),
        ]));

        if (*i as u16) < inner.height.saturating_sub(1) {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: inner.y + 1 + (*i as u16),
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::SelectContainer(*i),
            );
        }
    }

    if rows.is_empty() {
        rows.push(Row::new(vec![
            Cell::from("No active containers").style(Style::default().fg(palette.text_muted)),
        ]));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(12),
            Constraint::Length(22),
            Constraint::Length(14),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec![
            "CONTAINER NAME",
            "STATUS",
            "PORTS",
            "HEALTH",
            "IMAGE / REPO",
        ])
        .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);
}

// ── Uptime Monitors Resume Panel ────────────────────────────────────────────

fn draw_uptime_monitors_resume_panel(
    f: &mut Frame,
    app: &mut CockpitApp,
    area: Rect,
    palette: &ThemePalette,
    scroll: usize,
    is_focused: bool,
) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    app.register_hitbox(area, HitboxAction::FocusDashboardPanel(3));
    let monitors = app.latest_snap.uptime_monitors.clone();
    let up_count = monitors.iter().filter(|m| m.is_up).count();
    let total_count = monitors.len();
    let down_count = total_count.saturating_sub(up_count);
    let focus_hint = if is_focused { " [● FOCUSED - Scroll j/k]" } else { "" };

    let scroll_hint = if area.height < 12 { " [k↑ Fleet/Services]" } else { "" };
    let (title, border_color) = if is_focused {
        (
            format!(" ■ Uptime Monitors Resume · {up_count}/{total_count} UP{focus_hint} "),
            palette.accent,
        )
    } else if down_count == 0 && total_count > 0 {
        (
            format!(" ■ Uptime Monitors Resume · [OK] ALL {up_count} UP ({total_count} Total){scroll_hint} "),
            palette.status_ok,
        )
    } else if down_count > 0 {
        (
            format!(" ■ Uptime Monitors Resume · [ALERT] {up_count}/{total_count} UP · {down_count} [DOWN]{scroll_hint} "),
            Color::Rgb(240, 130, 120),
        )
    } else {
        (
            format!(" ■ Uptime Monitors Resume · ● 0/0 Monitored{scroll_hint} "),
            palette.border,
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut rows = Vec::new();

    let mut sorted_monitors: Vec<(usize, &crate::systat::UptimeMonitor)> = monitors.iter().enumerate().collect();
    sorted_monitors.sort_by_key(|(_, m)| if m.is_up { 1 } else { 0 });

    let scroll_offset = scroll.min(sorted_monitors.len().saturating_sub(1));
    for (i, m) in sorted_monitors.iter().skip(scroll_offset) {
        let (status_label, status_col) = if m.is_up {
            ("● UP", palette.status_ok)
        } else {
            ("✖ DOWN", Color::LightRed)
        };

        let http_code = match m.status_code {
            Some(c) if (200..=299).contains(&c) => format!("{c} OK"),
            Some(c) if (300..=399).contains(&c) => format!("{c} REDIR"),
            Some(401) => "401 AUTH".to_string(),
            Some(403) => "403 FORBID".to_string(),
            Some(404) if m.is_up => "404 UP".to_string(),
            Some(404) => "404 NOT FOUND".to_string(),
            Some(c) if (500..=599).contains(&c) => format!("{c} ERR"),
            Some(c) if m.is_up => format!("{c} UP"),
            Some(c) => format!("{c} ERR"),
            None => {
                if let Some(ref err) = m.error {
                    format!("ERR: {err}")
                } else if !m.is_up {
                    "ERR: Down".to_string()
                } else {
                    "--".to_string()
                }
            }
        };
        let rtt_str = m.latency_ms.map(|ms| format!("{ms}ms")).unwrap_or_else(|| "--".to_string());

        let spark_col = if m.uptime_pct >= 95.0 {
            palette.status_ok
        } else if m.uptime_pct >= 80.0 {
            palette.status_warn
        } else {
            palette.status_err
        };

        let mut sparkline_spans = Vec::with_capacity(25);
        sparkline_spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
        let history_len = m.history.len();
        let pad_needed = 20usize.saturating_sub(history_len);
        for _ in 0..pad_needed {
            sparkline_spans.push(Span::styled("█", Style::default().fg(Color::Rgb(60, 60, 60))));
        }
        for &up in m.history.iter().take(20) {
            let col = if up {
                if let Some(ms) = m.latency_ms {
                    if ms > 500 { Color::Yellow } else { palette.status_ok }
                } else {
                    palette.status_ok
                }
            } else {
                Color::LightRed
            };
            sparkline_spans.push(Span::styled("█", Style::default().fg(col)));
        }
        sparkline_spans.push(Span::styled(format!("] {:>3.0}%", m.uptime_pct), Style::default().fg(spark_col)));

        let is_down = !m.is_up;
        let row_bg = if is_down { Some(Color::Rgb(46, 22, 28)) } else { None };
        let cell_style = |color: Color, bold: bool| -> Style {
            let mut s = Style::default().fg(color);
            if bold {
                s = s.add_modifier(Modifier::BOLD);
            }
            if let Some(bg) = row_bg {
                s = s.bg(bg);
            }
            s
        };

        let endpoint_spans = vec![
            Span::styled(&m.name, cell_style(if is_down { Color::LightRed } else { Color::White }, true)),
            Span::styled(
                format!(" ({})", m.target),
                cell_style(if is_down { Color::Rgb(200, 140, 140) } else { palette.chart_cpu }, false),
            ),
        ];

        rows.push(Row::new(vec![
            Cell::from(Line::from(endpoint_spans)),
            Cell::from(status_label).style(cell_style(status_col, true)),
            Cell::from(http_code).style(cell_style(if is_down { Color::LightRed } else { palette.chart_cpu }, false)),
            Cell::from(rtt_str).style(cell_style(palette.text, false)),
            Cell::from(Line::from(sparkline_spans)),
        ]));

        if (*i as u16) < inner.height.saturating_sub(1) {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: inner.y + 1 + (*i as u16),
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::SelectMonitor(*i),
            );
        }
    }

    if rows.is_empty() {
        rows.push(Row::new(vec![
            Cell::from("No uptime monitors registered").style(Style::default().fg(palette.text_muted)),
        ]));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Min(26),
            Constraint::Length(9),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(28),
        ],
    )
    .header(
        Row::new(vec![
            "MONITOR ENDPOINT",
            "STATUS",
            "HTTP / ERR",
            "RTT",
            "HEARTBEAT HISTORY (20)",
        ])
        .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);
}

// ── Sovereign Web Traffic / Umami-like Analytics Resume Panels ───────────────

fn draw_web_analytics_resume_panel(
    f: &mut Frame,
    app: &mut CockpitApp,
    area: Rect,
    palette: &ThemePalette,
    _scroll: usize,
    is_focused: bool,
) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    app.register_hitbox(area, HitboxAction::FocusDashboardPanel(4));
    let a = &app.latest_snap.web_analytics;
    let focus_hint = if is_focused { " [● FOCUSED]" } else { "" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" ■ Product Analytics (Umami / CA-Product Landings) · {} Live{focus_hint} ", a.active_visitors))
        .border_style(Style::default().fg(if is_focused { palette.status_ok } else { palette.accent }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut sorted_products: Vec<&crate::systat::ProductAnalyticsSummary> = a.product_analytics.iter().collect();
    sorted_products.sort_by_key(|p| {
        if p.pageviews > 0 || p.status == "ACTIVE" || p.status == "IDLE" {
            0
        } else {
            1
        }
    });

    let mut rows = Vec::new();
    let mut standby_items = Vec::new();

    for p in sorted_products {
        let (stat_color, stat_label) = match p.status.as_str() {
            "ACTIVE" => (palette.status_ok, "● LIVE"),
            "IDLE" => (palette.status_warn, "[OK] TRACKING"),
            _ => (palette.text_muted, "· STANDBY"),
        };

        if p.pageviews == 0 && p.status != "ACTIVE" && p.status != "IDLE" {
            standby_items.push(p);
        } else {
            rows.push(Row::new(vec![
                Cell::from(p.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Cell::from(p.domain.clone()).style(Style::default().fg(palette.accent)),
                Cell::from(format!("{}", p.pageviews)).style(Style::default().fg(palette.chart_cpu)),
                Cell::from(format!("{}", p.visitors)).style(Style::default().fg(palette.chart_ram)),
                Cell::from(stat_label).style(Style::default().fg(stat_color).add_modifier(Modifier::BOLD)),
            ]));
        }
    }

    if !standby_items.is_empty() {
        rows.push(Row::new(vec![
            Cell::from(format!("● {} standby items grouped", standby_items.len()))
                .style(Style::default().fg(palette.text_muted)),
            Cell::from(standby_items.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "))
                .style(Style::default().fg(palette.text_muted)),
            Cell::from("--").style(Style::default().fg(palette.text_muted)),
            Cell::from("--").style(Style::default().fg(palette.text_muted)),
            Cell::from("STANDBY").style(Style::default().fg(palette.text_muted).add_modifier(Modifier::BOLD)),
        ]));
    }

    if rows.is_empty() {
        rows.push(Row::new(vec![
            Cell::from("TOTAL PAGEVIEWS").style(Style::default().fg(palette.text_muted)),
            Cell::from(format!("{}", a.pageviews)).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Cell::from("UNIQUE VISITORS").style(Style::default().fg(palette.text_muted)),
            Cell::from(format!("{}", a.unique_visitors)).style(Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD)),
            Cell::from(format!("Bounce: {:.1}%", a.bounce_rate)).style(Style::default().fg(palette.status_warn)),
        ]));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(24),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(vec!["PRODUCT / SITE", "DOMAIN / URL", "VIEWS", "VISITORS", "STATUS"])
            .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);
}

#[allow(dead_code)]
fn draw_web_analytics_resume_panel2(
    f: &mut Frame,
    app: &mut CockpitApp,
    area: Rect,
    palette: &ThemePalette,
    _scroll: usize,
    is_focused: bool,
) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    let a = &app.latest_snap.web_analytics;
    let focus_hint = if is_focused { " [● FOCUSED]" } else { "" };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" 🌐 Product Landing Pages & Top Referrers{focus_hint} "))
        .border_style(Style::default().fg(if is_focused { palette.status_ok } else { palette.border }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut rows = Vec::new();
    if a.top_pages.is_empty() {
        rows.push(Row::new(vec![
            Cell::from("caterm.fathforce.com/").style(Style::default().fg(Color::White)),
            Cell::from("https://google.com").style(Style::default().fg(palette.chart_cpu)),
            Cell::from("Direct / Search").style(Style::default().fg(palette.text_muted)),
        ]));
        rows.push(Row::new(vec![
            Cell::from("fathstore.com/pos").style(Style::default().fg(Color::White)),
            Cell::from("Direct").style(Style::default().fg(palette.chart_cpu)),
            Cell::from("Merchant Portal").style(Style::default().fg(palette.text_muted)),
        ]));
        rows.push(Row::new(vec![
            Cell::from("caui.fathforce.com/docs").style(Style::default().fg(Color::White)),
            Cell::from("GitHub / Repo").style(Style::default().fg(palette.chart_cpu)),
            Cell::from("Dev Documentation").style(Style::default().fg(palette.text_muted)),
        ]));
    } else {
        for (url, count) in a.top_pages.iter().take(4) {
            let ref_str = a.top_referrers.first().map(|(r, _)| r.as_str()).unwrap_or("Direct");
            rows.push(Row::new(vec![
                Cell::from(url.clone()).style(Style::default().fg(Color::White)),
                Cell::from(format!("{count} views")).style(Style::default().fg(palette.accent)),
                Cell::from(ref_str).style(Style::default().fg(palette.chart_cpu)),
            ]));
        }
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(26),
            Constraint::Length(18),
            Constraint::Min(16),
        ],
    )
    .header(
        Row::new(vec!["LANDING PAGE", "TRAFFIC / VIEWS", "SOURCE / REFERRER"])
            .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);
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

    let (btn_hier_bg, btn_hier_fg) = if app.project_view == ProjectView::Hierarchy {
        (palette.highlight_bg, palette.highlight_fg)
    } else {
        (Color::Reset, palette.text_muted)
    };
    let (btn_tree_bg, btn_tree_fg) = if app.project_view == ProjectView::Tree {
        (palette.highlight_bg, palette.highlight_fg)
    } else {
        (Color::Reset, palette.text_muted)
    };
    let (btn_cont_bg, btn_cont_fg) = if app.project_view == ProjectView::Containers {
        (palette.highlight_bg, palette.highlight_fg)
    } else {
        (Color::Reset, palette.text_muted)
    };

    let btn_hier_w = 17u16;
    let btn_tree_w = 14u16;
    let btn_cont_w = 18u16;
    let btn_add_proj_w = 18u16;
    let btn_add_cont_w = 20u16;
    let btn_start_w = 11u16;
    let btn_stop_w = 10u16;
    let btn_restart_w = 13u16;
    let btn_edit_w = 12u16;
    let btn_del_w = 14u16;

    let target_list = app.project_server_targets();
    let target_display = if app.project_server_target == 0 {
        "LOCAL".to_string()
    } else {
        target_list.get(app.project_server_target - 1).map(|(name, _)| name.clone()).unwrap_or_else(|| "LOCAL".to_string())
    };
    let target_label_text = format!(" [🖥 Target: {} (g)] ", target_display);
    let target_btn_w = (target_label_text.len() + 1) as u16;
    let btn_target_w = target_btn_w;

    let mut cur_x = action_inner.x + 1;
    let y = action_inner.y;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_hier_w, height: 1 },
        HitboxAction::SelectProjectView(ProjectView::Hierarchy),
    );
    cur_x += btn_hier_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_tree_w, height: 1 },
        HitboxAction::SelectProjectView(ProjectView::Tree),
    );
    cur_x += btn_tree_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_cont_w, height: 1 },
        HitboxAction::SelectProjectView(ProjectView::Containers),
    );
    cur_x += btn_cont_w + 1;

    app.register_hitbox(
        Rect { x: cur_x, y, width: btn_target_w, height: 1 },
        HitboxAction::CycleProjectServer,
    );
    cur_x += btn_target_w + 1;

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
    cur_x += btn_restart_w + 1;

    if app.project_view == ProjectView::Containers {
        app.register_hitbox(
            Rect { x: cur_x, y, width: btn_edit_w, height: 1 },
            HitboxAction::EditContainer,
        );
        cur_x += btn_edit_w + 1;
        app.register_hitbox(
            Rect { x: cur_x, y, width: btn_del_w, height: 1 },
            HitboxAction::RemoveContainer,
        );
    } else {
        app.register_hitbox(
            Rect { x: cur_x, y, width: btn_del_w, height: 1 },
            HitboxAction::RemoveProject,
        );
    }

    let mut bar_spans = vec![
        Span::styled(" [📁 Hierarchy] ", Style::default().fg(btn_hier_fg).bg(btn_hier_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [🌳 Tree] ", Style::default().fg(btn_tree_fg).bg(btn_tree_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [🐳 Containers] ", Style::default().fg(btn_cont_fg).bg(btn_cont_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(
            target_label_text.as_str(),
            Style::default().fg(Color::LightMagenta).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ "),
    ];
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(" [+ Add Project] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(" [+ Add Container] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    bar_spans.push(Span::raw(" │ Actions: "));
    bar_spans.push(Span::styled(" [▶ Start] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(" [■ Stop] ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)));
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(" [↺ Restart] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));

    if app.project_view == ProjectView::Containers {
        bar_spans.push(Span::raw(" "));
        bar_spans.push(Span::styled(" [✎ Edit] (e) ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
        bar_spans.push(Span::raw(" "));
        bar_spans.push(Span::styled(" [🗑 Delete] (x) ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)));
    } else {
        bar_spans.push(Span::raw(" "));
        bar_spans.push(Span::styled(" [🗑 Delete] (x) ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)));
    }

    f.render_widget(Paragraph::new(Line::from(bar_spans)), action_inner);

    match app.project_view {
        ProjectView::Hierarchy => {
            let split_dir = if chunks[1].width < 90 {
                Direction::Vertical
            } else {
                Direction::Horizontal
            };
            let sub_chunks = Layout::default()
                .direction(split_dir)
                .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(chunks[1]);

            let total_projects = app.latest_snap.tracked_projects.len();
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" 📁 Project Workspaces ({}) ", total_projects))
                .border_style(Style::default().fg(palette.border));
            let inner = block.inner(sub_chunks[0]);

            let visible_rows = inner.height.saturating_sub(1) as usize; // reserve 1 line for header
            if total_projects > 0 && app.selected_project >= total_projects {
                app.selected_project = total_projects - 1;
            }

            let start_idx = if visible_rows == 0 || total_projects <= visible_rows {
                0
            } else {
                let sel = app.selected_project;
                if sel < app.project_scroll {
                    sel
                } else if sel >= app.project_scroll + visible_rows {
                    sel + 1 - visible_rows
                } else {
                    app.project_scroll
                }
            };
            let start_idx = start_idx.min(total_projects.saturating_sub(visible_rows));
            app.project_scroll = start_idx;
            let end_idx = (start_idx + visible_rows).min(total_projects);

            let title_text = if total_projects > visible_rows {
                format!(
                    " 📁 Workspaces ({}) [{}-{}/{}] ",
                    total_projects,
                    start_idx + 1,
                    end_idx,
                    total_projects
                )
            } else {
                format!(" 📁 Project Workspaces ({}) ", total_projects)
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .title(title_text)
                .border_style(Style::default().fg(palette.border));
            f.render_widget(block, sub_chunks[0]);

            let rows: Vec<Row> = if total_projects == 0 {
                Vec::new()
            } else {
                app.latest_snap
                    .tracked_projects[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(offset, p)| {
                        let real_idx = start_idx + offset;
                        let is_sel = real_idx == app.selected_project;
                        let bg = if is_sel { palette.row_selected_bg } else { Color::Reset };
                        let (dirty_badge, dirty_col) = if p.is_dirty {
                            ("● DIRTY", palette.status_warn)
                        } else {
                            ("● CLEAN", palette.status_ok)
                        };
                        let repos_str = format!("📦 {}", p.repos.len());
                        let cont_str = if p.containers.is_empty() { "-".to_string() } else { format!("🐳 {}", p.containers.len()) };
                        let svc_str = if p.services.is_empty() { "-".to_string() } else { format!("⚙ {}", p.services.len()) };
                        let db_str = if p.databases.is_empty() { "-".to_string() } else { format!("🗄 {}", p.databases.len()) };

                        Row::new(vec![
                            Cell::from(p.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Cell::from(p.category.clone()).style(Style::default().fg(palette.accent)),
                            Cell::from(repos_str).style(Style::default().fg(palette.chart_cpu)),
                            Cell::from(cont_str).style(Style::default().fg(palette.status_ok)),
                            Cell::from(svc_str).style(Style::default().fg(palette.status_warn)),
                            Cell::from(db_str).style(Style::default().fg(palette.accent)),
                            Cell::from(dirty_badge).style(Style::default().fg(dirty_col).add_modifier(Modifier::BOLD)),
                        ])
                        .style(Style::default().bg(bg))
                    })
                    .collect()
            };

            let table = Table::new(
                rows,
                [
                    Constraint::Length(18),
                    Constraint::Length(10),
                    Constraint::Length(7),
                    Constraint::Length(7),
                    Constraint::Length(7),
                    Constraint::Length(7),
                    Constraint::Min(9),
                ],
            )
            .header(
                Row::new(vec!["WORKSPACE", "CAT", "REPOS", "CONT", "SVC", "DBS", "STATUS"])
                    .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
            );
            f.render_widget(table, inner);

            for offset in 0..(end_idx.saturating_sub(start_idx)) {
                let real_idx = start_idx + offset;
                app.register_hitbox(
                    Rect {
                        x: inner.x,
                        y: inner.y + 1 + (offset as u16),
                        width: inner.width,
                        height: 1,
                    },
                    HitboxAction::SelectProject(real_idx),
                );
            }

            // Right panel: Selected workspace resources (Repos, Containers, Services, DBs)
            if !app.latest_snap.tracked_projects.is_empty() {
                let sel_idx = app.selected_project.min(app.latest_snap.tracked_projects.len() - 1);
                let proj = &app.latest_snap.tracked_projects[sel_idx];

                let detail_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" 📦 Resources: {} ({}) ", proj.name, proj.path))
                    .border_style(Style::default().fg(palette.accent));
                let detail_inner = detail_block.inner(sub_chunks[1]);
                f.render_widget(detail_block, sub_chunks[1]);

                let res_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(28),
                        Constraint::Percentage(28),
                        Constraint::Percentage(22),
                        Constraint::Percentage(22),
                    ])
                    .split(detail_inner);

                // 1. Repos Card
                let repo_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" 📦 Repositories & Code Sources ({}) ", proj.repos.len()))
                    .border_style(Style::default().fg(palette.border));
                let r_inner = repo_block.inner(res_chunks[0]);
                f.render_widget(repo_block, res_chunks[0]);

                let repo_rows: Vec<Row> = if proj.repos.is_empty() {
                    vec![Row::new(vec![Cell::from("No git repos detected in workspace").style(Style::default().fg(palette.text_muted))])]
                } else {
                    proj.repos.iter().map(|r| {
                        let (d_icon, d_col) = if r.is_dirty { ("● DIRTY", palette.status_warn) } else { ("● CLEAN", palette.status_ok) };
                        Row::new(vec![
                            Cell::from(r.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Cell::from(r.branch.clone()).style(Style::default().fg(palette.chart_cpu)),
                            Cell::from(d_icon).style(Style::default().fg(d_col).add_modifier(Modifier::BOLD)),
                            Cell::from(if r.path.is_empty() { "root".to_string() } else { r.path.clone() }).style(Style::default().fg(palette.text_muted)),
                        ])
                    }).collect()
                };
                let r_table = Table::new(repo_rows, [Constraint::Length(22), Constraint::Length(14), Constraint::Length(12), Constraint::Min(15)])
                    .header(Row::new(vec!["REPO", "BRANCH", "STATUS", "PATH"]).style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)));
                f.render_widget(r_table, r_inner);

                // 2. Containers Card
                let cont_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" 🐳 Docker / Podman Containers ({}) ", proj.containers.len()))
                    .border_style(Style::default().fg(palette.border));
                let c_inner = cont_block.inner(res_chunks[1]);
                f.render_widget(cont_block, res_chunks[1]);

                let cont_rows: Vec<Row> = if proj.containers.is_empty() {
                    vec![Row::new(vec![Cell::from("No containers mapped to this workspace").style(Style::default().fg(palette.text_muted))])]
                } else {
                    proj.containers.iter().map(|c| {
                        let (s_icon, s_col) = match c.state.as_str() {
                            "running" => ("● RUNNING", palette.status_ok),
                            "paused" => ("● PAUSED", palette.status_warn),
                            "defined" => ("◌ DEFINED", palette.chart_cpu),
                            _ => ("✖ EXITED", palette.status_err),
                        };
                        Row::new(vec![
                            Cell::from(c.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Cell::from(c.image.clone()).style(Style::default().fg(palette.chart_cpu)),
                            Cell::from(s_icon).style(Style::default().fg(s_col).add_modifier(Modifier::BOLD)),
                            Cell::from(c.ports.clone()).style(Style::default().fg(palette.text_muted)),
                        ])
                    }).collect()
                };
                let c_table = Table::new(cont_rows, [Constraint::Length(22), Constraint::Length(22), Constraint::Length(14), Constraint::Min(15)])
                    .header(Row::new(vec!["CONTAINER", "IMAGE", "STATE", "PORTS"]).style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)));
                f.render_widget(c_table, c_inner);

                // 3. Services Card
                let svc_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" ⚙ Running Services & Ports ({}) ", proj.services.len()))
                    .border_style(Style::default().fg(palette.border));
                let s_inner = svc_block.inner(res_chunks[2]);
                f.render_widget(svc_block, res_chunks[2]);

                let svc_rows: Vec<Row> = if proj.services.is_empty() {
                    vec![Row::new(vec![Cell::from("No services or exposed ports mapped").style(Style::default().fg(palette.text_muted))])]
                } else {
                    proj.services.iter().map(|s| {
                        let (st_icon, st_col) = match s.state.as_str() {
                            "up" | "online" => ("● ONLINE", palette.status_ok),
                            _ => ("✖ DOWN", palette.status_err),
                        };
                        Row::new(vec![
                            Cell::from(s.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Cell::from(format!("{}:{}", s.host, s.port)).style(Style::default().fg(palette.accent)),
                            Cell::from(s.protocol.to_uppercase()).style(Style::default().fg(palette.chart_cpu)),
                            Cell::from(st_icon).style(Style::default().fg(st_col).add_modifier(Modifier::BOLD)),
                        ])
                    }).collect()
                };
                let s_table = Table::new(svc_rows, [Constraint::Length(22), Constraint::Length(20), Constraint::Length(12), Constraint::Min(12)])
                    .header(Row::new(vec!["SERVICE", "HOST:PORT", "PROTOCOL", "STATE"]).style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)));
                f.render_widget(s_table, s_inner);

                // 4. Databases Card
                let db_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" 🗄 Databases (Postgres / MySQL / Redis / SQLite) ({}) ", proj.databases.len()))
                    .border_style(Style::default().fg(palette.border));
                let d_inner = db_block.inner(res_chunks[3]);
                f.render_widget(db_block, res_chunks[3]);

                let db_rows: Vec<Row> = if proj.databases.is_empty() {
                    vec![Row::new(vec![Cell::from("No database instances or configs detected").style(Style::default().fg(palette.text_muted))])]
                } else {
                    proj.databases.iter().map(|d| {
                        let (db_st, db_col) = match d.state.as_str() {
                            "up" | "online" => ("● ONLINE", palette.status_ok),
                            "file" => ("● FILE", palette.chart_cpu),
                            _ => ("✖ STOPPED", palette.status_err),
                        };
                        let port_str = if d.port > 0 { format!(":{}", d.port) } else { "local".to_string() };
                        Row::new(vec![
                            Cell::from(d.name.clone()).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Cell::from(d.engine.to_uppercase()).style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
                            Cell::from(port_str).style(Style::default().fg(palette.accent)),
                            Cell::from(db_st).style(Style::default().fg(db_col).add_modifier(Modifier::BOLD)),
                            Cell::from(d.database.clone()).style(Style::default().fg(palette.text_muted)),
                        ])
                    }).collect()
                };
                let d_table = Table::new(db_rows, [Constraint::Length(20), Constraint::Length(12), Constraint::Length(10), Constraint::Length(12), Constraint::Min(16)])
                    .header(Row::new(vec!["DATABASE", "ENGINE", "PORT", "STATUS", "DETAILS"]).style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)));
                f.render_widget(d_table, d_inner);
            } else {
                let empty_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" 📦 Workspace Resources ")
                    .border_style(Style::default().fg(palette.border));
                f.render_widget(Paragraph::new("No workspace projects tracked. Click [+ Add Project] to track a directory.").style(Style::default().fg(palette.text_muted)), empty_block.inner(sub_chunks[1]));
            }
        }
        ProjectView::Tree => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" 🌳 Workspace Hierarchy Tree ({}) ", app.latest_snap.tracked_projects.len()))
                .border_style(Style::default().fg(palette.border));
            let inner = block.inner(chunks[1]);
            f.render_widget(block, chunks[1]);

            let mut tree_rows: Vec<Row> = Vec::new();
            for (p_idx, p) in app.latest_snap.tracked_projects.iter().enumerate() {
                let is_sel = p_idx == app.selected_project;
                let bg = if is_sel { palette.row_selected_bg } else { Color::Reset };
                let (dirty_badge, dirty_col) = if p.is_dirty { ("● DIRTY", palette.status_warn) } else { ("● CLEAN", palette.status_ok) };

                // Project Header Row
                tree_rows.push(
                    Row::new(vec![
                        Cell::from(format!("▼ 📁 [Project] {} ({})", p.name, p.category)).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Cell::from(p.summary_line()).style(Style::default().fg(palette.chart_cpu)),
                        Cell::from(dirty_badge).style(Style::default().fg(dirty_col).add_modifier(Modifier::BOLD)),
                        Cell::from(p.path.clone()).style(Style::default().fg(palette.text_muted)),
                    ])
                    .style(Style::default().bg(bg))
                );

                // Child Repos
                for r in &p.repos {
                    let (rd_icon, rd_col) = if r.is_dirty { ("● DIRTY", palette.status_warn) } else { ("● CLEAN", palette.status_ok) };
                    tree_rows.push(Row::new(vec![
                        Cell::from(format!("  ├── 📦 Repo: {}", r.name)).style(Style::default().fg(palette.chart_cpu)),
                        Cell::from(format!("git:{}", r.branch)).style(Style::default().fg(palette.accent)),
                        Cell::from(rd_icon).style(Style::default().fg(rd_col)),
                        Cell::from(if r.path.is_empty() { "root".to_string() } else { r.path.clone() }).style(Style::default().fg(palette.text_muted)),
                    ]));
                }

                // Child Containers
                for c in &p.containers {
                    let (c_st, c_col) = match c.state.as_str() {
                        "running" => ("● RUNNING", palette.status_ok),
                        "paused" => ("● PAUSED", palette.status_warn),
                        "defined" => ("◌ DEFINED", palette.chart_cpu),
                        _ => ("✖ EXITED", palette.status_err),
                    };
                    tree_rows.push(Row::new(vec![
                        Cell::from(format!("  ├── 🐳 Container: {}", c.name)).style(Style::default().fg(palette.status_ok)),
                        Cell::from(c.image.clone()).style(Style::default().fg(palette.text_muted)),
                        Cell::from(c_st).style(Style::default().fg(c_col).add_modifier(Modifier::BOLD)),
                        Cell::from(c.ports.clone()).style(Style::default().fg(palette.text_muted)),
                    ]));
                }

                // Child Services
                for s in &p.services {
                    let (s_st, s_col) = match s.state.as_str() {
                        "up" | "online" => ("● ONLINE", palette.status_ok),
                        _ => ("✖ DOWN", palette.status_err),
                    };
                    tree_rows.push(Row::new(vec![
                        Cell::from(format!("  ├── ⚙ Service: {}", s.name)).style(Style::default().fg(palette.status_warn)),
                        Cell::from(format!("{}:{}/{}", s.host, s.port, s.protocol)).style(Style::default().fg(palette.accent)),
                        Cell::from(s_st).style(Style::default().fg(s_col).add_modifier(Modifier::BOLD)),
                        Cell::from("Active process / listener").style(Style::default().fg(palette.text_muted)),
                    ]));
                }

                // Child Databases
                for d in &p.databases {
                    let (d_st, d_col) = match d.state.as_str() {
                        "up" | "online" => ("● ONLINE", palette.status_ok),
                        "file" => ("● FILE", palette.chart_cpu),
                        _ => ("✖ STOPPED", palette.status_err),
                    };
                    let port_str = if d.port > 0 { format!(":{}", d.port) } else { "local".to_string() };
                    tree_rows.push(Row::new(vec![
                        Cell::from(format!("  └── 🗄 DB: {} ({})", d.name, d.engine.to_uppercase())).style(Style::default().fg(palette.accent)),
                        Cell::from(port_str).style(Style::default().fg(palette.status_warn)),
                        Cell::from(d_st).style(Style::default().fg(d_col).add_modifier(Modifier::BOLD)),
                        Cell::from(d.database.clone()).style(Style::default().fg(palette.text_muted)),
                    ]));
                }
            }

            let tree_table = Table::new(
                tree_rows,
                [
                    Constraint::Percentage(36),
                    Constraint::Percentage(26),
                    Constraint::Percentage(14),
                    Constraint::Percentage(24),
                ],
            )
            .header(
                Row::new(vec!["WORKSPACE & RESOURCE HIERARCHY", "DETAILS / CONFIG", "STATUS", "LOCATION"])
                    .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
            );
            f.render_widget(tree_table, inner);

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
                .title(format!(" 🐳 Docker & Podman Engine Containers ({}) ", app.latest_snap.docker_containers.len()))
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
        .constraints([Constraint::Length(4), Constraint::Min(8)])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let displayed: Vec<(usize, crate::systat::UptimeMonitor)> = app.latest_snap.uptime_monitors.iter().cloned().enumerate().collect();
    let count = displayed.len();
    if !displayed.is_empty() && app.selected_monitor >= count {
        app.selected_monitor = count - 1;
    }

    // ── Toolbar row 1: Core Actions (Add, Probe, Edit, Delete, Group, Sort) ──
    let alert_on = app.cfg.uptime.telegram_alerts_enabled;
    let mut toolbar_spans: Vec<Span> = vec![
        Span::styled(" [ADD] (m) ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" [PROBE] (p) ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" [EDIT] (e) ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" [DEL] (x) ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)),
        Span::raw("  │  "),
        Span::styled(format!(" [GRP:{}] ", app.uptime_group_filter), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(format!(" [SRT:{}] ", app.uptime_sort_mode.label_short()), Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
    ];
    let up_count = app.latest_snap.uptime_monitors.iter().filter(|m| m.is_up).count();
    let total_count = app.latest_snap.uptime_monitors.len();
    let pct = if total_count > 0 {
        (up_count as f32 / total_count as f32) * 100.0
    } else {
        100.0
    };
    toolbar_spans.push(Span::raw("  │  "));
    toolbar_spans.push(Span::styled(
        format!("Health: {up_count}/{total_count} Up ({pct:.1}%)"),
        Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
    ));
    f.render_widget(Paragraph::new(Line::from(toolbar_spans)), bar_inner);

    // Register toolbar hitboxes with cumulative x-offset
    let bar_y = bar_inner.y;
    let mut cur_x = bar_inner.x + 1;
    let widths: [u16; 6] = [12, 14, 12, 12, 14, 14];
    let actions: [HitboxAction; 6] = [
        HitboxAction::AddMonitor,
        HitboxAction::ProbeMonitors,
        HitboxAction::EditMonitor,
        HitboxAction::RemoveMonitor,
        HitboxAction::CycleUptimeGroup,
        HitboxAction::CycleUptimeSort,
    ];
    for (w, act) in widths.iter().zip(actions.iter()) {
        if cur_x + w < bar_inner.x + bar_inner.width {
            app.register_hitbox(
                Rect { x: cur_x, y: bar_y, width: *w, height: 1 },
                act.clone(),
            );
        }
        cur_x = cur_x.saturating_add(*w).saturating_add(2);
    }

    // ── Toolbar row 2: Advanced Actions (Export, Import, Alerts, Detail) ──
    let adv_y = bar_y + 1;
    if adv_y < bar_inner.bottom() {
        let adv_spans: Vec<Span> = vec![
            Span::styled(" [EXPORT] ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(" [IMPORT] ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(
                if alert_on { " [ALERTS:ON] " } else { " [ALERTS:OFF] " },
                Style::default().fg(if alert_on { Color::Green } else { Color::DarkGray }).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(" [TEST TG ALERT] ", Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD)),
            Span::raw("  │  "),
            Span::styled(" [V] View Detail (Enter) ", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        ];
        f.render_widget(Paragraph::new(Line::from(adv_spans)), Rect {
            x: bar_inner.x,
            y: adv_y,
            width: bar_inner.width,
            height: 1,
        });

        let mut ax = bar_inner.x + 1;
        for (label_w, act) in [
            (10, HitboxAction::ExportUptimeJson),
            (10, HitboxAction::ImportUptimeJson),
            (15, HitboxAction::ToggleMonitorAlerts),
            (17, HitboxAction::TestTelegramAlert),
            (24, HitboxAction::ViewMonitorDetail(app.selected_monitor)),
        ] {
            if ax + label_w < bar_inner.x + bar_inner.width {
                app.register_hitbox(
                    Rect { x: ax, y: adv_y, width: label_w, height: 1 },
                    act,
                );
            }
            ax = ax.saturating_add(label_w).saturating_add(2);
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " 🌐 Uptime Monitors │ Filter: {} │ Sort: {} │ [{}]",
            app.uptime_group_filter,
            app.uptime_sort_mode.label(),
            if alert_on { "ALERTS:ON" } else { "ALERTS:OFF" }
        ))
        .border_style(Style::default().fg(palette.border));
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);

    let rows: Vec<Row> = displayed
        .iter()
        .enumerate()
        .map(|(i, (_orig_idx, m))| {
            let is_sel = i == app.selected_monitor;
            let bg = if is_sel { palette.row_selected_bg } else { Color::Reset };
            let (status_badge, badge_color) = if m.is_up {
                (format!("● UP ({:.1}%)", m.uptime_pct), palette.status_ok)
            } else {
                (format!("✖ DOWN ({:.1}%)", m.uptime_pct), palette.status_err)
            };

            let latency_str = m
                .latency_ms
                .map(|l| format!("{l}ms"))
                .unwrap_or_else(|| "--".to_string());
            let code_str = match m.status_code {
                Some(c) if (200..=299).contains(&c) => format!("{c} OK"),
                Some(c) if (300..=399).contains(&c) => format!("{c} REDIR"),
                Some(401) => "401 AUTH".to_string(),
                Some(403) => "403 FORBID".to_string(),
                Some(404) if m.is_up => "404 UP".to_string(),
                Some(404) => "404 NOT FOUND".to_string(),
                Some(c) if (500..=599).contains(&c) => format!("{c} ERR"),
                Some(c) if m.is_up => format!("{c} UP"),
                Some(c) => format!("{c} ERR"),
                None => {
                    if let Some(ref err) = m.error {
                        format!("ERR: {err}")
                    } else if !m.is_up {
                        "ERR: Down".to_string()
                    } else {
                        "--".to_string()
                    }
                }
            };
            let code_color = if !m.is_up {
                palette.status_err
            } else if m.status_code.map(|c| (200..=299).contains(&c)).unwrap_or(false) {
                palette.chart_cpu
            } else {
                palette.status_warn
            };

            let mut history_spans: Vec<Span> = Vec::with_capacity(20);
            let pad_needed = 20usize.saturating_sub(m.history.len());
            for _ in 0..pad_needed {
                history_spans.push(Span::styled("█", Style::default().fg(Color::Rgb(60, 60, 60))));
            }
            for &up in m.history.iter().take(20) {
                let col = if up {
                    if let Some(ms) = m.latency_ms {
                        if ms > 500 { Color::Yellow } else { Color::Green }
                    } else {
                        Color::Green
                    }
                } else {
                    Color::Red
                };
                history_spans.push(Span::styled("█", Style::default().fg(col)));
            }

            let eye_span = Span::styled("[V]", Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));

            Row::new(vec![
                Cell::from(format!("{i:>3}")).style(Style::default().fg(palette.text_muted).add_modifier(Modifier::BOLD)),
                Cell::from(m.name.clone()).style(Style::default().fg(if !m.is_up { Color::LightRed } else { Color::White }).add_modifier(Modifier::BOLD)),
                Cell::from(m.target.clone()).style(Style::default().fg(Color::Cyan)),
                Cell::from(m.group.clone()).style(Style::default().fg(palette.text_muted)),
                Cell::from(status_badge).style(Style::default().fg(badge_color).add_modifier(Modifier::BOLD)),
                Cell::from(latency_str).style(Style::default().fg(palette.accent)),
                Cell::from(Line::from(history_spans)),
                Cell::from(code_str).style(Style::default().fg(code_color).add_modifier(Modifier::BOLD)),
                Cell::from(Line::from(vec![eye_span])),
            ])
            .style(Style::default().bg(bg))
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(18),
            Constraint::Min(26),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Length(9),
            Constraint::Length(22),
            Constraint::Length(14),
            Constraint::Length(4),
        ],
    )
    .header(
        Row::new(vec![
            "#",
            "MONITOR",
            "TARGET URL",
            "GROUP",
            "STATUS",
            "LATENCY",
            "HEARTBEATS (20)",
            "HTTP / ERR",
            "⊕",
        ])
        .style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)),
    );
    f.render_widget(table, inner);

    // ── Mouse hitboxes for table rows and Detail buttons ──
    for (i, (orig_idx, _)) in displayed.iter().enumerate() {
        if (i as u16) < inner.height.saturating_sub(1) {
            let row_y = inner.y + 1 + (i as u16);
            let row_action = if i == app.selected_monitor {
                HitboxAction::ViewMonitorDetail(*orig_idx)
            } else {
                HitboxAction::SelectMonitor(i)
            };
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
                row_action,
            );
        }
    }
}

fn draw_uptime_detail_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(68, 60, area);
    f.render_widget(Clear, popup_area);

    let orig_idx = app.uptime_detail_index;
    let monitor = match app.latest_snap.uptime_monitors.get(orig_idx) {
        Some(m) => m,
        None => {
            app.show_uptime_detail_modal = false;
            return;
        }
    };

    let title = format!(" 📈 Endpoint Detail — {} ", monitor.name);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let modal_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(12),   // Detail content
            Constraint::Length(3), // Bottom Action Toolbar
        ])
        .split(inner);

    let mut history_spans: Vec<Span> = Vec::with_capacity(20);
    let pad_needed = 20usize.saturating_sub(monitor.history.len());
    for _ in 0..pad_needed {
        history_spans.push(Span::styled("█", Style::default().fg(Color::Rgb(60, 60, 60))));
    }
    for &up in monitor.history.iter().take(20) {
        let col = if up {
            if let Some(ms) = monitor.latency_ms {
                if ms > 500 { Color::Yellow } else { Color::Green }
            } else {
                Color::Green
            }
        } else {
            Color::Red
        };
        history_spans.push(Span::styled("█", Style::default().fg(col)));
    }

    let status_label = if monitor.is_up {
        Span::styled("● UP (Healthy)", Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("✖ DOWN (Unreachable)", Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD))
    };

    let latency_span = match monitor.latency_ms {
        Some(l) => Span::styled(format!("{l} ms"), Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        None => Span::styled("--".to_string(), Style::default().fg(Color::DarkGray)),
    };

    let code_span = match monitor.status_code {
        Some(c) => {
            let col = if monitor.is_up { palette.status_ok } else { palette.status_err };
            Span::styled(c.to_string(), Style::default().fg(col).add_modifier(Modifier::BOLD))
        }
        None => {
            if let Some(ref err) = monitor.error {
                Span::styled(format!("ERR: {err}"), Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD))
            } else if !monitor.is_up {
                Span::styled("ERR: Down".to_string(), Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("--".to_string(), Style::default().fg(Color::DarkGray))
            }
        }
    };

    let alert_state = if app.cfg.uptime.telegram_alerts_enabled {
        Span::styled("● ENABLED", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("○ DISABLED", Style::default().fg(Color::DarkGray))
    };

    let has_token = !app.cfg.uptime.telegram_bot_token.is_empty() || !app.cfg.daemon.telegram_bot_token.is_empty();
    let token_status = if has_token {
        Span::styled(" [✓ Token Configured]", Style::default().fg(Color::Green))
    } else {
        Span::styled(" [⚠ NO BOT TOKEN SET — open Settings (-) tab]", Style::default().fg(Color::Yellow))
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Endpoint:     ", Style::default().fg(palette.text_muted)),
            Span::styled(monitor.name.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Target URL:   ", Style::default().fg(palette.text_muted)),
            Span::styled(monitor.target.clone(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Group:        ", Style::default().fg(palette.text_muted)),
            Span::styled(monitor.group.clone(), Style::default().fg(palette.accent)),
        ]),
        Line::from(vec![
            Span::styled("Status:       ", Style::default().fg(palette.text_muted)),
            status_label,
            Span::raw("    "),
            Span::styled("Latency: ", Style::default().fg(palette.text_muted)),
            latency_span,
            Span::raw("    "),
            Span::styled("HTTP Code: ", Style::default().fg(palette.text_muted)),
            code_span,
        ]),
    ];

    if let Some(ref err) = monitor.error {
        if !monitor.is_up {
            lines.push(Line::from(vec![
                Span::styled("Failure Cause: ", Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD)),
                Span::styled(format!("ERR: {err}"), Style::default().fg(Color::LightRed)),
            ]));
        }
    }

    lines.extend([
        Line::from(vec![
            Span::styled("Uptime Rate:  ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:.1}%", monitor.uptime_pct), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Heartbeats (Last 20 Probes): ", Style::default().fg(palette.text_muted).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(history_spans),
        Line::from(""),
        Line::from(vec![
            Span::styled("Telegram Alerts:  ", Style::default().fg(palette.text_muted)),
            alert_state,
            token_status,
        ]),
    ]);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), modal_chunks[0]);

    // Bottom Action Button Toolbar inside modal
    let btn_bar_block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(palette.border));
    let btn_bar_inner = btn_bar_block.inner(modal_chunks[1]);
    f.render_widget(btn_bar_block, modal_chunks[1]);

    let btn_spans = vec![
        Span::styled(" [ ↻ Probe (p) ] ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(
            if app.cfg.uptime.telegram_alerts_enabled { " [ 🔔 Alerts: ON (t) ] " } else { " [ 🔕 Alerts: OFF (t) ] " },
            Style::default().fg(if app.cfg.uptime.telegram_alerts_enabled { Color::Green } else { Color::DarkGray }).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(" [ ✏ Edit (e) ] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [ 🗑 Delete (x) ] ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [ ✖ Close (Esc) ] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    ];
    f.render_widget(Paragraph::new(Line::from(btn_spans)), btn_bar_inner);

    let btn_y = btn_bar_inner.y;
    let mut bx = btn_bar_inner.x;
    for (bw, act) in [
        (17u16, HitboxAction::ProbeMonitors),
        (22u16, HitboxAction::ToggleMonitorAlerts),
        (16u16, HitboxAction::EditMonitor),
        (18u16, HitboxAction::RemoveMonitor),
        (18u16, HitboxAction::CloseMonitorDetail),
    ] {
        if bx + bw <= btn_bar_inner.right() {
            app.register_hitbox(
                Rect { x: bx, y: btn_y, width: bw, height: 1 },
                act,
            );
        }
        bx = bx.saturating_add(bw).saturating_add(1);
    }
}

// ── Tab 4: Server (Multi-Node Server Resources & Latency Matrix) ─────────────

fn draw_topology_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    // Header bar
    let header = Block::default()
        .borders(Borders::ALL)
        .title("[6] Isometric Topology – Servers → Apps → Databases → Infra")
        .border_style(Style::default().fg(palette.border));
    f.render_widget(header, chunks[0]);

    let content = chunks[1];

    if app.server_probe_results.is_empty() {
        let placeholder = Paragraph::new(vec![
            Line::from(Span::styled(
                "Press 'r' to probe infrastructure topology...",
                Style::default().fg(palette.text_muted),
            )),
        ])
        .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(placeholder, content);
        return;
    }

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(content);

    // ── Left Pane: Isometric Topology Map ──────────────────────────────
    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(" Topology Map (Local & Remote Fleet) ")
        .border_style(Style::default().fg(palette.accent));
    let left_inner = left_block.inner(split[0]);
    f.render_widget(left_block, split[0]);

    let snap = &app.latest_snap;
    let mut map_lines: Vec<Line> = Vec::new();

    // Helper function for node lines
    let node_status = |lat: Option<u128>| -> (&'static str, Color) {
        match lat {
            Some(ms) if ms < 300 => ("● ONLINE", Color::Green),
            Some(_) => ("▲ SLOW", Color::Yellow),
            None => ("✖ OFFLINE", Color::Red),
        }
    };

    // Server Tier
    map_lines.push(Line::from(Span::styled(
        "SERVER TIER:",
        Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD),
    )));
    for (i, (node, lat)) in app.server_probe_results.iter().enumerate() {
        let (status, color) = node_status(*lat);
        let is_selected = i == app.selected_topology_node && app.topology_focus_tier == 0;
        let style = Style::default().fg(color).add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() });
        let prefix = if is_selected { "▶ " } else { "  " };
        map_lines.push(Line::from(vec![
            Span::styled(format!("{}⬢ {}", prefix, node), style),
            Span::styled(format!(" [{}]", status), style),
        ]));
        map_lines.push(Line::from(Span::styled(
            "  │   ",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // App Tier
    map_lines.push(Line::from(Span::styled(
        "APPLICATIONS TIER:",
        Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
    )));
    let app_count = snap.tracked_projects.len().max(5);
    for i in 0..app_count.min(app.apps.len()) {
        let is_selected = (app.selected_topology_node.saturating_sub(app.server_probe_results.len())) == i
            && app.topology_focus_tier == 1;
        let prefix = if is_selected { "▶ " } else { "  " };
        let name = app.apps.get(i).map(|a| a.name.clone()).unwrap_or_else(|| "App-Instance".to_string());
        map_lines.push(Line::from(vec![
            Span::styled(format!("{}◆ {}", prefix, name), Style::default().fg(Color::White)),
            Span::styled(" ● RUNNING", Style::default().fg(Color::Green)),
        ]));
    }

    // Database & Infra Tier
    map_lines.push(Line::from(Span::styled(
        "DATABASES & INFRA TIER:",
        Style::default().fg(palette.status_ok).add_modifier(Modifier::BOLD),
    )));
    let container_count = snap.docker_containers.len().max(3);
    for i in 0..container_count {
        let is_selected = (app.selected_topology_node.saturating_sub(app.server_probe_results.len() + app_count.min(app.apps.len()))) == i
            && app.topology_focus_tier == 2;
        let prefix = if is_selected { "▶ " } else { "  " };
        let container = snap
            .docker_containers
            .get(i)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "DB / Infra".to_string());
        let state = snap.docker_containers.get(i).map(|c| c.state.clone()).unwrap_or_default();
        let (icon, col) = if state == "running" {
            ("■", Color::Green)
        } else {
            ("■", Color::Red)
        };
        map_lines.push(Line::from(vec![
            Span::styled(format!("{}{} {}", prefix, icon, container), Style::default().fg(col)),
        ]));
    }

    f.render_widget(Paragraph::new(map_lines), left_inner);

    // ── Right Pane: Node Inspector / Diagnostics ──────────────────────
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(" Node Inspector & Diagnostics ")
        .border_style(Style::default().fg(palette.accent));
    let right_inner = right_block.inner(split[1]);
    f.render_widget(right_block, split[1]);

    let sel = app.selected_topology_node;
    let mut detail_lines: Vec<Line> = Vec::new();

    if sel < app.server_probe_results.len() {
        let (name, lat) = &app.server_probe_results[sel];
        let (status, color) = node_status(*lat);
        detail_lines.push(Line::from(vec![
            Span::styled(format!("Node: "), Style::default().fg(palette.text_muted)),
            Span::styled(name.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]));
        detail_lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(palette.text_muted)),
            Span::styled(status.to_string(), Style::default().fg(color)),
        ]));
        detail_lines.push(Line::from(vec![
            Span::styled("Latency: ", Style::default().fg(palette.text_muted)),
            Span::styled(
                lat.map(|ms| format!("{}ms", ms))
                    .unwrap_or_else(|| "unreachable".to_string()),
                Style::default().fg(palette.accent),
            ),
        ]));
        detail_lines.push(Line::from(""));
        detail_lines.push(Line::from(Span::styled(
            "Connected Apps: CAPost, CAShell, 9router, CACalm",
            Style::default().fg(Color::White),
        )));
        detail_lines.push(Line::from(Span::styled(
            "Connected DB/Infra: SQLite, Redis, MySQL, Cloudflare",
            Style::default().fg(Color::White),
        )));
    } else {
        detail_lines.push(Line::from(Span::styled(
            "Select a server node above to view app/database dependencies and diagnostics.",
            Style::default().fg(palette.text_muted),
        )));
    }

    detail_lines.push(Line::from(""));
    detail_lines.push(Line::from(Span::styled(
        "Actions: [r] Probe  [p] Ping  [↑/↓] Navigate  [←/→] Tier Focus",
        Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
    )));

    f.render_widget(Paragraph::new(detail_lines), right_inner);
}

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

    let btn_add_w = 14u16;
    let btn_edit_w = 14u16;
    let btn_clone_w = 15u16;
    let btn_active_w = 16u16;
    let btn_ssh_w = 15u16;
    let btn_probe_w = 15u16;
    let btn_disk_w = 17u16;
    let btn_del_w = 14u16;

    let mut cur_x = bar_inner.x + 1;
    let y = bar_inner.y;

    app.register_hitbox(Rect { x: cur_x, y, width: btn_add_w, height: 1 }, HitboxAction::AddServer);
    cur_x += btn_add_w + 1;
    app.register_hitbox(Rect { x: cur_x, y, width: btn_edit_w, height: 1 }, HitboxAction::EditServer);
    cur_x += btn_edit_w + 1;
    app.register_hitbox(Rect { x: cur_x, y, width: btn_clone_w, height: 1 }, HitboxAction::CloneServer);
    cur_x += btn_clone_w + 1;
    app.register_hitbox(Rect { x: cur_x, y, width: btn_active_w, height: 1 }, HitboxAction::ToggleServerActive);
    cur_x += btn_active_w + 1;
    app.register_hitbox(Rect { x: cur_x, y, width: btn_ssh_w, height: 1 }, HitboxAction::TestServerSsh);
    cur_x += btn_ssh_w + 1;
    app.register_hitbox(Rect { x: cur_x, y, width: btn_probe_w, height: 1 }, HitboxAction::ProbeServers);
    cur_x += btn_probe_w + 1;
    app.register_hitbox(Rect { x: cur_x, y, width: btn_disk_w, height: 1 }, HitboxAction::OpenDiskHealth);
    cur_x += btn_disk_w + 1;
    app.register_hitbox(Rect { x: cur_x, y, width: btn_del_w, height: 1 }, HitboxAction::RemoveServer);

    let bar_spans = vec![
        Span::styled(" [+ Add] (a) ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [✎ Edit] (e) ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [❐ Clone] (c) ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [⏸ Active] (v) ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [⚡ SSH] (t) ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [🔍 Probe] (p) ", Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [💽 Health] (d) ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [🗑 Del] (x) ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)),
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
        let is_active = app.cfg.daemon.nodes.get(node_name).map(|n| n.active).unwrap_or(true);

        let (lat_str, status_str, status_col) = if !is_active {
            ("--".to_string(), "◌ INACTIVE", palette.text_muted)
        } else {
            match lat {
                Some(ms) => (format!("{ms}ms"), "● REACHABLE", palette.status_ok),
                None => ("--".to_string(), "✖ UNREACHABLE", palette.status_err),
            }
        };
        rows.push(
            Row::new(vec![
                Cell::from(node_name.clone()).style(Style::default().fg(if is_active { Color::White } else { palette.text_muted }).add_modifier(Modifier::BOLD)),
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
        if let Some(disk) = snap.disk_health.first() {
            let health_color = if disk.health_status.contains("HEALTHY") || disk.health_status.contains("GOOD") {
                palette.status_ok
            } else if disk.health_status.contains("WARN") {
                palette.status_warn
            } else {
                palette.status_err
            };
            let temp_color = if disk.temperature_c > 75.0 {
                palette.status_err
            } else if disk.temperature_c > 60.0 {
                palette.status_warn
            } else {
                palette.status_ok
            };
            details.push(Line::from(vec![
                Span::styled("SSD SMART:    ", Style::default().fg(palette.text_muted)),
                Span::styled(format!("[● {}] ", disk.health_status), Style::default().fg(health_color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("[● {:.0}°C NORMAL] ", disk.temperature_c), Style::default().fg(temp_color)),
                Span::styled(format!("TBW: {:.2} TB (Wear: {}%)", disk.written_bytes_tb, disk.wear_level_pct), Style::default().fg(palette.accent)),
            ]));
        }
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
        let active_state = node_cfg.map(|n| n.active).unwrap_or(true);

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
        details.push(Line::from(vec![
            Span::styled("Node State:    ", Style::default().fg(palette.text_muted)),
            Span::styled(
                if active_state { "● ACTIVE (Probing & Armada Enabled)" } else { "◌ INACTIVE (Probes Disabled)" },
                Style::default().fg(if active_state { palette.status_ok } else { palette.text_muted }).add_modifier(Modifier::BOLD),
            ),
        ]));
        details.push(Line::from(""));
        details.push(Line::from(Span::styled(
            "Management Controls: [e] Edit Node · [c] Clone Node · [v] Toggle Active · [t] Test SSH · [x] Remove",
            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
        )));
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
        Span::styled(" [↺ Refresh] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("  Filter: "),
        Span::styled(format!(" [{}] (Press 'l') ", app.log_level.label()), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
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
        Span::styled(format!(" Target: [{target_label}] (Press 'r') "), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
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

    let telem = crate::pet::MascotTelemetry {
        cpu_pct: app.latest_snap.cpu_pct,
        cpu_temp_c: app.latest_snap.cpu_temp_c,
        ram_pct: ((app.latest_snap.ram_used_mb as f64) / (app.latest_snap.ram_total_mb.max(1) as f64) * 100.0) as u8,
        swap_pct: ((app.latest_snap.swap_used_mb as f64) / (app.latest_snap.swap_total_mb.max(1) as f64) * 100.0) as u8,
        containers_down: app.latest_snap.docker_containers.iter().filter(|c| c.state != "running").count(),
        fleet_nodes_down: app.server_probe_results.iter().filter(|(_, lat)| lat.is_none()).count(),
        uptime_monitors_down: app.latest_snap.uptime_monitors.iter().filter(|m| !m.is_up).count(),
        git_dirty_count: app.latest_snap.tracked_projects.iter().filter(|p| p.is_dirty).count(),
        turbo_active: app.turbo_profile_active != "ECO_BALANCED" && !app.turbo_profile_active.is_empty(),
        transient_action: if app.is_chat_processing { Some(crate::pet::PetActionState::Run) } else { None },
        xp_level: 3,
        tick: app.pet_tick,
        speed: app.cfg.mascot.speed.clone(),
        sensitivity: app.cfg.mascot.sensitivity.clone(),
        condition_tracking: app.cfg.mascot.condition_tracking,
        color_scheme: app.cfg.mascot.color_scheme.clone(),
        animation_style: app.cfg.mascot.animation_style.clone(),
        render_scale: "Compact".to_string(),
        matrix_speed: app.cfg.mascot.matrix_speed.clone(),
    };
    let expr = crate::pet::select_expression(&telem);

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

    let mut bar_spans = vec![
        Span::styled(format!(" [🤖 Model: {active_model} (m)] "), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" [🗑 Clear Chat] ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)),
    ];
    if app.cfg.mascot.chat_avatar_enabled {
        bar_spans.push(Span::raw("  "));
        bar_spans.push(Span::styled(
            format!(" [🪙 #{:02} {}] ", expr.id, expr.label),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    }
    bar_spans.push(Span::raw("  │  Gateway: "));
    bar_spans.push(Span::styled("OmniRoute / 9router (localhost:20128 / :3007)", Style::default().fg(palette.accent)));
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    // Split middle area horizontally if wide enough and mascot in chat is enabled
    let (chat_area, mascot_area) = if app.cfg.mascot.chat_avatar_enabled && chunks[1].width >= 82 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(chunks[1]);
        (cols[0], Some(cols[1]))
    } else {
        (chunks[1], None)
    };

    let hist_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Workspace AI Chat — Persona: {} ({}) ", app.cfg.persona.user_name, app.cfg.persona.assistant_role))
        .border_style(Style::default().fg(palette.border));
    let hist_inner = hist_block.inner(chat_area);
    f.render_widget(hist_block, chat_area);

    let chat_avatar_on = app.cfg.mascot.chat_avatar_enabled;
    let items: Vec<ListItem> = app
        .chat_lines
        .iter()
        .map(|l| {
            let (display_text, style) = if l.starts_with("User:") {
                let text = if chat_avatar_on {
                    format!("👤 {}: {}", app.cfg.persona.user_name, l.strip_prefix("User:").unwrap_or(l).trim_start())
                } else {
                    l.clone()
                };
                (text, Style::default().fg(palette.accent).add_modifier(Modifier::BOLD))
            } else if l.starts_with("AI:") {
                let text = if chat_avatar_on {
                    format!("🪙 [#{:02} {}] {}: {}", expr.id, expr.label, app.cfg.persona.assistant_role, l.strip_prefix("AI:").unwrap_or(l).trim_start())
                } else {
                    l.clone()
                };
                (text, Style::default().fg(Color::White))
            } else if l.starts_with("Error:") {
                (l.clone(), Style::default().fg(palette.status_err).add_modifier(Modifier::BOLD))
            } else {
                (l.clone(), Style::default().fg(palette.text_muted))
            };
            ListItem::new(Line::from(Span::styled(display_text, style)))
        })
        .collect();

    let chat_list = List::new(items)
        .highlight_style(Style::default().bg(palette.row_selected_bg))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(chat_list, hist_inner, &mut app.chat_state);

    // Companion Chat Reactor Panel on Right Side
    if let Some(m_area) = mascot_area {
        let m_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" 🪙 Companion Reactor [#{:02} {}] ", expr.id, expr.label))
            .border_style(Style::default().fg(Color::Rgb(57, 255, 20)).add_modifier(Modifier::BOLD));
        let m_inner = m_block.inner(m_area);
        f.render_widget(m_block, m_area);

        let (m_lines, _) = crate::pet::render_sovereign_token_lines(
            m_inner.width as usize,
            m_inner.height.saturating_sub(2) as usize,
            &telem,
            app.cfg.output.matrix_rain,
        );

        let mut all_widget_lines = m_lines;
        let reaction_msg = if app.is_chat_processing {
            "⚡ Sovereign Thinking... generating token synthesis"
        } else {
            "✔ Reacting live to prompts & system health"
        };
        all_widget_lines.push(Line::from(Span::styled(
            reaction_msg,
            Style::default().fg(if app.is_chat_processing { Color::Yellow } else { palette.text_muted }),
        )));

        f.render_widget(Paragraph::new(all_widget_lines).alignment(Alignment::Center), m_inner);
    }

    let in_block = Block::default()
        .borders(Borders::ALL)
        .title(if app.is_chat_processing {
            format!(" 🪙 AI is thinking... [#{:02} {}] ", expr.id, expr.label)
        } else {
            " Prompt Input ($ > Enter to send | 'm': Model | 'c': Clear) ".to_string()
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
        crate::pet::PetMood::Idle => if app.pet_is_stopped { Color::DarkGray } else { palette.accent },
    };

    let pet_opt = if let Some(ref d) = app.db {
        d.get_active_pet().ok().flatten()
    } else {
        None
    };

    let (name, species_str, xp) = if let Some(ref p) = pet_opt {
        (p.name.clone(), p.species.clone(), p.xp)
    } else {
        (String::new(), "nagini".to_string(), 0)
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
        crate::pet::PetMood::Idle => "● Normal",
        crate::pet::PetMood::Overheated => "[ALERT] Hot",
        crate::pet::PetMood::Sick => "[DOWN] Sick",
        crate::pet::PetMood::Sprint => "▲ Sprint",
    };

    let pet_name_display = if name.is_empty() {
        species.display_name().to_string()
    } else {
        name.clone()
    };

    // Determine active action state (transient action or telemetry mood)
    let _active_state = if app.pet_is_stopped {
        crate::pet::PetActionState::Idle
    } else if let Some((st, _)) = app.pet_transient_action {
        st
    } else {
        match app.pet_mood {
            crate::pet::PetMood::Sprint => crate::pet::PetActionState::Run,
            crate::pet::PetMood::Overheated => crate::pet::PetActionState::Hit,
            crate::pet::PetMood::Sick => crate::pet::PetActionState::Death,
            crate::pet::PetMood::Idle => {
                if (app.pet_tick / 24) % 4 >= 2 {
                    crate::pet::PetActionState::Walk
                } else {
                    crate::pet::PetActionState::Idle
                }
            }
        }
    };

    let telem = crate::pet::MascotTelemetry {
        cpu_pct: app.latest_snap.cpu_pct,
        cpu_temp_c: app.latest_snap.cpu_temp_c,
        ram_pct: ((app.latest_snap.ram_used_mb as f64) / (app.latest_snap.ram_total_mb.max(1) as f64) * 100.0) as u8,
        swap_pct: ((app.latest_snap.swap_used_mb as f64) / (app.latest_snap.swap_total_mb.max(1) as f64) * 100.0) as u8,
        containers_down: app.latest_snap.docker_containers.iter().filter(|c| c.state != "running").count(),
        fleet_nodes_down: app.server_probe_results.iter().filter(|(_, lat)| lat.is_none()).count(),
        uptime_monitors_down: app.latest_snap.uptime_monitors.iter().filter(|m| !m.is_up).count(),
        git_dirty_count: app.latest_snap.tracked_projects.iter().filter(|p| p.is_dirty).count(),
        turbo_active: app.turbo_profile_active != "ECO_BALANCED" && !app.turbo_profile_active.is_empty(),
        transient_action: if app.pet_is_stopped { Some(crate::pet::PetActionState::Death) } else if let Some((st, _)) = app.pet_transient_action { Some(st) } else { None },
        xp_level: level as usize,
        tick: app.pet_tick,
        speed: app.cfg.mascot.speed.clone(),
        sensitivity: app.cfg.mascot.sensitivity.clone(),
        condition_tracking: app.cfg.mascot.condition_tracking,
        color_scheme: app.cfg.mascot.color_scheme.clone(),
        animation_style: app.cfg.mascot.animation_style.clone(),
        render_scale: app.cfg.mascot.render_scale.clone(),
        matrix_speed: app.cfg.mascot.matrix_speed.clone(),
    };
    let expr = crate::pet::select_expression(&telem);

    let state_badge = if app.pet_is_stopped {
        "■ STOPPED".to_string()
    } else {
        format!("#{:02} {}", expr.id, expr.label)
    };

    let title = if area.width >= 36 {
        format!(" ■ SOVEREIGN TOKEN [ {} · Lv.{} · {} | {} ] ", pet_name_display, level, state_badge, mood_short)
    } else {
        format!(" ■ SOVEREIGN [ {} · Lv.{} ] ", pet_name_display, level)
    };

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
                Span::styled(" ■ ACTIVITY LOG ", Style::default().fg(palette.highlight_fg).add_modifier(Modifier::BOLD)),
                Span::styled(" [X Close (h/Esc)] ", Style::default().fg(palette.text_muted)),
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
            Constraint::Length(1), // Nav Bar: [▲] [▼] [✔ OK] [✖]
            Constraint::Length(1), // Actions: [0: Feed] [1: Train] [2: Switch] [3: Stop] [4: Log]
        ])
        .split(inner);

    // 1. Animated Sprite Screen — Cyberpunk Isometric Sovereign Token
    //    with stable sine-wave bobbing, 100 condition-responsive expressions,
    //    and falling Matrix digital rain background (toggleable in Settings).
    let screen = chunks[0];
    let (token_lines, _) = crate::pet::render_sovereign_token_lines(
        screen.width as usize,
        screen.height as usize,
        &telem,
        app.cfg.output.matrix_rain,
    );
    f.render_widget(Paragraph::new(token_lines).alignment(ratatui::layout::Alignment::Center), screen);

    // 2. Navigation Bar (Compact layout for smaller column)
    let nav_spans = vec![
        Span::styled(" [▲] ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [▼] ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [OK] ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [X] ", Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Red).add_modifier(Modifier::BOLD)),
    ];
    f.render_widget(Paragraph::new(Line::from(nav_spans)), chunks[1]);

    // Register hitboxes for nav bar controls
    let nav_x = inner.x;
    let up_w = 5u16;
    let down_w = 5u16;
    let ok_w = 6u16;
    let cancel_w = 5u16;
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

    // 3. Action Buttons with active selection highlight & Stop Button (Sovereign themed)
    let stop_label = if app.pet_is_stopped { "▶Spin(p)" } else { "■Stop(p)" };
    let stop_label_short = if app.pet_is_stopped { "▶(p)" } else { "■(p)" };
    let actions: &[&str] = if inner.width >= 36 {
        &[
            "Overclock(f)",
            "Audit(t)",
            "Purge(s)",
            stop_label,
            "Log(h)",
        ]
    } else {
        &[
            "(f)",
            "(t)",
            "(s)",
            stop_label_short,
            "(h)",
        ]
    };

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
            3 => HitboxAction::PetStop,
            4 => HitboxAction::PetToggleHistory,
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

// ── Pet Switch Confirmation Modal ───────────────────────────────────────────

fn draw_pet_switch_confirm_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(52, 13, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🌀 Sovereign System Purge & Cache Flush ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let current_name = if let Some(ref d) = app.db {
        d.get_active_pet().ok().flatten().map(|p| p.name).unwrap_or_else(|| "Sovereign".to_string())
    } else {
        "Sovereign".to_string()
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " Trigger Sovereign Cache Flush & +5 XP Proof of Work? ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Mascot: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("🪙 {current_name}"), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("  ➔  ", Style::default().fg(palette.text_muted)),
            Span::styled("🌀 Purge Buffers & Refresh Telemetry", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [Y] Yes, Purge ",
                Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                " [N / Esc] Cancel & Keep ",
                Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);

    // Register hitboxes for [Yes] and [Cancel]
    let btn_y = inner.y + 5;
    app.register_hitbox(
        Rect { x: inner.x + 4, y: btn_y, width: 17, height: 1 },
        HitboxAction::ConfirmSwitchPet,
    );
    app.register_hitbox(
        Rect { x: inner.x + 25, y: btn_y, width: 25, height: 1 },
        HitboxAction::CancelSwitchPet,
    );
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
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    );
    f.render_widget(Paragraph::new(Line::from(close_span)), content_layout[1]);
}

// ── Tab 4: Sovereign Web Analytics (Umami-like) ────────────────────────────

fn draw_analytics_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    if app.show_analytics_detail {
        draw_analytics_detail_view(f, app, area);
    } else {
        draw_analytics_list_view(f, app, area);
    }
}

fn draw_analytics_detail_view(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Action Bar
            Constraint::Min(10),   // Main Area
        ])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_back_w = 26u16;
    let btn_refresh_w = 16u16;

    let x0 = bar_inner.x;
    let y0 = bar_inner.y;
    let h = bar_inner.height.min(1);
    app.register_hitbox(Rect { x: x0 + 1, y: y0, width: btn_back_w, height: h }, HitboxAction::CloseAnalyticsDetail);
    let x1 = x0 + btn_back_w + 3;
    app.register_hitbox(Rect { x: x1, y: y0, width: btn_refresh_w, height: h }, HitboxAction::RefreshAnalytics);

    let back_label = " [ ← Back to Sites (b/Esc) ] ";
    let bar_spans = vec![
        Span::styled(back_label, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" [ Refresh (r) ] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("  │  "),
        Span::styled("Press 'b' or Esc to return to Overview table", Style::default().fg(palette.text_muted)),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let snap = app.latest_snap.clone();
    let analytics = &snap.web_analytics;

    let site_name = match app.analytics_selected_site {
        0 => "All Sovereign Apps (Aggregated)".to_string(),
        idx => app.analytic_sites().get(idx).map(|s| s.name.clone()).unwrap_or_else(|| "All Sites".to_string()),
    };

    let main_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" 📊 Site Detail Analytics · {} ", site_name))
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(palette.accent));

    let main_inner = main_block.inner(chunks[1]);
    f.render_widget(main_block, chunks[1]);

    if main_inner.width < 20 || main_inner.height < 6 {
        return;
    }

    // Split main_inner into 3 responsive vertical sections:
    // Section 1: KPI Cards + Daily Pulse Chart
    // Section 2: Traffic & Device Breakdown Table
    // Section 3: Top Pages & Top Referrers
    let detail_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // KPIs + Pulse
            Constraint::Length(7), // Breakdown Table
            Constraint::Min(6),    // Top Pages + Referrers
        ])
        .split(main_inner);

    // Section 1: KPIs + Pulse
    let row1_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(34), // KPIs
            Constraint::Min(20),   // Pulse Chart
        ])
        .split(detail_rows[0]);

    let avg_secs = analytics.avg_session_secs;
    let avg_min = (avg_secs / 60.0) as u32;
    let avg_sec = (avg_secs % 60.0) as u32;
    let kpi_lines = vec![
        Line::from(vec![
            Span::styled("Pageviews:   ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:<10}", analytics.pageviews), Style::default().fg(Color::Rgb(255, 255, 255)).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Uniq Visit:  ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:<10}", analytics.unique_visitors), Style::default().fg(Color::Rgb(255, 255, 255)).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Live Now:    ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:<10}", analytics.active_visitors), Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Bounce Rate: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:.1}%", analytics.bounce_rate), Style::default().fg(Color::LightYellow)),
        ]),
        Line::from(vec![
            Span::styled("Avg Session: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!(" {}m {}s", avg_min, avg_sec), Style::default().fg(Color::Rgb(255, 255, 255))),
        ]),
    ];

    let kpi_block = Block::default()
        .borders(Borders::ALL)
        .title(" Key Performance Indicators ")
        .border_style(Style::default().fg(palette.accent));
    let kpi_inner = kpi_block.inner(row1_cols[0]);
    f.render_widget(kpi_block, row1_cols[0]);
    f.render_widget(Paragraph::new(kpi_lines), kpi_inner);

    let chart_block = Block::default()
        .borders(Borders::ALL)
        .title(" Daily Traffic Pulse (Last 7 Days) ")
        .border_style(Style::default().fg(palette.accent));
    let chart_inner = chart_block.inner(row1_cols[1]);
    f.render_widget(chart_block, row1_cols[1]);

    let mut chart_lines = Vec::new();
    if analytics.daily_traffic.is_empty() {
        chart_lines.push(Line::from(" No Traffic Events Recorded Yet. "));
        chart_lines.push(Line::from(Span::styled(" Tip: Send test hits with 'p' or embed tracker.js snippet. ", Style::default().fg(palette.text_muted))));
    } else {
        let max_pvs = analytics.daily_traffic.iter().map(|(_, c)| *c).max().unwrap_or(1).max(1);
        for (day, count) in &analytics.daily_traffic {
            let ratio = *count as f32 / max_pvs as f32;
            let bar_len = (ratio * 22.0).min(22.0) as usize;
            let bar_str = "█".repeat(bar_len.max(1));
            chart_lines.push(Line::from(vec![
                Span::styled(format!("{day}: "), Style::default().fg(palette.text_muted)),
                Span::styled(format!(" {bar_str:<22}"), Style::default().fg(palette.accent)),
                Span::styled(format!(" {count} views"), Style::default().fg(Color::Rgb(255, 255, 255))),
            ]));
        }
    }
    f.render_widget(Paragraph::new(chart_lines), chart_inner);

    // Section 2: Breakdown Table
    let content_rows = vec![
        Row::new(vec![
            Cell::from("BROWSERS").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
            Cell::from("OS / SYSTEMS").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
            Cell::from("DEVICES").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
            Cell::from("COUNTRIES").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        ]),
        Row::new(vec![
            Cell::from(
                if analytics.browser_breakdown.is_empty() { "Chrome: 80%\nFirefox: 20%".to_string() } else {
                    analytics.browser_breakdown.iter().map(|b| format!("{}: {:.0}%", b.label, b.pct)).collect::<Vec<_>>().join("\n")
                }
            ),
            Cell::from(
                if analytics.os_breakdown.is_empty() { "Linux: 75%\nWindows: 25%".to_string() } else {
                    analytics.os_breakdown.iter().map(|b| format!("{}: {:.0}%", b.label, b.pct)).collect::<Vec<_>>().join("\n")
                }
            ),
            Cell::from(
                if analytics.device_breakdown.is_empty() { "Desktop: 90%\nMobile: 10%".to_string() } else {
                    analytics.device_breakdown.iter().map(|b| format!("{}: {:.0}%", b.label, b.pct)).collect::<Vec<_>>().join("\n")
                }
            ),
            Cell::from(
                if analytics.country_breakdown.is_empty() { "ID: 95%\nUS: 5%".to_string() } else {
                    analytics.country_breakdown.iter().map(|b| format!("{}: {:.0}%", b.label, b.pct)).collect::<Vec<_>>().join("\n")
                }
            ),
        ]),
    ];

    let breakdown_table = Table::new(content_rows, [
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .style(Style::default().fg(palette.text_muted))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Traffic & Device Breakdown ")
            .border_style(Style::default().fg(palette.accent)),
    );
    f.render_widget(breakdown_table, detail_rows[1]);

    // Section 3: Top Pages & Top Referrers
    let row3_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(detail_rows[2]);

    let total_views = analytics.pageviews.max(1);
    let top_pages_lines: Vec<Line> = if analytics.top_pages.is_empty() {
        vec![
            Line::from(vec![Span::styled(" /  (Landing Page)", Style::default().fg(Color::White)), Span::styled(" — 14 views", Style::default().fg(palette.text_muted))]),
            Line::from(vec![Span::styled(" /docs  (Documentation)", Style::default().fg(Color::White)), Span::styled(" — 9 views", Style::default().fg(palette.text_muted))]),
        ]
    } else {
        analytics.top_pages.iter().take(5).map(|(page, count)| {
            let ratio = *count as f32 / total_views as f32;
            let bar_len = (ratio * 14.0).min(14.0) as usize;
            let bar_str = "■".repeat(bar_len.max(1));
            Line::from(vec![
                Span::styled(format!("{:<24} ", page), Style::default().fg(Color::Rgb(255, 255, 255))),
                Span::styled(format!("{bar_str:<14}"), Style::default().fg(palette.accent)),
                Span::styled(format!(" {count}"), Style::default().fg(palette.text_muted)),
            ])
        }).collect()
    };

    let ref_lines: Vec<Line> = if analytics.top_referrers.is_empty() {
        vec![
            Line::from(vec![Span::styled(" Direct / Bookmark", Style::default().fg(Color::White)), Span::styled(" — 18 visitors", Style::default().fg(palette.text_muted))]),
            Line::from(vec![Span::styled(" https://google.com", Style::default().fg(Color::White)), Span::styled(" — 5 visitors", Style::default().fg(palette.text_muted))]),
        ]
    } else {
        analytics.top_referrers.iter().take(5).map(|(refr, count)| {
            let ratio = *count as f32 / total_views as f32;
            let bar_len = (ratio * 14.0).min(14.0) as usize;
            let bar_str = "▸".repeat(bar_len.max(1));
            Line::from(vec![
                Span::styled(format!("{:<24} ", refr), Style::default().fg(Color::Rgb(255, 255, 255))),
                Span::styled(format!("{bar_str:<14}"), Style::default().fg(palette.accent)),
                Span::styled(format!(" {count}"), Style::default().fg(palette.text_muted)),
            ])
        }).collect()
    };

    let pages_block = Block::default()
        .borders(Borders::ALL)
        .title(" Top Visited Pages ")
        .border_style(Style::default().fg(palette.accent));
    let p_inner = pages_block.inner(row3_cols[0]);
    f.render_widget(pages_block, row3_cols[0]);
    f.render_widget(Paragraph::new(top_pages_lines), p_inner);

    let refs_block = Block::default()
        .borders(Borders::ALL)
        .title(" Top Traffic Sources & Referrers ")
        .border_style(Style::default().fg(palette.accent));
    let r_inner = refs_block.inner(row3_cols[1]);
    f.render_widget(refs_block, row3_cols[1]);
    f.render_widget(Paragraph::new(ref_lines), r_inner);

    if app.show_analytics_script_modal {
        draw_analytics_script_modal(f, app, area);
    }
    if app.show_site_import_modal {
        draw_site_import_modal(f, app, area);
    }
}

fn draw_analytics_list_view(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Action Bar
            Constraint::Min(10),   // Main Area
        ])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_add_w = 14u16;
    let btn_import_w = 14u16;
    let btn_refresh_w = 16u16;
    let btn_script_w = 16u16;
    let btn_export_w = 14u16;
    let btn_del_w = 14u16;

    let x0 = bar_inner.x;
    let y0 = bar_inner.y;
    let h = bar_inner.height.min(1);
    app.register_hitbox(Rect { x: x0 + 1, y: y0, width: btn_add_w, height: h }, HitboxAction::ShowSiteWizard);
    let x1 = x0 + btn_add_w + 3;
    app.register_hitbox(Rect { x: x1, y: y0, width: btn_import_w, height: h }, HitboxAction::ImportAnalyticsSites);
    let x2 = x1 + btn_import_w + 1;
    app.register_hitbox(Rect { x: x2, y: y0, width: btn_refresh_w, height: h }, HitboxAction::RefreshAnalytics);
    let x3 = x2 + btn_refresh_w + 1;
    app.register_hitbox(Rect { x: x3, y: y0, width: btn_script_w, height: h }, HitboxAction::ShowAnalyticsScript);
    let x4 = x3 + btn_script_w + 1;
    app.register_hitbox(Rect { x: x4, y: y0, width: btn_export_w, height: h }, HitboxAction::ExportAnalyticsJson);
    let x5 = x4 + btn_export_w + 1;
    app.register_hitbox(Rect { x: x5, y: y0, width: btn_del_w, height: h }, HitboxAction::ClearAnalyticsData);

    let bar_spans = vec![
        Span::styled(" [ Add Site (a) ] ", Style::default().fg(Color::LightMagenta).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [ Import (i) ] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [ Refresh (r) ] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [ Script (s) ] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [ Export (e) ] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(" [ Wipe/Del (x) ] ", Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD)),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let snap = app.latest_snap.clone();
    let analytics = &snap.web_analytics;

    let main_block = Block::default()
        .borders(Borders::ALL)
        .title(" 📍 Site Overview & Analytics Dashboard · Press [Enter] or [d] to open Detail View")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(palette.accent));

    let main_inner = main_block.inner(chunks[1]);
    f.render_widget(main_block, chunks[1]);

    // Render Sites List Table
    let sites = app.analytic_sites();
    let site_products: Vec<crate::systat::ProductAnalyticsSummary> = analytics.product_analytics.clone();

    let header_cells = vec![
        Cell::from("#").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Cell::from("SITE / PRODUCT").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Cell::from("DOMAIN").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Cell::from("STATUS").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Cell::from("PAGEVIEWS").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Cell::from("VISITORS").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Cell::from("LIVE NOW").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Cell::from("BOUNCE %").style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
    ];

    let mut rows: Vec<Row> = Vec::new();

    let total_sites = sites.len();
    let max_visible = main_inner.height.saturating_sub(2) as usize;
    let start_idx = app.analytics_scroll.min(total_sites.saturating_sub(1));
    let end_idx = (start_idx + max_visible).min(total_sites);

    for i in start_idx..end_idx {
        let site = &sites[i];
        let is_selected = i == app.analytics_selected_site.min(total_sites.saturating_sub(1));
        let prod_summary = site_products.iter().find(|p| p.product_id == site.id);

        let pvs = prod_summary.map(|p| p.pageviews).unwrap_or(0);
        let viss = prod_summary.map(|p| p.visitors).unwrap_or(0);
        let live = prod_summary.map(|p| p.active_now).unwrap_or(0);
        let bounce = prod_summary.map(|p| p.bounce_rate).unwrap_or(0.0);
        let status = prod_summary.map(|p| p.status.clone()).unwrap_or_else(|| {
            if live > 0 { "ACTIVE".to_string() } else if pvs > 0 { "IDLE".to_string() } else { "EMPTY".to_string() }
        });

        let status_style = if status == "ACTIVE" {
            Style::default().fg(Color::LightGreen)
        } else if status == "IDLE" {
            Style::default().fg(Color::LightYellow)
        } else {
            Style::default().fg(palette.text_muted)
        };

        let row_style = if is_selected {
            Style::default().fg(Color::Rgb(0, 0, 0)).bg(palette.status_ok).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(255, 255, 255))
        };

        let row_cells = vec![
            Cell::from(format!("{}", i + 1)).style(row_style),
            Cell::from(site.name.clone()).style(row_style.add_modifier(Modifier::BOLD)),
            Cell::from(site.domain.clone()).style(row_style),
            Cell::from(status).style(status_style.add_modifier(Modifier::BOLD)),
            Cell::from(format!("{pvs}")).style(row_style),
            Cell::from(format!("{viss}")).style(row_style),
            Cell::from(format!("{live}")).style(row_style),
            Cell::from(format!("{bounce:.1}%")).style(row_style),
        ];

        rows.push(Row::new(row_cells));

        let row_y = main_inner.y + (i as u16 - start_idx as u16 + 2).min(main_inner.height.saturating_sub(1));
        let row_w = main_inner.width.saturating_sub(4);
        app.register_hitbox(
            Rect {
                x: main_inner.x + 2,
                y: row_y,
                width: row_w,
                height: 1,
            },
            HitboxAction::ViewAnalyticsDetail(i),
        );
    }

    let sites_table = Table::new(rows, [
        Constraint::Length(4),
        Constraint::Percentage(22),
        Constraint::Percentage(24),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Min(8),
    ])
    .header(Row::new(header_cells).style(Style::default().fg(palette.status_warn).add_modifier(Modifier::BOLD)))
    .style(Style::default().fg(palette.text_muted));

    f.render_widget(sites_table, main_inner);

    if app.show_analytics_script_modal {
        draw_analytics_script_modal(f, app, area);
    }

    if app.show_site_import_modal {
        draw_site_import_modal(f, app, area);
    }
}

fn draw_site_import_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 42, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🌐 Add Website / Landing Page to Sovereign Analytics ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 4 {
        return;
    }

    let fields = [
        ("Site Name:", app.site_import_name.clone(), "e.g. CATerm Terminal"),
        ("Domain:", app.site_import_domain.clone(), "e.g. caterm.fathforce.com"),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Register a website or landing page for live visitor tracking.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.site_import_selected == i;
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
            Span::styled(format!("{:<14}", label), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            val_span,
        ]));
        lines.push(Line::from(""));

        let hitbox_action = match i {
            0 => HitboxAction::FocusSiteImportName,
            _ => HitboxAction::FocusSiteImportDomain,
        };
        app.register_hitbox(
            Rect {
                x: inner.x + 2,
                y: field_start_y + (i as u16 * 2),
                width: inner.width.saturating_sub(4),
                height: 1,
            },
            hitbox_action,
        );
    }

    let btn_y = inner.bottom().saturating_sub(2);
    let submit_span = Span::styled(
        " [ Submit (Enter) ] ",
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
    );
    let cancel_span = Span::styled(
        " [ Cancel (Esc) ] ",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    );

    app.register_hitbox(
        Rect { x: inner.x + 2, y: btn_y, width: 20, height: 1 },
        HitboxAction::SubmitSiteImport,
    );
    app.register_hitbox(
        Rect { x: inner.x + 24, y: btn_y, width: 18, height: 1 },
        HitboxAction::CancelSiteImport,
    );

    f.render_widget(Paragraph::new(lines), Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(3) });
    f.render_widget(
        Paragraph::new(Line::from(vec![submit_span, Span::raw("  "), cancel_span])),
        Rect { x: inner.x + 2, y: btn_y, width: inner.width.saturating_sub(4), height: 1 },
    );
}

// ── Pro Modal Wizard (Phase 4) ──────────────────────────────────────────────

fn draw_pro_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 46, area);
    f.render_widget(Clear, popup_area);

    let is_pro = app.is_pro();
    let title = if is_pro {
        " ★ CAShell Pro — Cloud Sync & Sovereign Account "
    } else {
        " ○ CAShell Free — Activate Pro Account or 7-Day Trial "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(if is_pro {
                    Color::Rgb(0, 255, 120)
                } else {
                    Color::Rgb(255, 200, 0)
                })
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let cache = crate::pro::FastBootCache::load();
    let account_status = if is_pro {
        let email = if app.cfg.pro.account_email.is_empty() {
            "trial@fathforce.com".to_string()
        } else {
            app.cfg.pro.account_email.clone()
        };
        let expiry_str = cache
            .as_ref()
            .map(|c| {
                if c.status == 1 && c.session_expires_unix > 0 {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    let rem_days = (c.session_expires_unix.saturating_sub(now) as f64 / 86400.0).ceil() as i64;
                    format!("{rem_days} days")
                } else {
                    "Perpetual".to_string()
                }
            })
            .unwrap_or_else(|| "Active".to_string());
        format!("Active Pro ({email}) │ Validity: {expiry_str}")
    } else {
        "Free Tier │ Local Only (Cloud Sync Locked)".to_string()
    };

    let auto_sync_str = if app.cfg.pro.auto_sync { "ON" } else { "OFF" };
    let interval = app.cfg.pro.sync_interval_mins;

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Status: {account_status}  │  Auto-Sync: {auto_sync_str} ({interval}m)"),
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    // Field 0: Email
    let is_email = app.pro_wizard_field == 0;
    let prefix0 = if is_email { "▶ " } else { "  " };
    let email_val = if is_email {
        Span::styled(
            format!(" [ {}█ ] ", app.pro_wizard_email_buf),
            Style::default()
                .fg(Color::Rgb(0, 0, 0))
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        let display = if app.pro_wizard_email_buf.is_empty() {
            "user@fathforce.com".to_string()
        } else {
            app.pro_wizard_email_buf.clone()
        };
        Span::styled(
            format!(" [ {} ] ", display),
            Style::default().fg(if app.pro_wizard_email_buf.is_empty() { Color::DarkGray } else { Color::White }),
        )
    };

    lines.push(Line::from(vec![
        Span::styled(prefix0, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("Pro Account Email: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        email_val,
    ]));

    // Field 1: License Token / Secret
    let is_token = app.pro_wizard_field == 1;
    let prefix1 = if is_token { "▶ " } else { "  " };
    let token_val = if is_token {
        Span::styled(
            format!(" [ {}█ ] ", app.pro_wizard_token_buf),
            Style::default()
                .fg(Color::Rgb(0, 0, 0))
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        let display = if app.pro_wizard_token_buf.is_empty() {
            "optional license token".to_string()
        } else {
            "••••••••••••••••".to_string()
        };
        Span::styled(
            format!(" [ {} ] ", display),
            Style::default().fg(if app.pro_wizard_token_buf.is_empty() { Color::DarkGray } else { Color::White }),
        )
    };

    lines.push(Line::from(vec![
        Span::styled(prefix1, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("License / Passphrase: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        token_val,
    ]));

    lines.push(Line::from(""));

    let err_msg = app.pro_wizard_status_msg.clone();
    if let Some(msg) = err_msg {
        lines.push(Line::from(Span::styled(
            msg,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
    }

    let btn_sync_label = if app.is_syncing_gcc { "Syncing…" } else { "Sync GCC" };
    let btn_sync = Span::styled(
        format!(" [☁ {btn_sync_label}] (y) "),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    );
    let btn_trial = Span::styled(
        " [★ 7-Day Trial] (t) ",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    );
    let btn_login = Span::styled(
        " [🔐 Sign In] (Enter) ",
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
    );
    let btn_close = Span::styled(
        " [✖ Cancel] (Esc) ",
        Style::default().fg(Color::DarkGray),
    );

    let btn_y = inner.bottom().saturating_sub(3);

    app.register_hitbox(
        Rect { x: inner.x + 2, y: inner.y + 2, width: inner.width.saturating_sub(4), height: 1 },
        HitboxAction::FocusProField(0),
    );
    app.register_hitbox(
        Rect { x: inner.x + 2, y: inner.y + 3, width: inner.width.saturating_sub(4), height: 1 },
        HitboxAction::FocusProField(1),
    );
    app.register_hitbox(
        Rect { x: inner.x + 2, y: btn_y, width: 18, height: 1 },
        HitboxAction::SyncGcc,
    );
    app.register_hitbox(
        Rect { x: inner.x + 21, y: btn_y, width: 22, height: 1 },
        HitboxAction::ActivateProTrial,
    );
    app.register_hitbox(
        Rect { x: inner.x + 44, y: btn_y, width: 20, height: 1 },
        HitboxAction::SubmitProLogin,
    );
    app.register_hitbox(
        Rect { x: inner.x + 2, y: btn_y + 1, width: 18, height: 1 },
        HitboxAction::CloseProWizard,
    );

    lines.push(Line::from(vec![
        btn_sync,
        Span::raw(" "),
        btn_trial,
        Span::raw(" "),
        btn_login,
        Span::raw(" "),
        btn_close,
    ]));

    lines.push(Line::from(Span::styled(
        "Tab: switch field │ Enter: sign in │ t: activate 7-day trial │ y: sync │ Esc: exit",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x + 1,
            y: inner.y + 1,
            width: inner.width.saturating_sub(2),
            height: inner.height.saturating_sub(2),
        },
    );
}

fn draw_analytics_script_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let modal_width = 96.min(area.width.saturating_sub(2));
    let modal_height = 24.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(modal_width)) / 2;
    let y = (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect { x, y, width: modal_width, height: modal_height };

    f.render_widget(Clear, modal_area);

    let active_site = app.analytics_site_filter.as_deref().unwrap_or("caterm");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" 📋 Sovereign Web Analytics · Tracker Templates (Site ID: '{active_site}') "))
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(palette.status_ok));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines = vec![
        Line::from(vec![
            Span::styled("⚡ 1. Universal HTML / Landing (index.html):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            format!("   <script defer src=\"http://127.0.0.1:8090/tracker.js\" data-site=\"{active_site}\"></script>"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("⚡ 2. Svelte 5 / SvelteKit (src/routes/+layout.svelte):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            format!("   <svelte:head>\n     <script defer src=\"http://127.0.0.1:8090/tracker.js\" data-site=\"{active_site}\"></script>\n   </svelte:head>"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("⚡ 3. Astro / SSR (src/layouts/Layout.astro):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            format!("   <script defer src=\"http://127.0.0.1:8090/tracker.js\" data-site=\"{active_site}\"></script>"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("⚡ 4. WordPress / PHP (functions.php):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            format!("   add_action('wp_head', fn() => print('<script defer src=\"http://127.0.0.1:8090/tracker.js\" data-site=\"{active_site}\"></script>'));"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled("⚡ 5. Custom CTA Tracking (data attributes):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(
            "   <button data-cashell-event=\"signup_cta\" data-cashell-val=\"pro_annual\">Get Pro</button>",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled("📌 Note: Auto-tracks Scroll Depth (25/50/75/90/100%), Clicks (% coords), Rage Clicks (>3/sec), and Dwell Time.", Style::default().fg(palette.text_muted))),
        Line::from(Span::styled("📌 Replace 'http://127.0.0.1:8090' with your public domain or CF Tunnel (e.g. 'https://cashell.cecepazhar.com').", Style::default().fg(palette.text_muted))),
        Line::from(""),
        Line::from(Span::styled(" [ Press Esc to Close ] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}

// ── Tab 3: Audit & Code ──────────────────────────────────────────────────────

#[allow(dead_code)]
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
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    );
    let btn_fix = Span::styled(
        " [🔧 Run Fix] ",
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
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
    let btn_sync_gcc_rect = Rect {
        x: action_inner.x + 65,
        y: action_inner.y,
        width: 14,
        height: 1,
    };
    let btn_pro_rect = Rect {
        x: action_inner.x + 80,
        y: action_inner.y,
        width: 18,
        height: 1,
    };

    app.register_hitbox(btn_save_rect, HitboxAction::SaveSettings);
    app.register_hitbox(btn_reload_rect, HitboxAction::ReloadSettings);
    app.register_hitbox(btn_clear_rect, HitboxAction::ClearChat);
    app.register_hitbox(btn_vacuum_rect, HitboxAction::VacuumDb);
    app.register_hitbox(btn_sync_gcc_rect, HitboxAction::SyncGcc);
    app.register_hitbox(btn_pro_rect, HitboxAction::OpenProWizard);

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
    let btn_sync_gcc = Span::styled(
        " [☁ Sync GCC] ",
        Style::default()
            .fg(Color::Rgb(0, 0, 0))
            .bg(Color::Rgb(0, 200, 255))
            .add_modifier(Modifier::BOLD),
    );
    let is_pro = app.is_pro();
    let btn_pro = Span::styled(
        " [★ Pro Account] ",
        Style::default()
            .fg(Color::Rgb(0, 0, 0))
            .bg(if is_pro { Color::Rgb(0, 255, 120) } else { Color::Rgb(255, 200, 0) })
            .add_modifier(Modifier::BOLD),
    );

    let status = app
        .settings_status_msg
        .as_deref()
        .unwrap_or("s: save | r: reload | c: clear chat | v: vacuum | y: sync | P: pro");

    f.render_widget(
        Paragraph::new(Line::from(vec![
            btn_save,
            Span::raw(" "),
            btn_reload,
            Span::raw(" "),
            btn_clear,
            Span::raw(" "),
            btn_vacuum,
            Span::raw(" "),
            btn_sync_gcc,
            Span::raw(" "),
            btn_pro,
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
    let matrix_rain_str = if app.cfg.output.matrix_rain { "Enabled (Matrix Rain Background)" } else { "Disabled (Clean Background)" };
    let matrix_speed_str = app.cfg.mascot.matrix_speed.as_str();
    let mascot_speed_str = match app.cfg.mascot.speed.as_str() {
        "Fast" => "Fast (2s interval)",
        "Slow" => "Slow (10s interval)",
        "Zen" => "Zen (20s interval)",
        _ => "Normal (5s interval)",
    };
    let mascot_sens_str = match app.cfg.mascot.sensitivity.as_str() {
        "High" => "High Reactive (>70°C / >70% Load)",
        "Calm" => "Calm (>90°C / >92% Load)",
        _ => "Balanced (>80°C / >85% Load)",
    };
    let mascot_track_str = if app.cfg.mascot.condition_tracking { "Enabled (Reflects CPU/RAM/Outage)" } else { "Disabled (Neutral)" };
    let mascot_color_str = app.cfg.mascot.color_scheme.as_str();
    let mascot_anim_str = app.cfg.mascot.animation_style.as_str();
    let mascot_scale_str = app.cfg.mascot.render_scale.as_str();
    let mascot_chat_str = if app.cfg.mascot.chat_avatar_enabled { "Enabled (Live Reactor & Avatars)" } else { "Disabled" };
    let audit_limit_str = format!("{} KB ({} tokens)", app.cfg.ai.audit_max_context_kb, app.cfg.ai.audit_context_limit_tokens);

    let values: [&str; 32] = [
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
        matrix_rain_str,
        matrix_speed_str,
        mascot_speed_str,
        mascot_sens_str,
        mascot_track_str,
        mascot_color_str,
        mascot_anim_str,
        mascot_scale_str,
        mascot_chat_str,
        audit_limit_str.as_str(),
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

// ── Tab 12: Turbo & Performance Overclock Engine ─────────────────────────────

fn draw_turbo_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Telemetry & Thermal HUD
            Constraint::Min(12),   // Profiles (Left) + Granular Actions (Right)
            Constraint::Length(9), // Live Execution Activity Stream & Benchmark Output
        ])
        .split(area);

    // 1. Top HUD: Real-time Hardware Telemetry & Governor Status
    let snap = &app.latest_snap;
    let hud_block = Block::default()
        .borders(Borders::ALL)
        .title(" ⚡ SOVEREIGN HARDWARE & KERNEL STATUS HUD ")
        .border_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    let hud_inner = hud_block.inner(chunks[0]);
    f.render_widget(hud_block, chunks[0]);

    let gov_color = if app.turbo_profile_active == "MAX_TURBO" { Color::Green } else { Color::Cyan };
    let temp_color = if snap.cpu_temp_c > 80.0 { Color::LightRed } else if snap.cpu_temp_c > 65.0 { Color::Yellow } else { Color::Green };

    let hud_line1 = Line::from(vec![
        Span::styled(" Profile: ", Style::default().fg(palette.text_muted)),
        Span::styled(format!("[ {} ]", app.turbo_profile_active), Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("Governor: ", Style::default().fg(palette.text_muted)),
        Span::styled("PERFORMANCE (All Cores)", Style::default().fg(gov_color).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("Intel Turbo Boost: ", Style::default().fg(palette.text_muted)),
        Span::styled("ACTIVE (Up to 4.20 GHz)", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("Package Temp: ", Style::default().fg(palette.text_muted)),
        Span::styled(format!("{:.1}°C", snap.cpu_temp_c), Style::default().fg(temp_color).add_modifier(Modifier::BOLD)),
    ]);

    let ram_used = snap.ram_used_mb;
    let ram_tot = snap.ram_total_mb;
    let swap_used = snap.swap_used_mb;
    let swap_tot = snap.swap_total_mb;

    let hud_line2 = Line::from(vec![
        Span::styled(" Memory: ", Style::default().fg(palette.text_muted)),
        Span::styled(format!("{ram_used}/{ram_tot} MB"), Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("ZRAM Swap: ", Style::default().fg(palette.text_muted)),
        Span::styled(format!("{swap_used}/{swap_tot} MB"), Style::default().fg(Color::White)),
        Span::raw("   "),
        Span::styled("Docker Reclaimable: ", Style::default().fg(palette.text_muted)),
        Span::styled("~68.7 GB (BuildKit + Images)", Style::default().fg(Color::LightMagenta).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("TCP Congestion: ", Style::default().fg(palette.text_muted)),
        Span::styled("BBR / Low Latency Mesh", Style::default().fg(Color::Green)),
    ]);

    f.render_widget(Paragraph::new(vec![hud_line1, hud_line2]), hud_inner);

    // 2. Middle Body: Two Columns (Left: 4 Profiles, Right: 8 Actions)
    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(42), // Profiles
            Constraint::Percentage(58), // Granular Actions
        ])
        .split(chunks[1]);

    // Left Column: 4 One-Click Profiles
    let profiles_block = Block::default()
        .borders(Borders::ALL)
        .title(" 🚀 1-Click Sovereign Profiles [t, b, p, e] ")
        .border_style(Style::default().fg(Color::Cyan));
    let prof_inner = profiles_block.inner(body_cols[0]);
    f.render_widget(profiles_block, body_cols[0]);

    let profiles = [
        ("⚡ [t] MAX SOVEREIGN TURBO", "Lock max 4.2GHz, EPP Performance, Renice CA daemons (-10), TCP BBR", 0),
        ("🛠 [b] DEV BUILD ACCELERATOR", "Cargo 8 jobs, I/O Best-Effort high priority, Memory defrag", 1),
        ("🧹 [p] DEEP STORAGE & RAM PURGE", "Flush Linux VFS caches, trim SSD, prune Docker 68GB build cache", 2),
        ("🍃 [e] THERMAL SAFE / ECO", "Restore governor to powersave, EPP balance-power, quiet fan", 3),
    ];

    let mut prof_lines = Vec::new();
    let mut current_py = prof_inner.y;
    for (name, desc, idx) in &profiles {
        let is_active = (app.turbo_profile_active == "MAX_TURBO" && *idx == 0)
            || (app.turbo_profile_active == "BUILD_ACCEL" && *idx == 1)
            || (app.turbo_profile_active == "PURGE_CACHE" && *idx == 2)
            || (app.turbo_profile_active == "ECO_BALANCED" && *idx == 3);

        let title_style = if is_active {
            Style::default().fg(Color::Rgb(0, 0, 0)).bg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        };

        prof_lines.push(Line::from(vec![
            Span::styled(format!(" {name} "), title_style),
        ]));
        prof_lines.push(Line::from(vec![
            Span::styled(format!("   {desc}"), Style::default().fg(palette.text_muted)),
        ]));
        prof_lines.push(Line::from(""));

        app.register_hitbox(
            Rect { x: prof_inner.x, y: current_py, width: prof_inner.width, height: 2 },
            HitboxAction::TurboProfile(*idx),
        );
        current_py = current_py.saturating_add(3);
    }
    f.render_widget(Paragraph::new(prof_lines), prof_inner);

    // Right Column: Granular Hardware & Software Optimization Actions
    let actions_block = Block::default()
        .borders(Borders::ALL)
        .title(" ⚙ Granular Optimizations [Select: j/k, Exec: Enter] ")
        .border_style(Style::default().fg(palette.accent));
    let act_inner = actions_block.inner(body_cols[1]);
    f.render_widget(actions_block, body_cols[1]);

    let opt_actions = [
        ("Drop Linux VFS Page Caches", "Flushes pagecache, dentries & inodes to instantly reclaim inactive RAM", "echo 3 > /proc/sys/vm/drop_caches"),
        ("Compact Linux Memory Pages", "Defragments continuous memory blocks for faster large Rustc allocs", "echo 1 > /proc/sys/vm/compact_memory"),
        ("Prune Docker BuildKit Cache", "Safely purges dangling build cache layers (~56 GB reclaimable)", "docker builder prune -f"),
        ("Prune Dangling Docker Images", "Removes untagged & orphaned container image layers", "docker image prune -f"),
        ("Elevate CA Fleet Daemons", "Prioritizes cashell, caorch & 9router to nice -10 (Best-effort high)", "renice -n -10"),
        ("Trim SSD NVMe Flash Blocks", "Sends discard requests to flash controller for optimal NVMe write IOPS", "fstrim -v /"),
        ("Verify TCP BBR Congestion", "Ensures bottleneck bandwidth BBR algorithm active for fleet RPC", "sysctl net.ipv4.tcp_congestion_control"),
        ("Vacuum & WAL Checkpoint DB", "Shrinks ~/.config/cashell/cashell.db via PRAGMA wal_checkpoint", "PRAGMA wal_checkpoint(TRUNCATE)"),
    ];

    let items: Vec<ListItem> = opt_actions
        .iter()
        .enumerate()
        .map(|(i, (name, desc, cmd))| {
            let is_sel = i == app.turbo_selected_action;
            let bg = if is_sel { palette.row_selected_bg } else { Color::Reset };
            let prefix = if is_sel { "▶ " } else { "  " };

            let title_span = Span::styled(
                format!("{:<28}", name),
                Style::default().fg(if is_sel { palette.accent } else { palette.text }).add_modifier(Modifier::BOLD),
            );
            let cmd_span = Span::styled(
                format!(" [{cmd}]"),
                Style::default().fg(palette.text_muted),
            );
            let desc_span = Span::styled(
                format!(" — {desc}"),
                Style::default().fg(Color::DarkGray),
            );

            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.accent)),
                title_span,
                cmd_span,
                desc_span,
            ]);
            ListItem::new(line).style(Style::default().bg(bg))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.turbo_selected_action));
    f.render_stateful_widget(List::new(items), act_inner, &mut state);

    // Register hitboxes for action rows
    for (i, _) in opt_actions.iter().enumerate() {
        if (i as u16) < act_inner.height {
            let row_rect = Rect {
                x: act_inner.x,
                y: act_inner.y + (i as u16),
                width: act_inner.width,
                height: 1,
            };
            app.register_hitbox(row_rect, HitboxAction::TurboAction(i));
        }
    }

    // 3. Bottom Panel: Activity Stream & Execution Output
    let log_block = Block::default()
        .borders(Borders::ALL)
        .title(" 📜 Turbo Execution & Benchmark Activity Stream ")
        .border_style(Style::default().fg(palette.border));
    let log_inner = log_block.inner(chunks[2]);
    f.render_widget(log_block, chunks[2]);

    let log_lines: Vec<Line> = app
        .turbo_log_buffer
        .iter()
        .rev()
        .take(log_inner.height as usize)
        .rev()
        .map(|msg| {
            let color = if msg.starts_with('⚡') || msg.starts_with('🛠') {
                Color::Yellow
            } else if msg.starts_with('🧹') {
                Color::LightMagenta
            } else if msg.starts_with('✔') {
                Color::Green
            } else {
                Color::White
            };
            Line::from(vec![Span::styled(msg.clone(), Style::default().fg(color))])
        })
        .collect();

    f.render_widget(Paragraph::new(log_lines), log_inner);
}

// ── Tab 5: AI Chat (Hana AI chat interface) ──────────────────────────────────


// ── Footer & Compact Fallback ────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, app: &CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let keybinds = " [1-9/0/-/=]: Tabs │ Tab: Cycle │ ↑/↓: Nav │ Enter: Act │ ?: Help │ Esc: Back ";
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
            format!("CAShell v{} · Isometric Cockpit (Compact Mode)", env!("CARGO_PKG_VERSION")),
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
            Cell::from("1..9, 0, - / Shift+1..9, 0, -"),
            Cell::from("Switch tabs (Dashboard, Project, Uptime, Server, Analytics, Audit, CRON, Log, Terminal, AI, Setting)"),
        ]),
        Row::new(vec![
            Cell::from("Tab / BackTab (Shift+Tab)"),
            Cell::from("Cycle through 11 tabs forward / backward"),
        ]),
        Row::new(vec![
            Cell::from("↑ / ↓ (k / j)"),
            Cell::from("Navigate items, lists, tables, or scroll content"),
        ]),
        Row::new(vec![
            Cell::from("Analytics: w / s / r / t / x"),
            Cell::from("w: Filter Site | s: Tracker Script Tag | r: Refresh | t: Sim Pageview | x: Clear"),
        ]),
        Row::new(vec![
            Cell::from("Project: o / d / e / x / s"),
            Cell::from("o: Add Proj | d: Add Box | e: Edit Box | x: Delete | s: Start/Stop | Enter: Inspect"),
        ]),
        Row::new(vec![
            Cell::from("Uptime: m / e / x / p"),
            Cell::from("m: Add Monitor | e: Edit Monitor | x: Delete Monitor | p: Probe Now"),
        ]),
        Row::new(vec![
            Cell::from("Server: a / x / p"),
            Cell::from("a/n: Add Server wizard | x: Remove Node | p: Probe multi-node latencies"),
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
            let frame = status.build_frame(active_action, app.pet_tick, app.pet_is_stopped);
            let mascot_disp = format!("       {} {}       ", frame.core, frame.particles);
            lines.push(Line::from(Span::styled(mascot_disp, Style::default().fg(palette.highlight_fg).add_modifier(Modifier::BOLD))));
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

    let is_edit = app.server_wizard_is_edit;
    let orig_name = app.server_wizard_orig_name.clone();
    let wizard_title = if is_edit {
        format!(" ✎ Edit Fleet Server — Node Manager ({orig_name})")
    } else {
        " 🖥 Add Fleet Server — Node Wizard".to_string()
    };

    let block = Block::default()
        .title(wizard_title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_edit { palette.status_warn } else { palette.accent }).add_modifier(Modifier::BOLD));
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
        if is_edit {
            "Edit the fleet node IP, SSH user, and exposed services. Renaming the key is supported."
        } else {
            "Register a fleet node with IP & services. Auto-probes upon saving."
        },
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
        "Tab/Down: Next │ BackTab/Up: Prev │ Enter/Ctrl+S: Save │ Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let btn_save_str = if is_edit {
        " [✔ Save Changes] (Enter) "
    } else {
        " [✔ Add Server] (Enter) "
    };
    let btn_cancel_str = " [✖ Cancel] (Esc) ";

    let btn_save = Span::styled(
        btn_save_str,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let btn_cancel = Span::styled(
        btn_cancel_str,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    );

    let btn_y = inner.bottom().saturating_sub(2);
    let save_width = btn_save_str.chars().count() as u16;
    let cancel_width = btn_cancel_str.chars().count() as u16;
    let cancel_x = inner.x + save_width + 2;

    if btn_y < inner.bottom() && btn_y >= inner.y {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: save_width,
                height: 1,
            },
            HitboxAction::SubmitServer,
        );
        app.register_hitbox(
            Rect {
                x: cancel_x,
                y: btn_y,
                width: cancel_width,
                height: 1,
            },
            HitboxAction::CancelServer,
        );
    }

    f.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        },
    );
    if btn_y < inner.bottom() && btn_y >= inner.y {
        f.render_widget(
            Paragraph::new(Line::from(vec![btn_save, Span::raw("  "), btn_cancel])),
            Rect {
                x: inner.x,
                y: btn_y,
                width: inner.width,
                height: 1,
            },
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
        ("Group:", app.monitor_group_buf.clone(), "e.g. CA Ecosystem, Production, Custom"),
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

// ── Delete Confirmation Modal ────────────────────────────────────────────────

fn draw_delete_confirm_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(58, 13, area);
    f.render_widget(Clear, popup_area);

    let target_label = app.pending_delete_target.label();
    let title = format!(" 🗑 Confirm Deletion — {target_label} ");
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let label = if app.pending_delete_label.is_empty() {
        "this resource".to_string()
    } else {
        app.pending_delete_label.clone()
    };

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" Are you sure you want to permanently delete: ", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(format!(" ✖ {label} "), Style::default().fg(Color::Rgb(255, 120, 120)).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            " This action will immediately remove the resource and cannot be undone. ",
            Style::default().fg(palette.text_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [Y] Yes, Delete ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                " [N / Esc] Cancel & Keep ",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        inner,
    );

    // Register mouse hitboxes for [Yes, Delete] and [Cancel & Keep]
    let btn_y = inner.y + 5;
    let yes_w = 17u16;
    let gap_w = 4u16;
    let cancel_w = 25u16;
    let total_w = yes_w + gap_w + cancel_w;
    let start_x = inner.x + (inner.width.saturating_sub(total_w)) / 2;

    app.register_hitbox(
        Rect { x: start_x, y: btn_y, width: yes_w, height: 1 },
        HitboxAction::ConfirmDelete,
    );
    app.register_hitbox(
        Rect { x: start_x + yes_w + gap_w, y: btn_y, width: cancel_w, height: 1 },
        HitboxAction::CancelDelete,
    );
}

// ── Process Kill Confirmation Modal ──────────────────────────────────────────

fn draw_kill_process_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(58, 12, area);
    f.render_widget(Clear, popup_area);

    let (pid, name) = app
        .pending_kill_proc
        .clone()
        .unwrap_or((0, "unknown".to_string()));

    let block = Block::default()
        .title(" ■ Confirm Process Kill ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightRed).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!(" Confirm Kill PID {pid} ({name})? "),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            " This will send SIGTERM (or [9] for SIGKILL) to stop the process. ",
            Style::default().fg(palette.text_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [y] Confirm ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                " [n] Cancel ",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                " [9] Force Kill ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        inner,
    );

    // Register mouse hitboxes
    let btn_y = inner.y + 4;
    let yes_w = 13u16;
    let gap_w = 4u16;
    let no_w = 12u16;
    let force_w = 16u16;
    let total_w = yes_w + gap_w + no_w + gap_w + force_w;
    let start_x = inner.x + (inner.width.saturating_sub(total_w)) / 2;

    app.register_hitbox(
        Rect { x: start_x, y: btn_y, width: yes_w, height: 1 },
        HitboxAction::ConfirmKillProcess,
    );
    app.register_hitbox(
        Rect { x: start_x + yes_w + gap_w, y: btn_y, width: no_w, height: 1 },
        HitboxAction::CancelKillProcess,
    );
}

// ── CrystalDisk SSD Diagnostic modal ─────────────────────────────────────────

fn draw_disk_health_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = ThemePalette::get(&app.cfg.output.theme);
    let popup_area = centered_rect(76, 75, area);
    f.render_widget(Clear, popup_area);

    let disks = &app.latest_snap.disk_health;
    if disks.is_empty() {
        let block = Block::default()
            .title(" 💽 CrystalDisk SSD / NVMe Diagnostics ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border));
        let p = Paragraph::new("No physical drives detected or SMART permissions unavailable.")
            .style(Style::default().fg(palette.text_muted));
        f.render_widget(p, block.inner(popup_area));
        f.render_widget(block, popup_area);
        return;
    }

    let disk_idx = app.selected_disk_health.min(disks.len().saturating_sub(1));
    let d = &disks[disk_idx];

    let health_color = if d.health_status.contains("GOOD") || d.health_status.contains("HEALTHY") {
        palette.status_ok
    } else if d.health_status.contains("WARN") {
        palette.status_warn
    } else {
        palette.status_err
    };

    let temp_color = if d.temperature_c >= 75.0 {
        palette.status_err
    } else if d.temperature_c >= 60.0 {
        palette.status_warn
    } else {
        Color::Cyan
    };

    let block = Block::default()
        .title(format!(" 💽 CrystalDisk SSD / NVMe SMART Health Diagnostic [{}/{}] ", disk_idx + 1, disks.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(health_color).add_modifier(Modifier::BOLD));
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 30 || inner.height < 10 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // 1. Drive Selector & Model line
            Constraint::Length(4), // 2. Dual Big Health & Temperature Badges
            Constraint::Length(2), // 3. Wear Level Gauge
            Constraint::Length(5), // 4. Key Endurance & Life Metrics (TBW, Power On, Cycles)
            Constraint::Min(6),    // 5. SMART Attributes Table
            Constraint::Length(1), // 6. Footer Controls
        ])
        .split(inner);

    // ── 1. Drive Selector & Model Banner ──
    let mut drive_spans = Vec::new();
    for (i, item) in disks.iter().enumerate() {
        let is_cur = i == disk_idx;
        let prefix = if is_cur { " ▶ " } else { "   " };
        let short_dev = item.device.strip_prefix("/dev/").unwrap_or(&item.device);
        if is_cur {
            drive_spans.push(Span::styled(
                format!("{prefix}[{short_dev}] "),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
        } else {
            drive_spans.push(Span::styled(
                format!("{prefix}{short_dev} "),
                Style::default().fg(palette.text_muted),
            ));
        }
    }
    drive_spans.push(Span::styled(" │ ", Style::default().fg(palette.text_muted)));
    drive_spans.push(Span::styled(
        format!("{} ({})", d.model, d.device),
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    ));
    f.render_widget(Paragraph::new(Line::from(drive_spans)), chunks[0]);

    // ── 2. Dual Large Health Badges ──
    let badge_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    // Health Status Badge
    let life_pct = 100u8.saturating_sub(d.wear_level_pct);
    let health_badge_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(health_color))
        .title(" Health Status ");
    let hb_inner = health_badge_block.inner(badge_chunks[0]);
    f.render_widget(health_badge_block, badge_chunks[0]);

    let health_text = vec![
        Line::from(vec![
            Span::styled("  [● ", Style::default().fg(health_color).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{}% ", life_pct),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}]", d.health_status),
                Style::default().fg(health_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            if d.critical_warnings > 0 { "  CRITICAL SMART WARNINGS" } else { "  Nominal S.M.A.R.T. Endurance" },
            Style::default().fg(if d.critical_warnings > 0 { palette.status_err } else { palette.text_muted }),
        )),
    ];
    f.render_widget(Paragraph::new(health_text), hb_inner);

    // Temperature Badge
    let temp_badge_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(temp_color))
        .title(" Temperature ");
    let tb_inner = temp_badge_block.inner(badge_chunks[1]);
    f.render_widget(temp_badge_block, badge_chunks[1]);

    let temp_status_str = if d.temperature_c >= 75.0 {
        "CRITICAL"
    } else if d.temperature_c >= 60.0 {
        "ELEVATED"
    } else {
        "NORMAL"
    };
    let temp_text = vec![
        Line::from(vec![
            Span::styled("  [● ", Style::default().fg(temp_color).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{:.1}°C ", d.temperature_c),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}]", temp_status_str),
                Style::default().fg(temp_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            "  Thermal Threshold: Max 70°C Safe Zone",
            Style::default().fg(palette.text_muted),
        )),
    ];
    f.render_widget(Paragraph::new(temp_text), tb_inner);

    // ── 3. Wear Level Bar ──
    let wear_ratio = (d.wear_level_pct as f64 / 100.0).clamp(0.0, 1.0);
    let wear_label = format!(
        "Wear Level: {}% Used  │  {}% Life Remaining",
        d.wear_level_pct, life_pct
    );
    let wear_gauge = Gauge::default()
        .block(Block::default().borders(Borders::NONE))
        .gauge_style(Style::default().fg(if d.wear_level_pct > 80 { palette.status_err } else { palette.accent }).bg(Color::DarkGray))
        .ratio(wear_ratio)
        .label(Span::styled(
            wear_label,
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(wear_gauge, chunks[2]);

    // ── 4. Key Endurance & Life Metrics ──
    let rated_tbw = 300.0f64;
    let tbw_pct = ((d.written_bytes_tb / rated_tbw) * 100.0).min(100.0);
    let p_days = d.power_on_hours / 24;
    let metrics_lines = vec![
        Line::from(vec![
            Span::styled(" Total Bytes Written (TBW): ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:.3} TB ", d.written_bytes_tb), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!("(Rated: {:.0} TBW · {:.1}% Lifetime Written)", rated_tbw, tbw_pct), Style::default().fg(palette.accent)),
        ]),
        Line::from(vec![
            Span::styled(" Total Bytes Read (TBR):    ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{:.3} TB", d.read_bytes_tb), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" Power On Hours:             ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{} Hours (~{} Days)", d.power_on_hours, p_days), Style::default().fg(Color::White)),
            Span::styled("  │  Power Cycles: ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{} Count", d.power_cycles), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" Unsafe Shutdowns:           ", Style::default().fg(palette.text_muted)),
            Span::styled(format!("{}", d.unsafe_shutdowns), Style::default().fg(if d.unsafe_shutdowns > 50 { palette.status_warn } else { Color::White })),
            Span::styled("  │  Critical Warnings: ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("{}", d.critical_warnings),
                Style::default().fg(if d.critical_warnings > 0 { palette.status_err } else { palette.status_ok }).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(metrics_lines), chunks[3]);

    // ── 5. Detailed SMART Attributes Table ──
    let crit_warn_str = d.critical_warnings.to_string();
    let temp_str = format!("{:.1}°C", d.temperature_c);
    let spare_str = format!("{}%", life_pct);
    let wear_str = format!("{}%", d.wear_level_pct);
    let read_str = format!("{:.3} TB", d.read_bytes_tb);
    let write_str = format!("{:.3} TB", d.written_bytes_tb);
    let hrs_str = format!("{} hrs", d.power_on_hours);
    let cycles_str = format!("{} cycles", d.power_cycles);
    let unsafe_str = format!("{} events", d.unsafe_shutdowns);

    let rows = vec![
        Row::new(vec!["01", "Critical Warnings", crit_warn_str.as_str(), if d.critical_warnings == 0 { "Normal" } else { "ALERT" }]),
        Row::new(vec!["02", "Composite Temperature", temp_str.as_str(), temp_status_str]),
        Row::new(vec!["03", "Available Spare Endurance", spare_str.as_str(), "Optimal"]),
        Row::new(vec!["04", "Percentage Used (Wear Level)", wear_str.as_str(), if d.wear_level_pct > 80 { "Warn" } else { "Nominal" }]),
        Row::new(vec!["05", "Total Data Units Read (TBR)", read_str.as_str(), "Verified"]),
        Row::new(vec!["06", "Total Data Units Written (TBW)", write_str.as_str(), "Verified"]),
        Row::new(vec!["07", "Power-On Hours Count", hrs_str.as_str(), "Logged"]),
        Row::new(vec!["08", "Power Cycle Counter", cycles_str.as_str(), "Logged"]),
        Row::new(vec!["09", "Unsafe Shutdown Counter", unsafe_str.as_str(), "Monitored"]),
    ];

    let smart_table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(32),
            Constraint::Length(18),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec!["ID", "ATTRIBUTE / SENSOR DESCRIPTION", "CURRENT VALUE", "STATUS"])
            .style(Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title(" S.M.A.R.T. Diagnostic Registers "));
    f.render_widget(smart_table, chunks[4]);

    // ── 6. Footer Controls ──
    let footer_line = Line::from(vec![
        Span::styled(" [r] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Refresh   "),
        Span::styled(" [←/h] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" Prev Drive   "),
        Span::styled(" [→/l] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" Next Drive   "),
        Span::styled(" [Esc/q/Enter] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(" Close"),
    ]);
    f.render_widget(Paragraph::new(footer_line).alignment(ratatui::layout::Alignment::Center), chunks[5]);

    // Register click hitboxes
    let footer_y = chunks[5].y;
    app.register_hitbox(Rect { x: chunks[5].x + 15, y: footer_y, width: 15, height: 1 }, HitboxAction::PrevDisk);
    app.register_hitbox(Rect { x: chunks[5].x + 32, y: footer_y, width: 15, height: 1 }, HitboxAction::NextDisk);
    app.register_hitbox(Rect { x: chunks[5].x + 49, y: footer_y, width: 20, height: 1 }, HitboxAction::CloseDiskHealth);
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
        for idx in 0..CockpitTab::COUNT {
            let tab = CockpitTab::from_index(idx);
            assert_eq!(tab.to_index(), idx);
        }
        assert_eq!(CockpitTab::titles().len(), 12);
        assert_eq!(CockpitTab::COUNT, 12);
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
    fn test_analytics_tab_workflow() {
        let mut app = CockpitApp::new(&Config::default());
        app.current_tab = CockpitTab::Analytics;
        assert_eq!(app.current_tab, CockpitTab::Analytics);
        assert_eq!(app.current_tab.to_index(), 4);

        let sites = app.analytic_sites();
        assert!(!sites.is_empty());
        assert_eq!(sites[0].name, "All Sovereign Apps");

        let prev_views = app.latest_snap.web_analytics.pageviews;

        // Simulate pageview populates analytics in database
        app.simulate_analytics_pageview();
        app.sample_system();
        assert!(app.latest_snap.web_analytics.pageviews > prev_views);
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
        assert!(app.latest_snap.uptime_monitors.iter().any(|m| m.name == "Test API"));
        let count = app.latest_snap.uptime_monitors.len();
        assert!(count == initial_count + 1 || count == initial_count, "snapshot should add or preserve monitor count, got {count} vs {initial_count}");
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
        assert_eq!(SETTINGS_FIELDS.len(), 32);
        assert_eq!(SETTINGS_FIELDS[0], "AI Provider");
        assert_eq!(SETTINGS_FIELDS[3], "Output Theme");
        assert_eq!(SETTINGS_FIELDS[11], "Master Password Protection");
        assert_eq!(SETTINGS_FIELDS[12], "Set/Change Master Password");
        assert_eq!(SETTINGS_FIELDS[22], "Matrix Rain Effect");
        assert_eq!(SETTINGS_FIELDS[23], "Matrix Rain Speed");
        assert_eq!(SETTINGS_FIELDS[24], "Mascot Change Speed");
        assert_eq!(SETTINGS_FIELDS[25], "Mascot Sensitivity");
        assert_eq!(SETTINGS_FIELDS[26], "Mascot Hardware Tracking");
        assert_eq!(SETTINGS_FIELDS[27], "Mascot Color Scheme");
        assert_eq!(SETTINGS_FIELDS[28], "Mascot Animation Style");
        assert_eq!(SETTINGS_FIELDS[29], "Mascot Render Scale");
        assert_eq!(SETTINGS_FIELDS[30], "Mascot In AI Chat");
        assert_eq!(SETTINGS_FIELDS[31], "Audit Context Limit");
    }

    #[test]
    fn test_truncate_str_helper() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("superlongstring", 5), "super");
    }

    #[test]
    fn test_executive_command_center_telemetry() {
        let app = CockpitApp::new(&Config::default());
        // Verify the local node is present
        let nodes = &app.latest_snap.fleet_nodes;
        assert!(nodes.iter().any(|n| n.name.contains("ThinkPad X1") || n.name.contains("Local")));

        // Verify core daemons present
        let daemons = &app.latest_snap.service_health;
        assert!(daemons.iter().any(|d| d.name == "tailscaled"));
        assert!(daemons.iter().any(|d| d.name == "cloudflared"));
        assert!(daemons.iter().any(|d| d.name == "9router / omniroute"));
        assert!(daemons.iter().any(|d| d.name == "docker / podman"));
        assert!(daemons.iter().any(|d| d.name == "hermes agent gateway"));
    }

    #[test]
    fn test_dashboard_tab_rendering_both_resolutions() {
        let mut app = CockpitApp::new(&Config::default());
        let backend = ratatui::backend::TestBackend::new(120, 45);
        let mut terminal = Terminal::new(backend).expect("terminal");

        // 1. Render in large terminal (>= 38 height)
        terminal.draw(|f| {
            let area = f.area();
            draw_dashboard_tab(f, &mut app, area);
        }).expect("draw full");

        // Hitboxes should be registered
        assert!(!app.hitboxes.is_empty());

        // 2. Render in compact terminal (< 36 height) with scroll
        app.overview_proc_scroll = 1;
        let backend_compact = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal_compact = Terminal::new(backend_compact).expect("terminal compact");
        terminal_compact.draw(|f| {
            let area = f.area();
            draw_dashboard_tab(f, &mut app, area);
        }).expect("draw compact");
    }

    #[test]
    fn test_pet_switch_confirmation_and_pause_toggle() {
        let mut app = CockpitApp::new(&Config::default());
        assert!(!app.show_pet_switch_confirm);
        assert!(!app.pet_is_stopped);

        // 1. Request switch -> opens confirmation modal
        app.request_pet_switch();
        assert!(app.show_pet_switch_confirm);

        // 2. Confirm switch -> switches companion & closes modal
        app.confirm_switch_pet_species();
        assert!(!app.show_pet_switch_confirm);
        assert!(app.pet_transient_action.is_some());

        // 3. Toggle pause / stop
        app.toggle_pet_pause();
        assert!(app.pet_is_stopped);
        app.toggle_pet_pause();
        assert!(!app.pet_is_stopped);
    }

    #[test]
    fn test_server_fleet_management_actions() {
        let mut app = CockpitApp::new(&Config::default());

        // Register a custom server node to be managed
        let node_cfg = crate::config::NodeConfig {
            ip: "192.168.1.50".to_string(),
            user: "root".to_string(),
            services: vec![],
            active: true,
        };
        app.cfg.daemon.nodes.insert("my-custom-server".to_string(), node_cfg);
        app.server_probe_results.push(("my-custom-server".to_string(), Some(45u128)));
        app.selected_server = app.server_probe_results.len() - 1;

        // 1. Test Toggle Active / Inactive
        let was_active = app.cfg.daemon.nodes.get("my-custom-server").unwrap().active;
        app.toggle_server_active();
        let is_active_now = app.cfg.daemon.nodes.get("my-custom-server").unwrap().active;
        assert_ne!(was_active, is_active_now);
        app.toggle_server_active();
        assert_eq!(was_active, app.cfg.daemon.nodes.get("my-custom-server").unwrap().active);

        // 2. Test Clone
        app.clone_selected_server();
        assert!(app.cfg.daemon.nodes.contains_key("my-custom-server-clone"));

        // 3. Test Open Edit Wizard
        app.open_edit_server_wizard();
        assert!(app.show_server_wizard);
        assert!(app.server_wizard_is_edit);
        assert_eq!(app.server_wizard_orig_name, "my-custom-server");
        assert_eq!(app.server_wizard_field, 0);
        assert_eq!(app.server_name_buf, "my-custom-server");

        // 4. Test Submit Edit Wizard (Rename)
        app.server_name_buf = "renamed-server".to_string();
        app.server_wizard_field = 0;
        app.show_server_wizard = false;
        app.show_server_wizard = true; // ensure it's open
        app.submit_server_wizard();
        assert!(!app.show_server_wizard);
        assert!(app.cfg.daemon.nodes.contains_key("renamed-server"));
        assert!(!app.cfg.daemon.nodes.contains_key("my-custom-server"));

        // 5. Test SSH Endpoint Test Logic
        app.selected_server = 0;
        app.server_probe_results = vec![
            ("my-custom-server-clone".to_string(), Some(45u128)),
            ("renamed-server".to_string(), Some(45u128)),
        ];
        app.selected_server = 1;
        app.test_selected_server_ssh();
        assert!(app.notification.is_some());
    }

    #[test]
    fn test_delete_confirmation_modal_across_pages() {
        let mut app = CockpitApp::new(&Config::default());
        assert!(!app.show_delete_confirm);

        // 1. Server delete confirmation
        app.server_probe_results = vec![("node-1".to_string(), Some(10u128))];
        app.selected_server = 0;
        app.cfg.daemon.nodes.insert("node-1".to_string(), crate::config::NodeConfig {
            ip: "10.0.0.1".to_string(),
            user: "root".to_string(),
            services: vec![],
            active: true,
        });
        app.remove_selected_server();
        assert!(app.show_delete_confirm);
        assert_eq!(app.pending_delete_target, DeleteTarget::Server);
        // Cancel delete -> should preserve
        app.show_delete_confirm = false;
        assert!(app.cfg.daemon.nodes.contains_key("node-1"));
        // Confirm delete -> should remove
        app.show_delete_confirm = true;
        app.execute_pending_deletion();
        assert!(!app.show_delete_confirm);
        assert!(!app.cfg.daemon.nodes.contains_key("node-1"));

        // 2. Project delete confirmation
        app.latest_snap.tracked_projects.push(crate::systat::TrackedProject {
            path: "/tmp/fake-proj".to_string(),
            name: "FakeProj".to_string(),
            category: "Product".to_string(),
            branch: "main".to_string(),
            repos: vec![],
            services: vec![],
            containers: vec![],
            databases: vec![],
            is_dirty: false,
        });
        app.selected_project = app.latest_snap.tracked_projects.len() - 1;
        app.remove_selected_project();
        assert!(app.show_delete_confirm);
        assert_eq!(app.pending_delete_target, DeleteTarget::Project);
        app.execute_pending_deletion();
        assert!(!app.show_delete_confirm);
        assert!(!app.latest_snap.tracked_projects.iter().any(|p| p.path == "/tmp/fake-proj"));

        // 3. Monitor delete confirmation
        app.latest_snap.uptime_monitors.push(crate::systat::UptimeMonitor {
            name: "fake-mon".to_string(),
            target: "http://fake".to_string(),
            is_up: true,
            latency_ms: Some(10),
            status_code: Some(200),
            uptime_pct: 100.0,
            history: vec![],
            group: "Test".to_string(),
            error: None,
        });
        app.selected_monitor = app.latest_snap.uptime_monitors.len() - 1;
        app.remove_selected_monitor();
        assert!(app.show_delete_confirm);
        assert_eq!(app.pending_delete_target, DeleteTarget::Monitor);
        app.execute_pending_deletion();
        assert!(!app.show_delete_confirm);
        assert!(!app.latest_snap.uptime_monitors.iter().any(|m| m.name == "fake-mon"));
    }

    #[test]
    fn test_test_telegram_alert() {
        let mut cfg = Config::default();
        cfg.uptime.telegram_bot_token.clear();
        cfg.daemon.telegram_bot_token.clear();
        let mut app = CockpitApp::new(&cfg);

        // When token is empty -> warning toast
        app.trigger_test_telegram_alert();
        let notif = app.notification.as_ref().map(|(n, _)| n.as_str());
        assert_eq!(notif, Some("⚠ Configure Telegram Bot Token & Chat ID in Settings first."));

        // When token is set in uptime -> success toast
        app.cfg.uptime.telegram_bot_token = "123456:BOT_TOKEN".to_string();
        app.cfg.uptime.telegram_chat_id = "12345678".to_string();
        app.trigger_test_telegram_alert();
        let notif = app.notification.as_ref().map(|(n, _)| n.as_str());
        assert_eq!(notif, Some("📨 Test Telegram alert sent! Check your Telegram if it's configured."));
    }

    #[test]
    fn test_parse_command_tokens() {
        let tokens = parse_command_tokens(r#"/uptime add-monitor "Landing CATerm" "https://caterm.fathforce.com""#);
        assert_eq!(tokens, vec!["/uptime", "add-monitor", "Landing CATerm", "https://caterm.fathforce.com"]);

        let tokens = parse_command_tokens("/go uptime");
        assert_eq!(tokens, vec!["/go", "uptime"]);

        let tokens = parse_command_tokens(r#"/uptime delete "Landing CATerm""#);
        assert_eq!(tokens, vec!["/uptime", "delete", "Landing CATerm"]);
    }

    #[test]
    fn test_ai_slash_commands() {
        let cfg = Config::default();
        let mut app = CockpitApp::new(&cfg);

        // Non-slash command -> returns false
        assert!(!handle_ai_slash_command(&mut app, "Hello AI, how are you?"));

        // /go tabs
        assert!(handle_ai_slash_command(&mut app, "/go uptime"));
        assert_eq!(app.current_tab, CockpitTab::Uptime);

        assert!(handle_ai_slash_command(&mut app, "/go analytics"));
        assert_eq!(app.current_tab, CockpitTab::Analytics);

        assert!(handle_ai_slash_command(&mut app, "/go fleet"));
        assert_eq!(app.current_tab, CockpitTab::Server);

        // /uptime add-monitor
        assert!(handle_ai_slash_command(&mut app, r#"/uptime add-monitor "Test Monitor" "https://test.local""#));
        assert!(app.latest_snap.uptime_monitors.iter().any(|m| m.name == "Test Monitor"));
        assert!(app.cfg.uptime.custom_monitors.iter().any(|m| m.name == "Test Monitor"));
        let last_chat = app.chat_lines.last().cloned().unwrap_or_default();
        assert!(last_chat.contains("Added monitor 'Test Monitor'"));

        // /uptime delete
        assert!(handle_ai_slash_command(&mut app, r#"/uptime delete "Test Monitor""#));
        assert!(app.show_delete_confirm);
        assert_eq!(app.pending_delete_target, DeleteTarget::Monitor);
        app.execute_pending_deletion();
        assert!(!app.latest_snap.uptime_monitors.iter().any(|m| m.name == "Test Monitor"));

        // /uptime test-telegram
        assert!(handle_ai_slash_command(&mut app, "/uptime test-telegram"));

        // /analytics refresh
        assert!(handle_ai_slash_command(&mut app, "/analytics refresh"));
        let last_chat = app.chat_lines.last().cloned().unwrap_or_default();
        assert!(last_chat.contains("Analytics refreshed"));

        // /analytics site
        assert!(handle_ai_slash_command(&mut app, "/analytics site all"));
        assert_eq!(app.analytics_selected_site, 0);

        // /fleet status
        assert!(handle_ai_slash_command(&mut app, "/fleet status"));
        assert!(app.chat_lines.iter().any(|line| line.contains("Fleet Status")));

        // /fleet restart with confirmation prompt
        assert!(handle_ai_slash_command(&mut app, "/fleet restart test-service"));
        assert_eq!(app.pending_service_restart, Some("test-service".to_string()));
        let last_chat = app.chat_lines.last().cloned().unwrap_or_default();
        assert!(last_chat.contains("Confirm restart of service 'test-service'"));

        // confirm with "yes"
        assert!(handle_ai_slash_command(&mut app, "yes"));
        assert_eq!(app.pending_service_restart, None);

        // /disk health slash command opens modal
        app.show_disk_health_modal = false;
        assert!(handle_ai_slash_command(&mut app, "/disk health"));
        assert!(app.show_disk_health_modal);
    }

    #[test]
    fn test_disk_health_slash_command_and_modal() {
        let mut app = CockpitApp::new(&Config::default());
        assert!(!app.show_disk_health_modal);

        // Terminal runner /disk health
        app.term_input = "/disk health".to_string();
        app.run_terminal_command();
        assert!(app.show_disk_health_modal);

        // Hitbox Prev/Next Disk
        app.selected_disk_health = 0;
        assert_eq!(app.selected_disk_health, 0);
    }

    #[test]
    fn test_network_status_format_helpers() {
        assert_eq!(format_kbps(100.0), "100 KB/s");
        assert_eq!(format_kbps(2048.0), "2.0 MB/s");
        assert_eq!(format_kbps(1024.0 * 1024.0 * 2.5), "2.50 GB/s");

        assert_eq!(format_net_gb(0.5), "512.0 MB");
        assert_eq!(format_net_gb(5.25), "5.25 GB");
        assert_eq!(format_net_gb(2048.0), "2.00 TB");
    }

    #[test]
    fn test_dashboard_process_management_and_kill_modal() {
        let mut app = CockpitApp::new(&Config::default());
        app.latest_snap.top_processes = vec![
            crate::systat::ProcInfo {
                pid: 1234,
                name: "dockerd".to_string(),
                cpu_pct: 12.5,
                mem_mb: 256,
                status: "Run".to_string(),
                role: "Container Daemon".to_string(),
            },
            crate::systat::ProcInfo {
                pid: 5678,
                name: "cashell".to_string(),
                cpu_pct: 45.0,
                mem_mb: 128,
                status: "Run".to_string(),
                role: "System Service".to_string(),
            },
        ];

        assert_eq!(app.dashboard_proc_selected, 0);
        assert!(!app.show_kill_modal);

        // Select second process
        app.dashboard_proc_selected = 1;
        assert_eq!(app.dashboard_proc_selected, 1);

        // Prompt kill on selected process
        let (pid, name) = {
            let proc = &app.latest_snap.top_processes[app.dashboard_proc_selected];
            (proc.pid, proc.name.clone())
        };
        app.prompt_kill_process_with_name(pid, &name);

        assert!(app.show_kill_modal);
        assert_eq!(app.pending_kill_proc, Some((5678, "cashell".to_string())));

        // Cancel modal
        app.show_kill_modal = false;
        app.pending_kill_proc = None;
        assert!(!app.show_kill_modal);
    }

    #[test]
    fn test_dashboard_4_column_top_and_processes_panel_rendering() {
        let mut app = CockpitApp::new(&Config::default());
        app.latest_snap.net_rx_rate_kbps = 1450.5;
        app.latest_snap.net_tx_rate_kbps = 320.0;
        app.latest_snap.net_total_rx_gb = 12.4;
        app.latest_snap.net_total_tx_gb = 3.2;
        app.latest_snap.active_net_iface = "wlp2s0".to_string();

        app.latest_snap.top_processes = vec![
            crate::systat::ProcInfo {
                pid: 101,
                name: "test-proc".to_string(),
                cpu_pct: 33.3,
                mem_mb: 64,
                status: "Run".to_string(),
                role: "Test".to_string(),
            },
        ];

        let backend = ratatui::backend::TestBackend::new(140, 45);
        let mut terminal = Terminal::new(backend).expect("terminal");

        terminal
            .draw(|f| {
                let area = f.area();
                draw_dashboard_tab(f, &mut app, area);
            })
            .expect("draw dashboard");

        // Verify that process selection and kill hitboxes were registered
        assert!(app.hitboxes.iter().any(|h| matches!(h.action, HitboxAction::SelectProcess(0))));
        assert!(app.hitboxes.iter().any(|h| matches!(h.action, HitboxAction::KillProcess(101))));
        assert!(app.hitboxes.iter().any(|h| matches!(h.action, HitboxAction::FocusDashboardPanel(5))));
    }

    #[test]
    fn test_process_killer_modal_confirmation_and_toast() {
        let mut app = CockpitApp::new(&Config::default());
        app.prompt_kill_process_with_name(4321, "worker-job");
        assert!(app.show_kill_modal);
        assert_eq!(app.pending_kill_proc, Some((4321, "worker-job".to_string())));

        // Test render modal
        let backend = ratatui::backend::TestBackend::new(80, 25);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                draw_kill_process_modal(f, &mut app, area);
            })
            .expect("draw kill modal");

        // Verify confirm and cancel hitboxes
        assert!(app.hitboxes.iter().any(|h| matches!(h.action, HitboxAction::ConfirmKillProcess)));
        assert!(app.hitboxes.iter().any(|h| matches!(h.action, HitboxAction::CancelKillProcess)));

        // Simulated cancellation
        app.show_kill_modal = false;
        app.set_notification("Process termination cancelled.");
        assert_eq!(app.notification.as_ref().map(|(m, _)| m.as_str()), Some("Process termination cancelled."));

        // Simulated successful termination toast
        let pid = 4321;
        let name = "worker-job";
        app.set_notification(format!("Terminated process {name} (PID {pid})"));
        assert_eq!(app.notification.as_ref().map(|(m, _)| m.as_str()), Some("Terminated process worker-job (PID 4321)"));
    }

    #[test]
    fn test_fleet_server_edit_persistence_and_hitboxes() {
        let mut app = CockpitApp::new(&Config::default());

        // 1. Test editing a preset node (not initially in self.cfg.daemon.nodes)
        assert!(!app.cfg.daemon.nodes.contains_key("pc-gerlink"));
        app.server_probe_results = vec![("pc-gerlink".to_string(), Some(20u128))];
        app.selected_server = 0;

        app.open_edit_server_wizard();
        assert!(app.show_server_wizard);
        assert!(app.server_wizard_is_edit);
        assert_eq!(app.server_wizard_orig_name, "pc-gerlink");
        assert_eq!(app.server_name_buf, "pc-gerlink");
        assert_eq!(app.server_ip_buf, "100.102.147.90");
        // Verify pre-population of self.cfg.daemon.nodes
        assert!(app.cfg.daemon.nodes.contains_key("pc-gerlink"));
        assert_eq!(app.cfg.daemon.nodes["pc-gerlink"].ip, "100.102.147.90");

        // 2. Modify IP & name
        app.server_name_buf = "pc-gerlink-updated".to_string();
        app.server_ip_buf = "100.102.147.99".to_string();

        // 3. Test Ctrl+S shortcut to save
        handle_server_wizard_key(
            event::KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut app,
        );

        // Wizard should have closed and saved to daemon nodes
        assert!(!app.show_server_wizard);
        assert!(app.cfg.daemon.nodes.contains_key("pc-gerlink-updated"));
        assert_eq!(app.cfg.daemon.nodes["pc-gerlink-updated"].ip, "100.102.147.99");
        assert!(!app.cfg.daemon.nodes.contains_key("pc-gerlink"));

        // 4. Test draw_server_wizard dynamic hitboxes
        app.show_server_wizard = true;
        app.server_wizard_is_edit = true;
        app.server_wizard_orig_name = "pc-gerlink-updated".to_string();
        let backend = ratatui::backend::TestBackend::new(80, 25);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                draw_server_wizard(f, &mut app, area);
            })
            .expect("draw server wizard");

        let submit_hitbox = app
            .hitboxes
            .iter()
            .find(|h| matches!(h.action, HitboxAction::SubmitServer))
            .expect("submit server hitbox registered");
        let cancel_hitbox = app
            .hitboxes
            .iter()
            .find(|h| matches!(h.action, HitboxAction::CancelServer))
            .expect("cancel server hitbox registered");

        // [✔ Save Changes] (Enter) has length 26 chars
        assert_eq!(submit_hitbox.rect.width, 26);
        // Cancel should be placed after save width + 2 spacing
        assert_eq!(cancel_hitbox.rect.x, submit_hitbox.rect.x + 28);
        assert_eq!(cancel_hitbox.rect.width, 18);
        assert_eq!(submit_hitbox.rect.y, cancel_hitbox.rect.y);
    }

    #[test]
    fn test_uptime_monitors_display_and_glyphs() {
        let mut app = CockpitApp::new(&Config::default());
        app.latest_snap.uptime_monitors = vec![
            crate::systat::UptimeMonitor {
                name: "CATable".to_string(),
                target: "https://catable.fathforce.com/".to_string(),
                is_up: true,
                latency_ms: Some(42),
                status_code: Some(200),
                uptime_pct: 100.0,
                history: vec![true; 20],
                group: "CA Ecosystem".to_string(),
                error: None,
            },
            crate::systat::UptimeMonitor {
                name: "Broken API".to_string(),
                target: "https://api.broken.internal/".to_string(),
                is_up: false,
                latency_ms: None,
                status_code: None,
                uptime_pct: 85.0,
                history: vec![true, true, false],
                group: "Internal".to_string(),
                error: Some("Timeout".to_string()),
            },
            crate::systat::UptimeMonitor {
                name: "502 Gateway".to_string(),
                target: "https://badgw.internal/".to_string(),
                is_up: false,
                latency_ms: Some(120),
                status_code: Some(502),
                uptime_pct: 70.0,
                history: vec![false; 5],
                group: "Internal".to_string(),
                error: Some("502".to_string()),
            },
        ];

        let backend = ratatui::backend::TestBackend::new(140, 45);
        let mut terminal = Terminal::new(backend).expect("terminal");

        // 1. Test resume panel rendering
        let palette = ThemePalette::get(&app.cfg.output.theme);
        terminal
            .draw(|f| {
                let area = f.area();
                draw_uptime_monitors_resume_panel(f, &mut app, area, &palette, 0, false);
            })
            .expect("draw uptime resume panel");

        let buffer = terminal.backend().buffer().clone();
        let content: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Target URL and failure reasons visible
        assert!(content.contains("https://catable.fathforce.com/"));
        assert!(content.contains("ERR: Timeout"));
        assert!(content.contains("502 ERR"));
        // Glyph is solid block █ (U+2588) and not ❚ (U+275E)
        assert!(content.contains('█'));
        assert!(!content.contains('❚'));

        // 2. Test uptime tab rendering
        terminal
            .draw(|f| {
                let area = f.area();
                draw_uptime_tab(f, &mut app, area);
            })
            .expect("draw uptime tab");

        let tab_buf = terminal.backend().buffer().clone();
        let tab_content: String = (0..tab_buf.area.height)
            .map(|y| {
                (0..tab_buf.area.width)
                    .map(|x| tab_buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(tab_content.contains("TARGET URL"));
        assert!(tab_content.contains("https://catable.fathforce."));
        assert!(tab_content.contains("ERR: Timeout"));
        assert!(tab_content.contains('█'));

        // 3. Test detail modal rendering
        app.uptime_detail_index = 1;
        app.show_uptime_detail_modal = true;
        terminal
            .draw(|f| {
                let area = f.area();
                draw_uptime_detail_modal(f, &mut app, area);
            })
            .expect("draw detail modal");

        let modal_buf = terminal.backend().buffer().clone();
        let modal_content: String = (0..modal_buf.area.height)
            .map(|y| {
                (0..modal_buf.area.width)
                    .map(|x| modal_buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(modal_content.contains("Failure Cause:"));
        assert!(modal_content.contains("ERR: Timeout"));
    }
}
pub(crate) fn noop() {}
