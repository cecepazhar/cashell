//! Central path inventory for everything CAShell reads or writes.
//!
//! K7: factory reset must delete ONLY paths listed here. No other module may
//! invent a new location on disk without adding it to this module and to the
//! inventory table at the bottom of this file.
//!
//! All paths respect the `CASHELL_HOME` override (used by the R-08
//! integration test: point it at a temp dir and the whole tree relocates).
//!
//! Resolution rules (mirrors `dirs` on Linux, so no other module needs to
//! touch `$HOME` / XDG vars directly):
//! - `CASHELL_HOME` set            → home = `$CASHELL_HOME`
//! - `CASHELL_HOME` unset          → home = `$HOME` (via `dirs::home_dir`)
//! - `XDG_CONFIG_HOME` absolute    → config root = `$XDG_CONFIG_HOME`
//! - `XDG_DATA_HOME` absolute      → data root = `$XDG_DATA_HOME`
//! - `XDG_CACHE_HOME` absolute     → cache root = `$XDG_CACHE_HOME`
//! - otherwise                     → `.config` / `.local/share` / `.cache` under home

use std::cell::Cell;
use std::path::PathBuf;

/// Test override for the home directory. Every path in this module derives
/// from [`home_dir`], so one env var relocates the entire CAShell tree.
const HOME_ENV: &str = "CASHELL_HOME";

// ── Thread-local test seams ─────────────────────────────────────────────
// Parallel tests must NOT mutate process-global env vars (`CASHELL_HOME`,
// `$HOME`, `__TEST_TMP_DIR`): those race across test threads in the same
// process. Instead each test thread sets a thread-local override here, and
// `home_dir()` / `tmp_dir()` consult it first. Production behaviour is
// unchanged: when no override is active the original resolution order
// (env → `dirs`) applies untouched.
thread_local! {
    static TEST_HOME_OVERRIDE: Cell<Option<PathBuf>> = const { Cell::new(None) };
    static TEST_TMP_OVERRIDE: Cell<Option<PathBuf>> = const { Cell::new(None) };
}

/// Per-test hook: read the active thread-local home override, if any.
// NOTE: implemented via `take()` + `set()` — plain `Cell::get()` on a
// non-`Copy` type is rejected by the current rustc (Copy-inference
// regression, E0599); take/set route through `Clone` and compile.
pub fn test_home_override() -> Option<PathBuf> {
    let v = TEST_HOME_OVERRIDE.with(|c| c.take());
    TEST_HOME_OVERRIDE.with(|c| c.set(v.clone()));
    v
}

/// Run `f` with the thread-local home override active; restore the
/// previous override afterwards. Nested calls restore LIFO.
///
/// While active the override relocates the *entire* CAShell tree (R-08
/// semantics): XDG env vars are ignored so no part of the tree leaks
/// outside `home` on a dev machine that exports `XDG_*`.
pub fn with_test_home<P: FnOnce() -> R, R>(home: &std::path::Path, f: P) -> R {
    let prev = TEST_HOME_OVERRIDE.with(|c| c.take());
    TEST_HOME_OVERRIDE.with(|c| c.set(Some(home.to_path_buf())));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    TEST_HOME_OVERRIDE.with(|c| c.set(prev));
    match r {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e),
    }
}

/// Run `f` with the thread-local tmp-dir override active (test seam
/// replacing the `__TEST_TMP_DIR` env var); restore afterwards.
pub fn with_test_tmp<P: FnOnce() -> R, R>(tmp: &std::path::Path, f: P) -> R {
    let prev = TEST_TMP_OVERRIDE.with(|c| c.take());
    TEST_TMP_OVERRIDE.with(|c| c.set(Some(tmp.to_path_buf())));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    TEST_TMP_OVERRIDE.with(|c| c.set(prev));
    match r {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e),
    }
}

