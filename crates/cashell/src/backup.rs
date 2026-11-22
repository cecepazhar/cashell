//! Full backup & restore — `cashell backup create | list | restore`.
//!
//! A backup is one `.tar.gz` holding everything needed to move CAShell to
//! another machine or roll back:
//! - `config.toml`   servers, monitors, projects, AI / Telegram / deploy settings
//! - `cashell.db`    uptime history, incidents, analytics sites & visits, builds
//!   (a consistent copy taken with `VACUUM INTO`, safe while the daemon runs)
//! - `secrets.enc`   encrypted SSH passwords / tokens (still encrypted inside)
//! - `ssh_known_hosts` confirmed server host keys (TOFU)
//! - `profile.toml`  optional import profile
//! - `manifest.json` format version, CAShell version, timestamp, file list
//!
//! Not included on purpose: license / Pro session / HWID (bound to this
//! device — re-activate on the new machine) and containers (they live in
//! docker/podman, not in CAShell).
//!
//! Packing uses the system `tar` (no extra crates). Restore extracts into an
//! empty temp dir, then copies only the known file names above as regular
//! files, after taking an automatic pre-restore backup.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Manifest format understood by this build.
const FORMAT: u32 = 1;

/// Plain files copied as-is from the config dir.
const PLAIN_FILES: [&str; 4] = [
    "config.toml",
    "secrets.enc",
    "ssh_known_hosts",
    "profile.toml",
];
const DB_FILE: &str = "cashell.db";
const MANIFEST: &str = "manifest.json";

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub cashell_version: String,
    pub created_at: String,
    pub files: Vec<String>,
}

/// Where backups go when `--out` is not given: `~/Documents/cashell-backups`
/// (survives `cashell reset`), else `~/.local/share/cashell/backups`.
pub fn default_dir() -> PathBuf {
    if crate::paths::test_home_override().is_none() {
        if let Some(docs) = dirs::document_dir() {
            if docs.is_dir() {
                return docs.join("cashell-backups");
            }
        }
    }
    crate::paths::data_dir().join("backups")
}

#[cfg(unix)]
fn set_private(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}
#[cfg(not(unix))]
fn set_private(_path: &Path, _mode: u32) {}

fn fresh_temp_dir(tag: &str) -> Result<PathBuf> {
    let base = crate::paths::tmp_dir();
    fs::create_dir_all(&base).with_context(|| format!("creating {}", base.display()))?;
    let dir = base.join(format!(
        "{tag}-{}-{}",
        std::process::id(),
        chrono::Local::now().format("%Y%m%d%H%M%S%f")
    ));
    fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
    set_private(&dir, 0o700);
    Ok(dir)
}

