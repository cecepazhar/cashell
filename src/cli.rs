//! CLI argument definitions via clap derive.

use crate::commands::{auth::AuthCommand, config::ConfigCommand, product::ProductCommand};
use clap::{Parser, Subcommand};
#[derive(Parser, Debug)]
#[command(name = "cashell")]
#[command(about = "CA ecosystem CLI — universal shell for CATerm & tools", long_about = None)]
pub struct Cashell {
    /// Path to config file (default: ~/.config/cashell/config.toml)
    #[arg(short = 'c', long, global = true)]
    pub config_file: Option<std::path::PathBuf>,

    /// Output format: text (tty) | json (pipe) | auto (detect)
    #[arg(short = 'o', long, global = true, default_value = "auto")]
    pub output_format: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authentication & token management
    Auth(AuthCommand),

    /// CA products & marketplace info
    Product(ProductCommand),

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

    /// Upload assets to Cloudflare R2 (REQ-P4-01)
    R2(R2Command),

    /// Sync or query the markdown knowledge base (REQ-P4-02)
    Vault(VaultCommand),
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
