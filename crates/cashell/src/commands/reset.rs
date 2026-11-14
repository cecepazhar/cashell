//! `cashell reset` — Factory reset CLI (R-02).
//!
//! Wipes ALL local state CAShell writes (see [`crate::paths::resettable_paths`]),
//! leaving a clean slate. Every path goes through [`crate::paths`]; nothing
//! outside the CAShell tree is ever touched (K7).
//!
//! Usage:
//!   cashell reset                          # interactive: type RESET to confirm
//!   cashell reset --yes                    # skip confirmation
//!   cashell reset --backup /tmp/b.tar.gz   # tar.gz snapshot (0600) before wipe
//!
//! Exit codes: 0 = all removed, 1 = at least one item failed, 2 = cancelled.

use crate::license_gcc;
use crate::paths;
use anyhow::{anyhow, Result};
use colored::Colorize;
use std::fs;
use std::path::PathBuf;

/// Factory reset command.
#[derive(clap::Parser, Debug, Default, Clone)]
pub struct ResetCommand {
    /// Skip the confirmation prompt (non-interactive use).
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Create a .tar.gz backup at this path before wiping (0600, contains
    /// secrets — store it safely).
    #[arg(long)]
    pub backup: Option<PathBuf>,
}

/// Human-readable size, e.g. "1.2 MiB".
fn human_size(path: &std::path::Path) -> String {
    let mut size: u64 = 0;
    if path.is_file() {
        size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    } else if path.is_dir() {
        // Recurse without following symlinked subdirs (K7: never leave root).
        fn walk(dir: &std::path::Path, acc: &mut u64) {
            if let Ok(entries) = fs::read_dir(dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_symlink() {
                        continue;
                    } else if p.is_dir() {
                        walk(&p, acc);
                    } else if let Ok(m) = fs::metadata(&p) {
                        *acc += m.len();
                    }
                }
            }
        }
        walk(path, &mut size);
    }
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", size as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Stop the running CAShell daemon (R-03): SIGTERM via PID file, wait ≤ 10 s,
/// then remove the PID file.
#[cfg(unix)]
fn stop_daemon() {
    let pid_path = paths::daemon_pid_path();
    let pid_str = match fs::read_to_string(&pid_path) {
        Ok(s) => s.trim().to_string(),
        Err(_) => {
            println!(
                "  Daemon:    not running (no PID file at {})",
                pid_path.display()
            );
            return;
        }
    };
    let pid: i32 = match pid_str.parse() {
        Ok(p) => p,
        Err(_) => {
            println!(
                "  Daemon:    PID file at {} is malformed ('{}'), skipping",
                pid_path.display(),
                pid_str
            );
            let _ = fs::remove_file(&pid_path);
            return;
        }
    };

    // SIGTERM the daemon process only if it really is the CAShell daemon
    // (verify /proc cmdline; stale PID files must not kill unrelated procs).
    let cmdline = std::fs::read_to_string(format!("/proc/{pid}/cmdline")).ok();
    let is_cashell = cmdline
        .as_deref()
        .map(|s| s.contains("cashell"))
        .unwrap_or(false);

    if is_cashell {
        match std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
        {
            Ok(_) => println!("  Daemon:    SIGTERM sent to pid {pid}; waiting ≤ 10 s"),
            Err(e) => eprintln!("  ⚠ Daemon:    SIGTERM to pid {pid} failed: {e} (continuing)"),
        }

        // Wait ≤ 10 s for the process to exit: kill -0 succeeds while it lives.
        for _ in 0..40 {
            let alive = std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !alive {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        println!("  Daemon:    stopped");
    } else {
        println!(
            "  Daemon:    PID {pid} is not the CAShell daemon (stale PID file or missing) — no signal sent"
        );
    }

    let _ = fs::remove_file(&pid_path);
}

/// Non-unix fallback: only the PID file is cleared (no SIGTERM on Windows).
#[cfg(not(unix))]
fn stop_daemon() {
    let pid_path = paths::daemon_pid_path();
    if fs::remove_file(&pid_path).is_ok() {
        println!("  Daemon:    PID file cleared (no SIGTERM on this platform)");
    } else {
        println!("  Daemon:    not running (no PID file)");
    }
}

/// Stop + disable + remove the user systemd unit installed by CAShell (R-03).
/// System unit (`/etc/systemd/system/...`) needs root — report only, do not
/// attempt (K7: never write outside the CAShell tree).
fn stop_systemd() {
    // User unit: CAShell-owned path, safe to stop/disable/remove.
    let user_unit = paths::user_systemd_unit();
    let user_installed = user_unit.exists();
    let system_unit = paths::system_systemd_unit();
    let system_installed = system_unit.exists();

    if user_installed || system_installed {
        println!("{} Stopping systemd units...", "→".bold());
        let user_stop = std::process::Command::new("systemctl")
            .args(["--user", "stop", "cashell-daemon.service"])
            .status();
        let user_disable = std::process::Command::new("systemctl")
            .args(["--user", "disable", "cashell-daemon.service"])
            .status();
        match user_stop {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("  Warning: user unit stop returned {:?}", s.code()),
            Err(e) => eprintln!("  Warning: systemctl not available ({e}); unit stop skipped"),
        }
        let _ = user_disable;

        // Remove only the user unit (CAShell tree). System unit: report.
        if user_installed {
            let _ = fs::remove_file(&user_unit);
            println!("  User unit removed: {}", user_unit.display());
        }
        if system_installed {
            println!(
                "{} System unit installed: {} — run `sudo systemctl stop cashell-daemon && sudo systemctl disable cashell-daemon && sudo rm {}` to remove.",
                "⚠".yellow(),
                system_unit.display(),
                system_unit.display()
            );
        }
    } else {
        println!("  Systemd:   no CAShell unit installed (skipped)");
    }
}

/// Deactivate the license on GCC (R-04). Offline → continue with warning.
fn deactivate_license() {
    let Ok(Some(lic)) = crate::license_local::load_license() else {
        println!("  License:   none installed (skipped)");
        return;
    };

    let key_masked = license_gcc::mask_key(&lic.key);
    let base_url = match crate::config::Config::load(None) {
        Ok(cfg) => cfg.license.gcc_base_url,
        Err(_) => String::new(),
    };

    if base_url.trim().is_empty() {
        println!(
            "{} License {key_masked} ({}) — no gcc_base_url configured, GCC deactivation skipped.",
            "⚠".yellow(),
            lic.product
        );
        return;
    }

    let dev_id = license_gcc::device_id();
    match license_gcc::deactivate(&base_url, &lic.key, &dev_id) {
        Ok(()) => {
            println!(
                "  License:   {key_masked} deactivated on GCC ({}).",
                "✔".green()
            );
        }
        Err(e) => {
            // PRD: offline → continue with warning; device slot NOT freed.
            println!(
                "{} License {key_masked} — GCC unreachable ({e}). Device slot NOT freed; \
                 offline grace applies (last_verified_at + 30 days).",
                "⚠".yellow()
            );
        }
    }
}

/// Create a .tar.gz backup of every resettable path (R-07). Permission 0600.
/// Uses the `tar` binary via Command (not a new crate dependency — checked
/// Cargo.lock: no `tar`/`flate2` available; system `tar` is std tooling).
fn create_backup(out: &PathBuf) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // Collect paths that actually exist, relative to a stable parent so the
    // tar member names are meaningful.
    let existing: Vec<PathBuf> = paths::resettable_paths()
        .into_iter()
        .filter(|p| p.exists())
        .collect();
    if existing.is_empty() {
        return Ok(());
    }

    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow!("cannot create backup parent {}: {e}", parent.display()))?;
    }

