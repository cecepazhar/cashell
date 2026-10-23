//! Subcommand implementations.

pub mod audit;
pub mod auth;
pub mod chat;
pub mod cockpit;
pub mod config;
pub mod daemon;
pub mod deploy;
pub mod doctor;
pub mod fix;
pub mod git;
pub mod git_status;
pub mod product;
pub mod product_new;
pub mod project;
pub mod pro;
pub mod r2;
pub mod review;
pub mod server;
pub mod vault;
pub mod version;

use crate::config::Config;
use anyhow::Result;
use std::path::Path;

/// Load config honoring an optional override path.
pub fn load_config(path: &Option<std::path::PathBuf>) -> Result<Config> {
    Config::load(path.as_deref().map(Path::new))
}
