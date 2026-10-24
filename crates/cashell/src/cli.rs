//! CLI argument definitions via clap derive.

use crate::commands::{
    auth::AuthCommand, chat::ChatCommand, cockpit::CockpitCommand, config::ConfigCommand,
    daemon::DaemonCommand, product::ProductCommand, project::ProjectCommand, server::ServerCommand,
};
use clap::{Parser, Subcommand};
#[derive(Parser, Debug)]
#[command(name = "cashell", version)]
#[command(about = "CA ecosystem CLI — universal shell for CATerm & tools", long_about = None)]
pub struct Cashell {
    /// Path to config file (default: ~/.config/cashell/config.toml)
    #[arg(short = 'c', long, global = true)]
    pub config_file: Option<std::path::PathBuf>,

    /// Output format: text (tty) | json (pipe) | auto (detect)
    #[arg(short = 'o', long, global = true, default_value = "auto")]
    pub output_format: String,

    /// Suppress the startup banner
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authentication & token management
    Auth(AuthCommand),

    /// CA products & marketplace info
    Product(ProductCommand),

    /// Project workspaces: inspect and manage workspaces (repos, containers, services, DBs)
    Project(ProjectCommand),

    /// Print version & build info
    Version(crate::commands::version::VersionCommand),

    /// Config file operations
    Config(ConfigCommand),

    /// Git repository operations
    Git(GitCommand),

    /// Show git status for all tracked repositories
    #[command(name = "git-status")]
    GitStatus(GitStatusCommand),

    /// Deploy an application via Coolify
    Deploy(DeployCommand),

    /// System health diagnostics (REQ-P2-01)
    Doctor(DoctorCommand),

    /// AI assistant: chat, analyze, review (REQ-P2-02)
    Ai(AiCommand),

    /// AI-powered codebase auditor
    Audit(AuditCommand),

    /// Apply AI fixes for audit findings
    Fix(FixCommand),

    /// Review git diff with AI
    Review(ReviewCommand),

    /// Upload assets to Cloudflare R2 (REQ-P4-01)
    R2(R2Command),

    /// Sync or query the markdown knowledge base (REQ-P4-02)
    Vault(VaultCommand),

    /// Interactive AI Chat TUI (dual panel with mouse support)
    Chat(ChatCommand),

    /// Server management: stats, db, firewall, docker, ai
    Server(ServerCommand),

    /// Personal Cloud Cockpit TUI (btop + Dokploy + Settings + Hana AI)
    Cockpit(CockpitCommand),

    /// 24/7 fleet & health monitoring daemon & HTTP API
    Daemon(DaemonCommand),

    /// Pro multi-device zero-knowledge sync: login, status, sync, push, pull, devices, logout
    Pro(ProCommand),

    /// Probe and display live uptime status of all CA Ecosystem endpoints and fleet nodes
    Uptime(crate::commands::uptime::UptimeCommand),

    /// View sovereign web analytics & traffic summary across CA ecosystem sites
    Analytics(crate::commands::analytics::AnalyticsCommand),
}

// ── Pro Command CLI types ────────────────────────────────────────────────────

/// Top-level `cashell pro` CLI command.
#[derive(Parser, Debug, Clone)]
#[command(name = "pro", about = "CAShell Pro: multi-device zero-knowledge GCC sync & device management")]
pub struct ProCommand {
    #[command(subcommand)]
    pub mode: ProMode,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProMode {
    /// Log in / activate a Pro account or 7-day trial
    Login(ProLoginArgs),

    /// Show current Pro session status (HWID, tier, token validity, device count)
    Status(ProStatusArgs),

    /// Full GCC sync cycle: pull → resolve → push the zero-knowledge vault
    Sync(ProSyncArgs),

    /// Push the encrypted local vault to the GCC REST API
    Push(ProPushArgs),

    /// Pull & decrypt the remote zero-knowledge vault from the GCC REST API
    Pull(ProPullArgs),

    /// Manage registered Pro devices (list / remove / rename)
    Devices(ProDevicesArgs),

    /// Revoke the Pro session and mark this device inactive
    Logout(ProLogoutArgs),
}

/// `cashell pro login` arguments.
#[derive(Parser, Debug, Clone)]
pub struct ProLoginArgs {
    /// Pro account email address (prompted interactively if omitted on a TTY)
    #[arg(long)]
    pub email: Option<String>,

