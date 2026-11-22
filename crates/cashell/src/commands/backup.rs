//! `cashell backup` — full backup & restore of CAShell state.
//!
//!   cashell backup create [--out DIR]
//!   cashell backup list   [--dir DIR]
//!   cashell backup restore FILE [--yes]

use crate::{backup, output};
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
pub struct BackupCommand {
    #[command(subcommand)]
    pub action: BackupAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BackupAction {
    /// Create a full backup: servers, monitors, projects, settings, history, analytics, encrypted secrets
    Create {
        /// Folder for the .tar.gz (default: ~/Documents/cashell-backups)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// List backups, newest first
    List {
        /// Folder to look in (default: ~/Documents/cashell-backups)
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Restore a backup (takes an automatic backup of the current state first)
    Restore {
        /// Path to a cashell-backup-*.tar.gz
        file: PathBuf,
        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

pub fn run(cmd: &BackupCommand) -> Result<i32> {
    match &cmd.action {
        BackupAction::Create { out } => {
            let dir = out.clone().unwrap_or_else(backup::default_dir);
            let (path, files) = backup::create(&crate::paths::config_dir(), &dir)?;
            output::success(&format!("Backup created: {}", path.display()));
            output::kv("files", &files.join(", "));
            output::info(
                "Keep this file private: it holds server addresses and encrypted secrets.",
            );
            Ok(0)
        }
        BackupAction::List { dir } => {
            let dir = dir.clone().unwrap_or_else(backup::default_dir);
            let items = backup::list(&dir)?;
            if items.is_empty() {
                output::info(&format!("No backups in {}", dir.display()));
            }
            for p in items {
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                println!("{}  ({:.1} KiB)", p.display(), size as f64 / 1024.0);
            }
            Ok(0)
        }
        BackupAction::Restore { file, yes } => restore(file, *yes),
    }
}

fn restore(file: &std::path::Path, yes: bool) -> Result<i32> {
    if let Some(pid) = backup::running_daemon_pid() {
        bail!("the CAShell daemon is running (pid {pid}); stop it first, then restore");
    }
    let (staging, manifest) = backup::extract(file)?;
    let result = (|| {
        println!(
            "Backup from CAShell {} at {}\nFiles: {}",
            manifest.cashell_version,
            manifest.created_at,
            manifest.files.join(", ")
        );
        if !yes {
            if !std::io::stdin().is_terminal() {
                bail!("refusing to restore without --yes in a non-interactive shell");
            }
            print!("Replace the current CAShell data with this backup? [y/N] ");
            let _ = std::io::stdout().flush();
            let mut ans = String::new();
            std::io::stdin().read_line(&mut ans)?;
            if !matches!(ans.trim(), "y" | "Y" | "yes") {
                output::info("Restore cancelled");
                return Ok(2);
            }
        }
        let config_dir = crate::paths::config_dir();
        let (pre, _) = backup::create(&config_dir, &backup::default_dir())?;
        output::info(&format!("Current state saved first: {}", pre.display()));
        let restored = backup::apply(&staging, &config_dir)?;
        output::success(&format!("Restored: {}", restored.join(", ")));
        output::info("Restart the Cockpit/daemon to load the restored data.");
        Ok(0)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}