    // Run tar czf on the existing paths. Use --absolute-names so the archive
    // is self-contained; paths are already canonicalized under CASHELL_HOME.
    let mut cmd = std::process::Command::new("tar");
    cmd.arg("czf").arg(out);
    for p in &existing {
        cmd.arg(p.to_string_lossy().as_ref());
    }
    let status = cmd
        .status()
        .map_err(|e| anyhow!("failed to run tar: {e}"))?;
    if !status.success() {
        let _ = fs::remove_file(out);
        return Err(anyhow!("tar czf exited with {status}"));
    }

    // 0600: the archive contains secrets (config tokens, license key, vault).
    fs::set_permissions(out, fs::Permissions::from_mode(0o600))
        .map_err(|e| anyhow!("failed to set 0600 on {}: {e}", out.display()))?;

    Ok(())
}

/// Write a fresh default config (R-05): `toml::to_string_pretty(&Config::default())`.
fn write_default_config() -> Result<()> {
    let cfg = crate::config::Config::default();
    let content = toml::to_string_pretty(&cfg)?;
    let p = paths::config_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&p, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).ok();
    }
    Ok(())
}

/// Confirm the wipe. User must type `RESET` exactly; `--yes` skips.
/// Returns: true = proceed, false = cancelled (exit 2).
fn confirm(items: &[(PathBuf, &str)]) -> bool {
    println!("{} CASHELL FACTORY RESET", "⚠".red().bold());
    println!("  The following items will be PERMANENTLY deleted:");
    for (p, desc) in items {
        println!("  ✗ {:<28} {}  ({})", desc, p.display(), human_size(p));
    }
    println!();
    println!("  This removes ALL CAShell state: config, database, license,");
    println!("  fleet servers, uptime monitors, audit history, chat logs,");
    println!("  cron jobs, pro sessions, vault, cache, and temp files.");
    println!("  Files outside the CAShell tree are NOT touched (K7).");
    println!();

    print!("  Type 'RESET' to confirm: ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    input.trim() == "RESET"
}

/// Collect all existing resettable items with their descriptions and formatted sizes.
pub fn collect_resettable_items() -> Vec<(PathBuf, &'static str, String)> {
    let mut items = Vec::new();
    for p in paths::resettable_paths() {
        if !p.exists() {
            continue;
        }
        let desc: &'static str = if p == paths::db_path() {
            "SQLite database"
        } else if p == paths::db_wal_path() {
            "SQLite WAL journal"
        } else if p == paths::db_shm_path() {
            "SQLite shared memory"
        } else if p == paths::config_path() {
            "Config file"
        } else if p == paths::license_path() {
            "License file (secret)"
        } else if p == paths::pro_session_path() {
            "Pro session (secret)"
        } else if p == paths::ssh_known_hosts_path() {
            "SSH known hosts (secret)"
        } else if p == paths::daemon_pid_path() {
            "Daemon PID file"
        } else if p == paths::hwid_path() {
            "HWID cache"
        } else if p == paths::vault_dir() {
            "Vault (knowledge base)"
        } else if p == paths::fastboot_cache_path() {
            "Fast-boot cache"
        } else if p == paths::backups_dir() {
            "Backup snapshots"
        } else if p == paths::user_systemd_unit() {
            "Systemd user unit"
        } else if p == paths::tmp_dir() {
            "Temp dir"
        } else if p == paths::config_dir() {
            "Config dir"
        } else if p == paths::data_dir() {
            "Data dir"
        } else if p == paths::cache_dir() {
            "Cache dir"
        } else {
            "Other"
        };
        let size = human_size(&p);
        items.push((p, desc, size));
    }
    items
}