    /// Pro license token (prompted interactively if omitted on a TTY)
    #[arg(long)]
    pub token: Option<String>,

    /// Activate a 7-day free trial instead of a paid license
    #[arg(long)]
    pub trial: bool,

    /// Override the GCC API base URL
    #[arg(long)]
    pub api_base: Option<String>,

    /// Emit machine-readable JSON output
    #[arg(long)]
    pub json: bool,
}

/// `cashell pro status` arguments.
#[derive(Parser, Debug, Clone)]
pub struct ProStatusArgs {
    /// Emit machine-readable JSON output
    #[arg(long)]
    pub json: bool,
}

/// `cashell pro sync` arguments.
#[derive(Parser, Debug, Clone)]
pub struct ProSyncArgs {
    /// Explicit vault passphrase (zero-knowledge; never transmitted to the server).
    /// Falls back to `CASHELL_PRO_PASS` env var, then interactive prompt.
    #[arg(long)]
    pub passphrase: Option<String>,

    /// Emit machine-readable JSON output
    #[arg(long)]
    pub json: bool,
}

/// `cashell pro push` arguments.
#[derive(Parser, Debug, Clone)]
pub struct ProPushArgs {
    /// Explicit vault passphrase (zero-knowledge; never transmitted to the server).
    /// Falls back to `CASHELL_PRO_PASS` env var, then interactive prompt.
    #[arg(long)]
    pub passphrase: Option<String>,

    /// Emit machine-readable JSON output
    #[arg(long)]
    pub json: bool,
}

/// `cashell pro pull` arguments.
#[derive(Parser, Debug, Clone)]
pub struct ProPullArgs {
    /// Explicit vault passphrase (zero-knowledge; never transmitted to the server).
    /// Falls back to `CASHELL_PRO_PASS` env var, then interactive prompt.
    #[arg(long)]
    pub passphrase: Option<String>,

    /// Write the decrypted vault to a file instead of stdout
    #[arg(long)]
    pub out: Option<String>,

