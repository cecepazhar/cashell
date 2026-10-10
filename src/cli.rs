//! CLI argument definitions via clap derive.

use crate::commands::auth::AuthCommand;
use crate::commands::config::ConfigCommand;
use crate::commands::product::ProductCommand;
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
}