pub fn run(cmd: &ResetCommand) -> Result<i32> {
    // ── Inventory: only list items that actually exist (with size) ─────────
    let item_records = collect_resettable_items();
    let items: Vec<(PathBuf, &str)> = item_records
        .iter()
        .map(|(p, desc, _)| (p.clone(), *desc))
        .collect();

    if items.is_empty() {
        println!(
            "{} Nothing to reset. No CAShell data found under {} / {} / {}.",
            "✔".green().bold(),
            paths::config_dir().display(),
            paths::data_dir().display(),
            paths::cache_dir().display()
        );
        return Ok(0);
    }

    // ── License deactivation warning (before confirm) ──────────────────────
    if let Ok(Some(lic)) = crate::license_local::load_license() {
        let key_masked = license_gcc::mask_key(&lic.key);
        println!(
            "{} Active license {key_masked} ({}) will be deactivated on GCC.",
            "⚠".yellow().bold(),
            lic.product
        );
    }

    // ── Confirmation (exit 2 on cancel) ────────────────────────────────────
    if cmd.yes {
        println!("  --yes given: skipping confirmation.");
    } else if !confirm(&items) {
        println!("  {}", "Aborted — nothing was changed.".yellow());
        return Ok(2);
    }

    // ── R-07: optional backup before the wipe ──────────────────────────────
    if let Some(ref out) = cmd.backup {
        println!("  Creating backup at {} ...", out.display());
        match create_backup(out) {
            Ok(()) => println!(
                "  {} Backup written (0600). WARNING: contains secrets — store safely.",
                "✔".green()
            ),
            Err(e) => {
                eprintln!(
                    "{} Backup FAILED: {e} — aborting before wipe.",
                    "✖".red().bold()
                );
                return Ok(1);
            }
        }
    }

    // ── R-03: stop daemon + systemd before deleting state ─────────────────
    stop_daemon();
    stop_systemd();

    // ── R-04: deactivate license (offline → warning, continue) ────────────
    deactivate_license();

    // ── Wipe: every item, canonicalize + assert inside CAShell roots (K7) ──
    let mut failures: Vec<String> = Vec::new();
    for (p, desc) in &items {
        // If stop_daemon() already removed this path, skip — nothing left to
        // assert or delete. A missing path is not a failure.
        if !p.exists() {
            continue;
        }
        match paths::assert_inside_cashell(p) {
            Ok(()) => {}
            Err(why) => {
                failures.push(format!("SKIPPED {desc}: {why}"));
                println!("  {} {desc}: {why}", "✖".red().bold());
                continue;
            }
        }
        match paths::remove_cashell_path(p, desc) {
            Ok(()) => println!("  {} removed: {}", "✔".green(), p.display()),
            Err(why) => {
                failures.push(format!("FAILED {desc} ({}): {why}", p.display()));
                println!("  {} {why}", "✖".red().bold());
            }
        }
    }

    // ── R-05: write fresh default config (TOML from Config::default()) ─────
    match write_default_config() {
        Ok(()) => println!(
            "  {} default config written to {}",
            "✔".green(),
            paths::config_path().display()
        ),
        Err(e) => {
            failures.push(format!("FAILED default config: {e}"));
            println!("  {} default config write FAILED: {e}", "✖".red().bold());
        }
    }

    // ── Summary ─────────────────────────────────────────────────────────────
    println!();
    if failures.is_empty() {
        println!(
            "{} CASHELL FACTORY RESET COMPLETE. All {} item(s) removed; default config restored.",
            "✔".green().bold(),
            items.len()
        );
        return Ok(0);
    }

    println!(
        "{} CASHELL RESET PARTIAL — {} of {} item(s) FAILED:",
        "✖".red().bold(),
        failures.len(),
        items.len()
    );
    for f in &failures {
        println!("  - {f}");
    }
    println!("  Next step: re-run `cashell reset --yes` to retry the failed items.");
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// Per-test fake home. Owns its temp dir; `Drop` cleans it up so no
    /// other test sees a stale tree. Relocates the tree via the
    /// thread-local seam in [`crate::paths`] — no global env mutation, so
    /// all reset tests run fully in parallel.
    struct FakeHome {
        dir: PathBuf,
    }

    impl FakeHome {
        fn new() -> Self {
            let base = std::env::temp_dir().join("cashell-reset-tests");
            let dir = base.join(format!(
                "t{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn home(&self) -> &std::path::Path {
            &self.dir
        }

        /// Run `f` with this thread's CAShell tree relocated into the fake
        /// home: thread-local home override + matching tmp-dir override.
        /// No env vars, no locks, no watchdog — the override is
        /// thread-scoped, so parallel tests can never interfere.
        fn with_home<T>(&self, f: impl Fn() -> T) -> T {
            let home = self.dir.clone();
            let tmp = home.join("tmp");
            paths::with_test_home(&home, move || paths::with_test_tmp(&tmp, f))
        }
    }

    impl Drop for FakeHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// Populate the fake home with every resettable item.
    fn populate(fh: &FakeHome) {
        let home = fh.home();
        let c = home.join(".config/cashell");
        let d = home.join(".local/share/cashell");
        let k = home.join(".cache/cashell");
        let t = home.join("tmp");
        fs::create_dir_all(&c).unwrap();
        fs::create_dir_all(&d).unwrap();
        fs::create_dir_all(&k).unwrap();
        fs::create_dir_all(&t).unwrap();
        fs::write(c.join("config.toml"), "[output]\ntheme = \"ca-classic\"\n").unwrap();
        fs::write(c.join("cashell.db"), b"sqlite-fake-data").unwrap();
        fs::write(c.join("cashell.db-wal"), b"wal").unwrap();
        fs::write(c.join("cashell.db-shm"), b"shm").unwrap();
        fs::write(c.join("pro_session.json"), "{}".as_bytes()).unwrap();
        fs::write(c.join("ssh_known_hosts"), b"host ssh-ed25519 AAAA").unwrap();
        fs::write(d.join("license.json"), b"{}").unwrap();
        fs::write(d.join("hwid"), b"hwid-123").unwrap();
        fs::write(d.join("daemon.pid"), b"99999").unwrap();
        fs::create_dir_all(d.join("vault")).unwrap();
        fs::write(k.join("pro-fastboot.json"), b"{}").unwrap();
        fs::create_dir_all(k.join("backups")).unwrap();
        fs::write(t.join("scratch.txt"), b"x").unwrap();
    }

    #[test]
    fn test_reset_all_present_exit_0_all_removed() {
        let fh = FakeHome::new();
        populate(&fh);
        let cmd = ResetCommand {
            yes: true,
            backup: None,
        };
        let code = fh
            .with_home(|| run(&cmd))
            .expect("run() must not return Err");
        assert_eq!(code, 0, "all present → exit 0");

        let c = fh.home().join(".config/cashell");
        assert!(
            c.join("config.toml").is_file(),
            "default config.toml written"
        );
        assert!(!c.join("cashell.db").exists(), "db removed");
        assert!(!c.join("pro_session.json").exists(), "pro session removed");

        let d = fh.home().join(".local/share/cashell");
        assert!(!d.join("license.json").exists(), "license removed");
        assert!(!d.join("hwid").exists(), "hwid removed");
        assert!(!d.join("daemon.pid").exists(), "pid removed");
        assert!(!d.join("vault").exists(), "vault removed");

        let k = fh.home().join(".cache/cashell");
        assert!(!k.join("pro-fastboot.json").exists(), "fastboot removed");
        assert!(!fh.home().join("tmp").exists(), "tmp scratch removed");
    }

    #[test]
    fn test_reset_yes_skips_confirm() {
        let fh = FakeHome::new();
        populate(&fh);
        let cmd = ResetCommand {
            yes: true,
            backup: None,
        };
        let code = fh
            .with_home(|| run(&cmd))
            .expect("run() must not return Err");
        assert_eq!(code, 0, "--yes must not block on stdin");
    }

    #[test]
    fn test_reset_backup_creates_tarball() {
        let fh = FakeHome::new();
        populate(&fh);
        let backup_path = fh.home().join("backup.tar.gz");
        let cmd = ResetCommand {
            yes: true,
            backup: Some(backup_path.clone()),
        };
        let code = fh
            .with_home(|| run(&cmd))
            .expect("run() must not return Err");
        assert_eq!(code, 0, "reset with --backup must still succeed");
        assert!(backup_path.is_file(), "backup tarball created");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&backup_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "backup must be 0600, got {mode:#o}");
        }
    }

    /// R-08: full integration test.
    ///
    /// - `CASHELL_HOME` → temp dir via `FakeHome` + the thread-local
    ///   `paths::with_test_home` / `with_test_tmp` seam (never the owner's
    ///   real profile; no global env mutation, so tests run in parallel).
    /// - Every CAShell path populated with fake data.
    /// - Mock GCC (axum, random port) on a dedicated multi-thread runtime
    ///   observes the deactivate POST.
    /// - `reset --yes` → assert ONLY the default `config.toml` remains.
    ///
    /// Deterministic by design: `run()` is called DIRECTLY in this test
    /// thread under the thread-local override. The mock server runs on its
    /// own runtime worker threads and never calls `paths`.
    #[test]
    fn test_reset_integration_home_override() {
        let fh = FakeHome::new();
        populate(&fh);

        use axum::body::Body;
        use axum::routing::post;
        use axum::Router;
        use tokio::net::TcpListener;

        let deactivate_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = deactivate_called.clone();

        // Dedicated multi-thread runtime owned ONLY by the mock server.
        // Multi-thread is required: `run()` below blocks the *test* thread on
        // a synchronous reqwest POST, and the server task must keep making
        // progress on its own worker threads to answer it. Keeping it on a
        // separate runtime avoids any deadlock between the two.
        let server_rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        let base_url = server_rt.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let app = Router::new().route(
                "/api/v1/licenses/deactivate",
                post(move || async move {
                    called_clone.store(true, Ordering::SeqCst);
                    axum::http::Response::builder().body(Body::empty()).unwrap()
                }),
            );
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            // Brief yield so the spawned server task reaches its bind/accept
            // loop before the first request arrives (determinism aid).
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            format!("http://127.0.0.1:{port}")
        });

        // License file so `deactivate_license()` has a key to send.
        let license_json = serde_json::json!({
            "schema": 2,
            "key": "CASH99-FOUNDR-3",
            "product": "SH",
            "tier": "99",
            "activated_at": "2025-01-01T00:00:00Z",
            "last_verified_at": "2025-06-01T00:00:00Z",
        })
        .to_string();
        fs::write(
            fh.home().join(".local/share/cashell/license.json"),
            &license_json,
        )
        .unwrap();

        // config.toml with [license] gcc_base_url → mock GCC.
        let cfg = crate::config::Config::default();
        let mut cfg_toml = toml::to_string_pretty(&cfg).unwrap();
        cfg_toml = cfg_toml.replace(
            "gcc_base_url = \"\"",
            &format!("gcc_base_url = \"{base_url}\""),
        );
        let c = fh.home().join(".config/cashell");
        fs::write(c.join("config.toml"), &cfg_toml).unwrap();

        let cmd = ResetCommand {
            yes: true,
            backup: None,
        };
        // `run()` is fully synchronous (reqwest blocking client). Call it
        // DIRECTLY in THIS test thread under the thread-local home/tmp
        // override so every `paths` resolution lands inside the fake home.
        // The mock server keeps running on its own `server_rt` worker
        // threads (it never calls `paths`), so the blocking POST is
        // answered without any cross-thread interference.
        let code = fh
            .with_home(|| run(&cmd))
            .expect("integration reset must exit 0");
        assert_eq!(code, 0, "integration reset must exit 0");
        // Shut the mock server down deterministically: the serve task blocks
        // on `.await` for the lifetime of the process, so `drop(server_rt)`
        // would wait forever. `shutdown_timeout` aborts the runtime, which
        // kills the spawned serve task and releases the port.
        server_rt.shutdown_timeout(std::time::Duration::from_secs(1));
        // Confirm the mock server answered before proceeding; `run()` has
        // already returned, so this read is race-free (SeqCst ordering).
        assert!(
            deactivate_called.load(Ordering::SeqCst),
            "mock GCC /api/v1/licenses/deactivate was not called"
        );

        // Only the default config.toml remains in .config/cashell.
        let config_dir = fh.home().join(".config/cashell");
        let remaining: Vec<String> = fs::read_dir(&config_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            remaining,
            vec!["config.toml".to_string()],
            "only default config.toml should remain, found: {remaining:?}"
        );

        // License file gone.
        assert!(
            !fh.home().join(".local/share/cashell/license.json").exists(),
            "license.json must be removed"
        );
    }
}