/// Copy the config dir's state into `staging` and write the manifest.
/// Returns the list of files staged.
pub fn stage(config_dir: &Path, staging: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for name in PLAIN_FILES {
        let src = config_dir.join(name);
        if src.is_file() {
            fs::copy(&src, staging.join(name))
                .with_context(|| format!("reading {}", src.display()))?;
            files.push(name.to_string());
        }
    }
    let db = config_dir.join(DB_FILE);
    if db.is_file() {
        let dst = staging.join(DB_FILE);
        let conn =
            rusqlite::Connection::open(&db).with_context(|| format!("opening {}", db.display()))?;
        conn.execute("VACUUM INTO ?1", [dst.to_string_lossy().as_ref()])
            .with_context(|| format!("snapshotting {}", db.display()))?;
        files.push(DB_FILE.to_string());
    }
    let manifest = Manifest {
        format: FORMAT,
        cashell_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: chrono::Local::now().to_rfc3339(),
        files: files.clone(),
    };
    fs::write(
        staging.join(MANIFEST),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(files)
}

/// Create a backup archive in `out_dir`. Returns its path and staged files.
pub fn create(config_dir: &Path, out_dir: &Path) -> Result<(PathBuf, Vec<String>)> {
    fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    set_private(out_dir, 0o700);
    let staging = fresh_temp_dir("backup")?;
    let result = (|| {
        let files = stage(config_dir, &staging)?;
        let name = format!(
            "cashell-backup-{}.tar.gz",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        let out = out_dir.join(name);
        let status = Command::new("tar")
            .arg("czf")
            .arg(&out)
            .arg("-C")
            .arg(&staging)
            .arg(".")
            .status()
            .context("running `tar` (is it installed?)")?;
        if !status.success() {
            bail!("`tar` failed creating {}", out.display());
        }
        set_private(&out, 0o600);
        Ok((out, files))
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Backups in `dir`, newest first.
pub fn list(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut v: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("cashell-backup-") && n.ends_with(".tar.gz"))
        })
        .map(|p| {
            let m = fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (p, m)
        })
        .collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    Ok(v.into_iter().map(|(p, _)| p).collect())
}

/// Read and check the manifest of an extracted backup.
fn read_manifest(staging: &Path) -> Result<Manifest> {
    let raw = fs::read_to_string(staging.join(MANIFEST))
        .context("not a CAShell backup (manifest.json missing)")?;
    let m: Manifest = serde_json::from_str(&raw).context("manifest.json is invalid")?;
    if m.format != FORMAT {
        bail!(
            "backup format {} is not supported by this CAShell (expects {FORMAT})",
            m.format
        );
    }
    Ok(m)
}

/// Copy staged files into `config_dir`. Only known names, only regular files
/// (symlinks are refused). Returns the files restored.
pub fn apply(staging: &Path, config_dir: &Path) -> Result<Vec<String>> {
    fs::create_dir_all(config_dir)?;
    let mut restored = Vec::new();
    for name in PLAIN_FILES.iter().chain(std::iter::once(&DB_FILE)) {
        let src = staging.join(name);
        let Ok(meta) = fs::symlink_metadata(&src) else {
            continue;
        };
        if !meta.file_type().is_file() {
            bail!("refusing non-regular file '{name}' in backup");
        }
        let dst = config_dir.join(name);
        let tmp = config_dir.join(format!("{name}.restore-tmp"));
        fs::copy(&src, &tmp).with_context(|| format!("writing {}", tmp.display()))?;
        if *name == DB_FILE {
            // The snapshot is self-contained; a leftover WAL would corrupt it.
            for side in ["cashell.db-wal", "cashell.db-shm"] {
                let p = config_dir.join(side);
                if p.exists() {
                    fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
                }
            }
        }
        fs::rename(&tmp, &dst).with_context(|| format!("replacing {}", dst.display()))?;
        if matches!(*name, "secrets.enc" | "cashell.db" | "ssh_known_hosts") {
            set_private(&dst, 0o600);
        }
        restored.push((*name).to_string());
    }
    Ok(restored)
}

/// Extract `archive` into a fresh temp dir and return (dir, manifest).
pub fn extract(archive: &Path) -> Result<(PathBuf, Manifest)> {
    if !archive.is_file() {
        bail!("backup file not found: {}", archive.display());
    }
    let staging = fresh_temp_dir("restore")?;
    let status = Command::new("tar")
        .arg("xzf")
        .arg(archive)
        .arg("-C")
        .arg(&staging)
        .arg("--no-same-owner")
        .status()
        .context("running `tar` (is it installed?)");
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            let _ = fs::remove_dir_all(&staging);
            bail!("`tar` could not read {}", archive.display());
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    }
    match read_manifest(&staging) {
        Ok(m) => Ok((staging, m)),
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// PID of a live CAShell daemon, if any (pid file + /proc cmdline check).
pub fn running_daemon_pid() -> Option<i32> {
    let pid: i32 = fs::read_to_string(crate::paths::daemon_pid_path())
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let cmdline = fs::read_to_string(format!("/proc/{pid}/cmdline")).ok()?;
    cmdline.contains("cashell").then_some(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cashell-bk-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).ok();
        d
    }

    #[test]
    fn test_stage_then_apply_round_trips_files_and_db() -> Result<()> {
        let src = tmp("src");
        let stg = tmp("stg");
        let dst = tmp("dst");
        fs::write(src.join("config.toml"), "theme = \"x\"\n")?;
        fs::write(src.join("secrets.enc"), b"\x01\x02enc")?;
        let conn = rusqlite::Connection::open(src.join("cashell.db"))?;
        conn.execute_batch("CREATE TABLE t(v TEXT); INSERT INTO t VALUES('hello');")?;
        drop(conn);
        fs::write(dst.join("cashell.db-wal"), "stale")?;

        let staged = stage(&src, &stg)?;
        assert!(staged.contains(&"cashell.db".to_string()));
        assert_eq!(read_manifest(&stg)?.files, staged);

        let restored = apply(&stg, &dst)?;
        assert_eq!(restored.len(), 3);
        assert_eq!(
            fs::read_to_string(dst.join("config.toml"))?,
            "theme = \"x\"\n"
        );
        assert_eq!(fs::read(dst.join("secrets.enc"))?, b"\x01\x02enc");
        assert!(!dst.join("cashell.db-wal").exists());
        let conn = rusqlite::Connection::open(dst.join("cashell.db"))?;
        let v: String = conn.query_row("SELECT v FROM t", [], |r| r.get(0))?;
        assert_eq!(v, "hello");
        for d in [src, stg, dst] {
            let _ = fs::remove_dir_all(d);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_apply_refuses_symlinks_and_ignores_unknown_names() -> Result<()> {
        let stg = tmp("sym");
        let dst = tmp("symdst");
        fs::write(stg.join("evil.sh"), "x")?;
        std::os::unix::fs::symlink("/etc/passwd", stg.join("config.toml"))?;
        assert!(apply(&stg, &dst).is_err());
        assert!(!dst.join("evil.sh").exists());
        let _ = fs::remove_dir_all(stg);
        let _ = fs::remove_dir_all(dst);
        Ok(())
    }

    #[test]
    fn test_manifest_rejects_unknown_format() -> Result<()> {
        let stg = tmp("fmt");
        fs::write(
            stg.join(MANIFEST),
            r#"{"format":99,"cashell_version":"x","created_at":"x","files":[]}"#,
        )?;
        assert!(read_manifest(&stg).is_err());
        let _ = fs::remove_dir_all(stg);
        Ok(())
    }
}