/// The home root all CAShell paths hang off.
pub fn home_dir() -> PathBuf {
    if let Some(ov) = test_home_override() {
        return ov;
    }
    std::env::var_os(HOME_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn xdg_root(var: &str, home_fallback: &std::path::Path) -> PathBuf {
    // A thread-local home override relocates the whole tree — ignore XDG
    // env vars so nothing escapes the override (test seam only).
    if test_home_override().is_some() {
        return home_fallback.to_path_buf();
    }
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home_fallback.to_path_buf())
}

/// `~/.config` (or `$XDG_CONFIG_HOME`).
pub fn config_root() -> PathBuf {
    xdg_root("XDG_CONFIG_HOME", &home_dir().join(".config"))
}

/// `~/.local/share` (or `$XDG_DATA_HOME`).
pub fn data_root() -> PathBuf {
    xdg_root("XDG_DATA_HOME", &home_dir().join(".local/share"))
}

/// `~/.cache` (or `$XDG_CACHE_HOME`).
pub fn cache_root() -> PathBuf {
    xdg_root("XDG_CACHE_HOME", &home_dir().join(".cache"))
}

/// `~/.config/cashell/`
pub fn config_dir() -> PathBuf {
    config_root().join("cashell")
}

/// `~/.config/cashell/config.toml`
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// `~/.config/cashell/cashell.db`
pub fn db_path() -> PathBuf {
    config_dir().join("cashell.db")
}

/// `~/.config/cashell/cashell.db-wal`
pub fn db_wal_path() -> PathBuf {
    config_dir().join("cashell.db-wal")
}

/// `~/.config/cashell/cashell.db-shm`
pub fn db_shm_path() -> PathBuf {
    config_dir().join("cashell.db-shm")
}

/// `~/.config/cashell/pro_session.json` (Pro tokens — sensitive)
pub fn pro_session_path() -> PathBuf {
    config_dir().join("pro_session.json")
}

/// `~/.config/cashell/ssh_known_hosts` (daemon SSH host keys — sensitive)
pub fn ssh_known_hosts_path() -> PathBuf {
    config_dir().join("ssh_known_hosts")
}

/// `~/.config/cashell/secrets.enc` (encrypted secrets store — sensitive, F-02)
pub fn secrets_path() -> PathBuf {
    config_dir().join("secrets.enc")
}

/// `~/.local/share/cashell/`
pub fn data_dir() -> PathBuf {
    data_root().join("cashell")
}

/// `~/.local/share/cashell/license.json` (license key — sensitive)
pub fn license_path() -> PathBuf {
    data_dir().join("license.json")
}

/// `~/.local/share/cashell/hwid` (machine fingerprint)
pub fn hwid_path() -> PathBuf {
    data_dir().join("hwid")
}

/// `~/.local/share/cashell/vault` (knowledge-base vault, git repo)
pub fn vault_dir() -> PathBuf {
    data_dir().join("vault")
}

/// `~/.local/share/cashell/daemon.pid` (R-03: daemon PID file)
pub fn daemon_pid_path() -> PathBuf {
    data_dir().join("daemon.pid")
}

/// `~/.cache/cashell/`
pub fn cache_dir() -> PathBuf {
    cache_root().join("cashell")
}

/// `~/.cache/cashell/pro-fastboot.json`
pub fn fastboot_cache_path() -> PathBuf {
    cache_dir().join("pro-fastboot.json")
}

/// `~/.cache/cashell/backups/` (legacy `backup` module snapshots)
pub fn backups_dir() -> PathBuf {
    cache_dir().join("backups")
}

/// User systemd unit installed by `daemon install-service --user`:
/// `~/.config/systemd/user/cashell-daemon.service`
pub fn user_systemd_unit() -> PathBuf {
    config_root()
        .join("systemd")
        .join("user")
        .join("cashell-daemon.service")
}

/// System systemd unit installed by `daemon install-service` (system scope):
/// `/etc/systemd/system/cashell-daemon.service` (not under home — needs sudo
/// to remove; reset only reports it, K7).
pub fn system_systemd_unit() -> PathBuf {
    std::path::PathBuf::from("/etc/systemd/system/cashell-daemon.service")
}

/// Per-user temp dir `/tmp/cashell-<uid>` (R-07 backup target convention).
/// Nothing is written there by default; reset removes it when present.
///
/// Uses the real `/tmp` (never `$TMPDIR`) so the reset list matches the K7
/// inventory exactly. The test seam `with_test_tmp()` (thread-local) or the
/// legacy `__TEST_TMP_DIR` env var override it for R-08.
pub fn tmp_dir() -> PathBuf {
    // take()/set() route (not `get()`: non-`Copy` `Cell::get()` is
    // rejected by current rustc — see `test_home_override`).
    let t = TEST_TMP_OVERRIDE.with(|c| c.take());
    TEST_TMP_OVERRIDE.with(|c| c.set(t.clone()));
    if let Some(t) = t {
        return t;
    }
    if let Some(t) = std::env::var_os("__TEST_TMP_DIR") {
        return t.into();
    }
    let uid: u32 = std::env::var("USER")
        .ok()
        .and_then(|u| u.parse().ok())
        .unwrap_or(0);
    PathBuf::from("/tmp").join(format!("cashell-{}", uid))
}

/// Complete list of CAShell-owned paths for factory reset (R-02 K7).
///
/// Order: files first, then directories, so directory removals see their
/// contents already gone. Reset walks this list, canonicalizes each entry,
/// and refuses to delete anything that escapes the roots below.
pub fn resettable_paths() -> Vec<PathBuf> {
    vec![
        // ~/.config/cashell/ files
        config_path(),
        db_path(),
        db_wal_path(),
        db_shm_path(),
        pro_session_path(),
        ssh_known_hosts_path(),
        secrets_path(),
        // ~/.local/share/cashell/ files & dirs
        license_path(),
        hwid_path(),
        daemon_pid_path(),
        vault_dir(),
        // ~/.cache/cashell/ files & dirs
        fastboot_cache_path(),
        backups_dir(),
        // user systemd unit
        user_systemd_unit(),
        // temp dir
        tmp_dir(),
        // top-level dirs last (parents of the files above)
        config_dir(),
        data_dir(),
        cache_dir(),
    ]
}

/// Canonical roots a resettable path must stay inside (symlink protection).
pub fn protected_roots() -> Vec<PathBuf> {
    vec![
        config_dir(),
        data_dir(),
        cache_dir(),
        user_systemd_unit(),
        system_systemd_unit(),
        tmp_dir(),
    ]
}

/// Verify `path` (canonicalized) lives inside one of the protected roots.
/// Returns an error naming the offending path when it escapes (e.g. a symlink
/// pointing CAShell's state outside its own dirs — K7 / R-02).
pub fn assert_inside_cashell(path: &std::path::Path) -> Result<(), String> {
    let canon = std::fs::canonicalize(path)
        .map_err(|e| format!("cannot canonicalize {}: {e}", path.display()))?;
    // Absent roots: walk up to the nearest existing ancestor as the
    // containment boundary (the CAShell dirs we ourselves create).
    let inside = protected_roots().iter().any(|root| {
        let croot = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        let croot = if croot.exists() {
            croot
        } else {
            let mut anc = croot.clone();
            while !anc.exists() {
                match anc.parent() {
                    Some(p) => anc = p.to_path_buf(),
                    None => break,
                }
            }
            anc
        };
        canon.starts_with(&croot)
    });
    if inside {
        Ok(())
    } else {
        Err(format!(
            "{} resolves outside CAShell roots (symlink? refusing to delete)",
            path.display()
        ))
    }
}

/// Remove a CAShell-owned file or directory after asserting it stays inside
/// the protected roots (K7). A missing path is not a failure (it may have
/// been removed already or was a stale entry) — it returns `Ok`.
/// Caller reports real errors.
pub fn remove_cashell_path(path: &PathBuf, desc: &str) -> Result<(), String> {
    // A path that does not exist at all is fine — nothing to remove.
    if !path.exists() {
        return Ok(());
    }
    // Assert before removing: canonicalize so the target is what we check.
    assert_inside_cashell(path)?;
    if path.is_dir() && !path.is_symlink() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("failed removing {desc} {}: {e}", path.display()))?;
    } else {
        std::fs::remove_file(path)
            .map_err(|e| format!("failed removing {desc} {}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cashell_home_override_relocates_tree() {
        // Thread-local override: no global env mutation, safe to run in
        // parallel with every other test in the process.
        let tmp = std::env::temp_dir().join("cashell-paths-test");
        let got = with_test_home(&tmp, || {
            assert_eq!(home_dir(), tmp);
            assert_eq!(config_dir(), tmp.join(".config/cashell"));
            assert_eq!(data_dir(), tmp.join(".local/share/cashell"));
            assert_eq!(cache_dir(), tmp.join(".cache/cashell"));
            assert_eq!(db_path(), tmp.join(".config/cashell/cashell.db"));
            assert_eq!(
                license_path(),
                tmp.join(".local/share/cashell/license.json")
            );
            assert_eq!(
                daemon_pid_path(),
                tmp.join(".local/share/cashell/daemon.pid")
            );
        });
        let _ = got;
        // Outside `with_test_home` the override must not leak.
        assert_ne!(home_dir(), tmp, "override leaked outside with_test_home");
    }

    #[test]
    fn test_resettable_list_covers_top_dirs() {
        let list = resettable_paths();
        assert!(list.contains(&config_dir()));
        assert!(list.contains(&data_dir()));
        assert!(list.contains(&cache_dir()));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// INVENTORY — every path CAShell reads or writes (R-01)
// ─────────────────────────────────────────────────────────────────────────────
//
// | Function               | Path (Linux)                              | Kind   | Contents / notes                              |
// |------------------------|--------------------------------------------|--------|-------------------------------------------------|
// | `home_dir`             | `$HOME` (or `$CASHELL_HOME` override)      | dir    | Root of the whole tree (K7 test seam)            |
// | `config_root`          | `$XDG_CONFIG_HOME` or `~/.config`          | dir    | XDG config root                                  |
// | `data_root`            | `$XDG_DATA_HOME` or `~/.local/share`       | dir    | XDG data root                                    |
// | `cache_root`           | `$XDG_CACHE_HOME` or `~/.cache`            | dir    | XDG cache root                                   |
// | `config_dir`           | `~/.config/cashell`                        | dir    | CAShell config root                              |
// | `config_path`          | `~/.config/cashell/config.toml`            | file   | Main config (tokens, keys, secrets — sensitive)  |
// | `db_path`              | `~/.config/cashell/cashell.db`             | file   | SQLite database (events, audit, devices)         |
// | `db_wal_path`          | `~/.config/cashell/cashell.db-wal`         | file   | SQLite WAL journal                               |
// | `db_shm_path`          | `~/.config/cashell/cashell.db-shm`         | file   | SQLite shared memory                             |
// | `pro_session_path`     | `~/.config/cashell/pro_session.json`       | file   | Pro JWT session (sensitive)                      |
// | `ssh_known_hosts_path` | `~/.config/cashell/ssh_known_hosts`        | file   | Daemon SSH host keys (sensitive)                 |
// | `data_dir`             | `~/.local/share/cashell`                   | dir    | CAShell data root                                |
// | `license_path`         | `~/.local/share/cashell/license.json`      | file   | License key (sensitive)                          |
// | `hwid_path`            | `~/.local/share/cashell/hwid`              | file   | Cached machine HWID                              |
// | `vault_dir`            | `~/.local/share/cashell/vault`             | dir    | Knowledge-base vault (git)                       |
// | `daemon_pid_path`      | `~/.local/share/cashell/daemon.pid`        | file   | R-03 daemon PID file                             |
// | `cache_dir`            | `~/.cache/cashell`                         | dir    | CAShell cache root                               |
// | `fastboot_cache_path`  | `~/.cache/cashell/pro-fastboot.json`       | file   | Pro fast-boot cache                              |
// | `backups_dir`          | `~/.cache/cashell/backups`                 | dir    | Legacy snapshot archives (may hold secrets)      |
// | `user_systemd_unit`    | `~/.config/systemd/user/cashell-daemon.service` | file | User systemd unit (systemd dir, CAShell-owned)   |
// | `system_systemd_unit`  | `/etc/systemd/system/cashell-daemon.service`  | file | System unit — NOT under home; reset reports only  |
// | `tmp_dir`              | `/tmp/cashell-<uid>`                          | dir    | Per-user temp scratch (R-07 target; `$__TEST_TMP_DIR` in R-08) |
//
// Reset (R-02) removes exactly: `resettable_paths()` — i.e. everything above
// except `system_systemd_unit` (reported, needs sudo) and `home_dir` roots.
// Nothing outside these roots is ever touched.