    /// Emit machine-readable JSON output
    #[arg(long)]
    pub json: bool,
}

/// `cashell pro devices` arguments.
#[derive(Parser, Debug, Clone)]
pub struct ProDevicesArgs {
    /// What to do with the device registry
    #[command(subcommand)]
    pub action: ProDevicesAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProDevicesAction {
    /// List all registered Pro devices (default)
    List(ProDevicesListArgs),

    /// Remove a device from the `pro_state` registry
    Remove {
        /// The HWID of the device to remove
        device_id: String,
    },

    /// Rename a device in the `pro_state` registry
    Rename {
        /// The HWID of the device to rename
        device_id: String,
        /// New human-readable label
        label: String,
    },
}

/// `cashell pro devices list` arguments.
#[derive(Parser, Debug, Clone)]
pub struct ProDevicesListArgs {
    /// Emit machine-readable JSON output
    #[arg(long)]
    pub json: bool,
}

/// `cashell pro logout` arguments.
#[derive(Parser, Debug, Clone)]
pub struct ProLogoutArgs {
    /// Emit machine-readable JSON output
    #[arg(long)]
    pub json: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct R2Command {
    #[command(subcommand)]
    pub mode: R2Mode,
}

#[derive(Subcommand, Debug, Clone)]
pub enum R2Mode {
    /// Upload a local file to an R2 bucket
    Upload {
        /// Local file path
        local_path: String,

        /// R2 bucket name
        #[arg(long)]
        bucket: String,

        /// Remote key (defaults to local file name)
        #[arg(long)]
        key: Option<String>,

        /// App slug for the public edge URL
        #[arg(long)]
        app: Option<String>,

        /// Print the signed request without uploading
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct VaultCommand {
    #[command(subcommand)]
    pub mode: VaultMode,
}

#[derive(Subcommand, Debug, Clone)]
pub enum VaultMode {
    /// Atomic no-rebase pull & push on the vault
    Sync,

    /// Full-text search over vault markdown files
    Query {
        /// Search term
        term: String,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct GitCommand {
    #[command(subcommand)]
    pub mode: GitMode,
}

#[derive(Subcommand, Debug, Clone)]
pub enum GitMode {
    /// Show status for all tracked repositories
    Status(GitStatusCommand),
}

#[derive(Parser, Debug, Clone)]
pub struct GitStatusCommand {
    /// Show only changed (dirty/unpushed) repos
    #[arg(long)]
    pub changed: bool,

    /// Output in JSON format
    #[arg(long)]
    pub json: bool,

    /// Maximum depth to search for git repos (default 2, 0 = unlimited)
    #[arg(long, default_value = "2")]
    pub depth: u32,
}

#[derive(Parser, Debug, Clone)]
pub struct DeployCommand {
    /// Application slug (must be configured in deploy.apps)
    pub slug: String,

    /// Environment: production or staging
    #[arg(long, default_value = "production")]
    pub env: String,

    /// Show what would be done without executing
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct DoctorCommand {
    /// Output in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AiMode {
    /// Chat with AI (streaming on TTY, buffer in pipe)
    Chat {
        /// Prompt text (remainder of args)
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
    /// Analyze a file and return AI feedback
    Analyze {
        /// Path to file to analyze
        path: String,
    },
    /// Review staged changes or a branch diff
    Review {
        /// Review only the currently staged diff
        #[arg(long)]
        staged: bool,
        /// Review diff against this branch (default: current HEAD vs main)
        #[arg(long, default_value = "main")]
        branch: String,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct AiCommand {
    #[command(subcommand)]
    pub mode: AiMode,
}

#[derive(Parser, Debug, Clone)]
pub struct AuditCommand {
    /// Path to audit (defaults to current directory)
    pub path: Option<String>,

    /// Output in JSON format
    #[arg(long)]
    pub json: bool,

    /// Maximum number of source files to scan (default: 40)
    #[arg(long, default_value = "40")]
    pub max_files: u32,

    /// Write rendered report to a file
    #[arg(long)]
    pub out: Option<String>,

    /// Include only files matching glob (optional)
    #[arg(long)]
    pub include: Option<String>,

    /// Audit only files changed vs HEAD (git diff --name-only HEAD)
    #[arg(long)]
    pub changed: bool,

    /// Timeout in seconds per file AI call
    #[arg(long, default_value = "30")]
    pub timeout: u64,

    /// Fan the file scan out across all available CPU workers (rayon)
    #[arg(long)]
    pub turbo: bool,

    /// Cap the number of parallel workers (implies --turbo; defaults to all
    /// available CPUs)
    #[arg(long)]
    pub jobs: Option<usize>,
}

#[derive(Parser, Debug, Clone)]
pub struct ReviewCommand {
    /// Review only the currently staged diff
    #[arg(long)]
    pub staged: bool,

    /// Review diff against this branch (default: main)
    #[arg(long, default_value = "main")]
    pub branch: String,

    /// Optional path filter
    pub path: Option<String>,
}

#[derive(Parser, Debug, Clone)]
pub struct FixCommand {
    /// Path to the audit report (`audit/Audit_YYYY-MM-DD.md`). If omitted,
    /// the latest `.md` in the `audit/` directory is used.
    pub report_path: Option<String>,

    /// Explicit path to the audit report (alias for the positional arg).
    #[arg(long)]
    pub from: Option<String>,

    /// Machine-readable JSON output (no TUI)
    #[arg(long)]
    pub json: bool,

    /// Show diffs without writing any changes
    #[arg(long)]
    pub dry_run: bool,

    /// Auto-accept all prompts (CI mode, skips interactive)
    #[arg(long)]
    pub yes: bool,

    /// Comma-separated file slugs to fix (filters which files are touched).
    #[arg(long)]
    pub files: Option<String>,

    /// Timeout in seconds for the per-file AI call
    #[arg(long)]
    pub timeout: Option<u64>,
}
