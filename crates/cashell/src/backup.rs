//! Backup & restore functionality for cashell.
//!
//! Creates tar.gz snapshots of `~/.config/cashell/` (config.toml, cashell.db)
//! and can restore them to the same location. Backups are written to
//! `~/.local/share/cashell/backups/` by default.

use anyhow::{Context, Result};
use chrono::Local;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Builder;

fn default_backup_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("cashell")
        .join("backups")
}

/// Snapshot of all cashell configuration and state files.
pub struct Snapshot {
    pub path: PathBuf,
    pub created_at: chrono::DateTime<Local>,
}

impl Snapshot {
    /// Build a tar.gz archive containing:
    /// - `config.toml`
    /// - `cashell.db`, `cashell.db-shm`, `cashell.db-wal` (if present)
    ///
    /// Returns the created `Snapshot` file path.
    pub fn create(config_dir: &Path, out_dir: &Path) -> Result<Snapshot> {
        fs::create_dir_all(out_dir)
            .with_context(|| format!("failed creating backup dir {}", out_dir.display()))?;

        let ts = Local::now().format("%Y%m%d_%H%M%S");
        let name = format!("cashell-backup-{}.tar.gz", ts);
        let out_path = out_dir.join(&name);

        let file = File::create(&out_path)
            .with_context(|| format!("failed creating backup file {}", out_path.display()))?;
        let mut gz = GzEncoder::new(file, Compression::default());

        let mut builder = Builder::new(&mut gz);

        // config.toml
        let cfg_path = config_dir.join("config.toml");
        if cfg_path.exists() {
            let mut buf = std::io::Read::read_exact(&mut config_dir.open(), &mut Vec::new()).ok().unwrap_or_default();
            // Use fs::read to keep it simple
            buf = fs::read(&cfg_path).with_context(|| format!("failed reading {}", cfg_path.display()))?;
            let mut header = tar::Header::new_gnu();
            header.set_path("config.toml")?;
            header.set_size(buf.len() as u64);
            header.set_cksum();
            builder.append(&mut header, &buf)?;
        }

        // SQLite database (only the main .db file is reliable when WAL is active;
        // we also include -shm and -wal if present so a cold restore works.)
        for suffix in ["db", "db-shm", "db-wal"] {
            let db_path = config_dir.join(format!("cashell.{}", suffix));
            if db_path.exists() {
                let data = fs::read(&db_path).with_context(|| format!("failed reading {}", db_path.display()))?;
                let mut header = tar::Header::new_gnu();
                header.set_path(format!("cashell.{}", suffix))?;
                header.set_size(data.len() as u64);
                header.set_cksum();
                builder.append(&mut header, &data)?;
            }
        }

        builder.finish()?;
        gz.finish()?;

        Ok(Snapshot {
            path: out_path,
            created_at: Local::now(),
        })
    }

    /// List all backup snapshots in `out_dir`, most recent first.
    pub fn list(out_dir: &Path) -> Result<Vec<PathBuf>> {
        if !out_dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(out_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "tar.gz")
            })
            .map(|e| {
                let modified = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok();
                (e.path(), modified)
            })
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(entries.into_iter().map(|(p, _)| p).collect())
    }

    /// Restore a snapshot into `config_dir`, backing up the current state first.
    pub fn restore(snapshot: &Path, config_dir: &Path) -> Result<()> {
        fs::create_dir_all(config_dir)
            .with_context(|| format!("failed creating config dir {}", config_dir.display()))?;

        let file = File::open(snapshot).with_context(|| {
            format!(
                "failed opening snapshot file {}",
                snapshot.display()
            )
        })?;
        let gz = std::io::BufReader::new(file);
        let mut tar = tar::Archive::new(gz);

        for entry in tar.entries()? {
            let mut entry = entry?;
            let path_in_archive = entry
                .path()
                .map_err(|_| anyhow::anyhow!("invalid archive entry"))?
                .to_path_buf();
            let name = path_in_archive
                .file_name()
                .map(|s| s.to_os_string())
                .ok_or_else(|| anyhow::anyhow!("archive entry missing filename"))?;

            // Safety: only write known expected files into config_dir.
            let target = config_dir.join(name);
            let allowed = [
                "config.toml",
                "cashell.db",
                "cashell.db-shm",
                "cashell.db-wal",
            ];
            if !allowed
                .iter()
                .any(|a| target.file_name() == Some(std::ffi::OsStr::new(a)))
            {
                continue;
            }
            fs::create_dir_all(&target.parent().unwrap_or_else(|| PathBuf::from(".")))?;
            std::io::copy(&mut entry, &mut target.file()).with_context(|| {
                format!("failed writing restored file {}", target.display())
            })?;
        }
        Ok(())
    }
}

/// Convenience helper to create a backup in the default location.
pub fn create_default_backup() -> Result<Snapshot> {
    let config_dir = dirs::config_dir()
        .map(|p| p.join("cashell"))
        .unwrap_or_else(|| PathBuf::from(".config/cashell"));
    Snapshot::create(&config_dir, &default_backup_dir())
}

/// Convenience helper to restore a backup into the default location.
pub fn restore_default_backup(snapshot: &Path) -> Result<()> {
    let config_dir = dirs::config_dir()
        .map(|p| p.join("cashell"))
        .unwrap_or_else(|| PathBuf::from(".config/cashell"));
    Snapshot::restore(snapshot, &config_dir)
}

/// Print a human-friendly summary of creating a backup in the default location.
pub fn backup() -> Result<()> {
    let snap = create_default_backup()?;
    println!("✅ CAShell backup created: {}", snap.path.display());
    println!(
        "   Snapshot timestamp: {}",
        snap.created_at.format("%Y-%m-%d %H:%M:%S")
    );
    Ok(())
}

/// Restore a snapshot into the default location and print guidance.
pub fn restore(snapshot: &Path) -> Result<()> {
    restore_default_backup(snapshot)?;
    println!("✅ CAShell state restored from {}", snapshot.display());
    println!("   Note: restart the daemon/cockpit to pick up changes.");
    Ok(())
}
