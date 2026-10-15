//! Version command — build metadata & help.

use crate::output;
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug, Default)]
pub struct VersionCommand {}

pub fn run(_cmd: &VersionCommand) -> Result<()> {
    // Read version from Cargo.toml at build time
    let version = env!("CARGO_PKG_VERSION");
    let name = env!("CARGO_PKG_NAME");
    let authors = env!("CARGO_PKG_AUTHORS");
    let repo = env!("CARGO_PKG_REPOSITORY");
    let built_at = chrono::Utc::now()
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string();

    output::kv("name", name);
    output::kv("version", version);
    output::kv("author", authors.trim_end_matches(':').trim());
    output::kv("repo", repo);
    output::kv("built", &built_at);
    output::success("cashell ready");
    Ok(())
}
