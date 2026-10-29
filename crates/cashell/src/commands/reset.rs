//! `cashell reset` — Factory reset CLI.
//!
//! Wipes all local state (config + SQLite database + cache), leaving a clean slate.
//! Run once on a fresh server or when you want to start over.
//!
//! Usage:
//!   cashell reset          # interactive confirmation (yes/no)
//!   cashell reset --force  # no confirmation, destroys immediately

use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;

/// Factory reset command.
#[derive(clap::Parser, Debug, Default, Clone)]
pub struct ResetCommand {
    /// Skip the confirmation prompt (non-interactive use).
    #[arg(long, short = 'y')]
    pub force: bool,
}

/// Data paths managed by cashell.
fn data_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("cashell")
}

fn db_path() -> PathBuf {
    data_dir().join("cashell.db")
}

fn wal_path() -> PathBuf {
    data_dir().join("cashell.db-wal")
}

fn shm_path() -> PathBuf {
    data_dir().join("cashell.db-shm")
}

fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

fn cache_path() -> PathBuf {
    data_dir().join("cache")
}

pub fn run(cmd: &ResetCommand) -> Result<()> {
    let data_dir = data_dir();
    let db = db_path();
    let wal = wal_path();
    let shm = shm_path();
    let cfg = config_path();
    let cache = cache_path();

    // ── Collect items to remove ──────────────────────────────────────────────
    let mut items: Vec<(PathBuf, &'static str)> = Vec::new();

    if db.exists() {
        items.push((db, "SQLite database (cashell.db)"));
    }
    if wal.exists() {
        items.push((wal, "WAL journal file"));
    }
    if shm.exists() {
        items.push((shm, "Shared-memory file"));
    }
    if cfg.exists() {
        items.push((cfg, "Config file (config.toml)"));
    }
    if cache.exists() {
        items.push((cache, "Cache directory"));
    }

    if items.is_empty() {
        println!(
            "{} Nothing to reset. No data found at {}",
            "✔".green().bold(),
            data_dir.display()
        );
        println!();
        println!("  Cashell is already clean. Nothing was changed.");
        return Ok(());
    }

    // ── Confirmation ─────────────────────────────────────────────────────────
    println!("{} CAShell Factory Reset", "⚠".red().bold());
    println!("  The following items will be permanently deleted:");
    for (p, desc) in &items {
        println!("  ✗ {} → {}", desc.cyan(), p.display());
    }
    println!();
    println!("  This will remove ALL fleet servers, uptime monitors, analytics,");
    println!("  audit history, chat logs, cron jobs, pets, and configuration.");

    if !cmd.force {
        print!("  Type 'yes' to confirm: ");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        if input.trim() != "yes" {
            println!("  {}", "Aborted.".yellow());
            return Ok(());
        }
    }

    // ── Wipe ─────────────────────────────────────────────────────────────────
    for (p, _desc) in &items {
        if p.is_file() {
            std::fs::remove_file(p).ok();
        } else if p.is_dir() {
            std::fs::remove_dir_all(p).ok();
        }
    }

    // Create fresh config and empty dirs
    std::fs::create_dir_all(&data_dir).ok();
    std::fs::create_dir_all(cache_path()).ok();

    // Write fresh default config
    let fresh_cfg = r#"# CAShell config — defaults.
# Edit with: cashell config set <key> <value>
# Or manually: nano ~/.config/cashell/config.toml

[output]
theme = "ca-classic"
matrix_rain = true

[workspace]
scan_dirs = ["~/Product", "~/Project"]

[ai]
default_provider = "9router"

[database]
retention_days = 7
auto_prune = true

[security]
require_password = false

[uptime]
telegram_alerts_enabled = true
"#;
    std::fs::write(&config_path(), fresh_cfg).ok();

    // ── Report ───────────────────────────────────────────────────────────────
    println!(
        "\n{} CAShell has been reset to factory defaults.",
        "✔".green().bold()
    );
    println!();
    println!("  New config written to: {}", config_path().display());
    println!("  Database:              clean (empty)");
    println!("  Cache:                 empty");
    println!();
    println!("  Next step: register fleet nodes with `cas server` or open the cockpit.");
    Ok(())
}
