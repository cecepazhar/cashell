//! `cashell pro` command group — login, status, sync, push, pull, devices, logout.
//!
//! Handlers for the Pro (multi-device zero-knowledge GCC sync) feature.
//!
//! - `login`   — activate Pro account (bind HWID, persist session, update config + fast-boot cache)
//! - `status`  — show Pro session state, HWID, tier, sync metadata summary
//! - `sync`    — full GCC sync cycle: pull encrypted vault → resolve → push back
//! - `push`    — push encrypted local vault state to GCC REST API
//! - `pull`    — pull & decrypt remote vault state from GCC REST API
//! - `devices` — manage known multi-device entries (`pro_state` SQLite table)
//! - `logout`  — revoke session files, clear config, invalidate fast-boot cache

use crate::cli::{
    ProCommand, ProDevicesAction, ProDevicesListArgs, ProLoginArgs, ProLogoutArgs, ProMode,
    ProPullArgs, ProPushArgs, ProStatusArgs, ProSyncArgs,
};
use crate::config::Config;
use crate::output;
use crate::pro::{FastBootCache, HwidGenerator, SessionManager};
use crate::sync::SyncEngine;
use anyhow::{anyhow, bail, Result};
use colored::Colorize;
use serde::Serialize;
use std::io::{self, Write};
use std::path::Path;

// ── Exit-code semantics ────────────────────────────────────────────────────────
// 0 = success
// 1 = not logged in / no Pro session
// 2 = operational error
// 3 = cancelled by user

pub fn run(cmd: &ProCommand, cfg: &Config) -> Result<i32> {
    match &cmd.mode {
        ProMode::Login(args) => run_login(args, cfg),
        ProMode::Status(args) => run_status(args, cfg),
        ProMode::Sync(args) => run_sync(args, cfg),
        ProMode::Push(args) => run_push(args, cfg),
        ProMode::Pull(args) => run_pull(args, cfg),
        ProMode::Devices(args) => run_devices(args, cfg),
        ProMode::Logout(args) => run_logout(args, cfg),
    }
}

// ── Shared helpers ─────────────────────────────────────────────────────────────

fn hwid() -> Result<String> {
    HwidGenerator::new().get_or_create()
}

fn mask_token(t: &str) -> String {
    if t.len() <= 10 {
        return "••••".to_string();
    }
    format!("{}…{}", &t[..4], &t[t.len() - 4..])
}

fn prompt_secret(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn wants_json(explicit: bool) -> bool {
    explicit || !crate::output::is_tty()
}

fn now_unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn resolve_passphrase(flag_val: Option<&str>) -> Result<String> {
    if let Some(v) = flag_val {
        if !v.trim().is_empty() {
            return Ok(v.to_string());
        }
    }
    if let Ok(v) = std::env::var("CASHELL_PRO_PASS") {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    if crate::output::is_tty() {
        let p = prompt_secret("Zero-knowledge master passphrase (leave empty for trial): ")?;
        return Ok(p);
    }
    Ok(String::new())
}

fn load_active_session(cfg: &Config) -> Result<SessionManager> {
    let lic = match crate::license_local::load_license()? {
        Some(l) if crate::license_local::is_license_valid(&l) => l,
        _ => {
            bail!("Fitur Pro memerlukan lisensi aktif. Gunakan `cashell license activate <KEY>` untuk mengaktifkan lisensi Pro.");
        }
    };

    let api_base = if !cfg.pro.api_base.is_empty() {
        &cfg.pro.api_base
    } else if !cfg.license.gcc_base_url.is_empty() {
        &cfg.license.gcc_base_url
    } else {
        "https://gcc.fathforce.com"
    };

    SessionManager::from_license(&lic, api_base)
}

// ── JSON output structs ───────────────────────────────────────────────────────

#[derive(Serialize, Debug)]
struct StatusJson {
    pro_active: bool,
    trial_active: bool,
    hwid: String,
    account_email: String,
    api_base: String,
    auto_sync: bool,
    sync_interval_mins: u32,
    session: Option<SessionJson>,
    fast_boot_valid: bool,
}

#[derive(Serialize, Debug)]
struct SessionJson {
    access_masked: String,
    refresh_masked: String,
    hwid: String,
    api_base: String,
    access_expires_unix: i64,
    refresh_expires_unix: i64,
    access_expired: bool,
    refresh_expired: bool,
}

#[derive(Serialize, Debug)]
struct SyncResultJson {
    success: bool,
    pushed: bool,
    pulled: bool,
    bytes_transferred: usize,
    etag: Option<String>,
    error: Option<String>,
}

#[derive(Serialize, Debug)]
struct DeviceJson {
    device_id: String,
    device_label: String,
    status: String,
    last_seen: Option<String>,
    last_error: Option<String>,
    updated_at: Option<String>,
}

// ── pro login ─────────────────────────────────────────────────────────────────

fn run_login(args: &ProLoginArgs, _cfg: &Config) -> Result<i32> {
    if wants_json(args.json) {
        println!(
            "{}",
            serde_json::json!({
                "status": "deprecated",
                "message": "Gunakan `cashell license activate` untuk mengaktifkan lisensi Pro.",
                "hint": "cashell license activate <KEY>"
            })
        );
    } else {
        println!(
            "{} Perintah `cashell pro login` telah disatukan dengan aktivasi lisensi.",
            "ℹ".cyan().bold()
        );
        println!(
            "  Silakan gunakan: {}",
            "cashell license activate <KEY>".green().bold()
        );
    }
    Ok(0)
}

// ── pro status ────────────────────────────────────────────────────────────────

fn run_status(args: &ProStatusArgs, cfg: &Config) -> Result<i32> {
    let lic = crate::license_local::load_license().ok().flatten();
    let is_lic_valid = lic
        .as_ref()
        .map(crate::license_local::is_license_valid)
        .unwrap_or(false);
    let session = lic.as_ref().filter(|_| is_lic_valid).and_then(|l| {
        let api_base = if !cfg.pro.api_base.is_empty() {
            &cfg.pro.api_base
        } else if !cfg.license.gcc_base_url.is_empty() {
            &cfg.license.gcc_base_url
        } else {
            "https://gcc.fathforce.com"
        };
        SessionManager::from_license(l, api_base).ok()
    });

    let hwid_val = hwid().unwrap_or_else(|_| "unknown".to_string());

    if wants_json(args.json) {
        let session_json = session.as_ref().map(|sm| SessionJson {
            access_masked: mask_token(sm.access_token()),
            refresh_masked: mask_token(sm.refresh_token()),
            hwid: sm.hwid().to_string(),
            api_base: sm.api_base.clone(),
            access_expires_unix: sm.access_expires_unix(),
            refresh_expires_unix: sm.refresh_expires_unix(),
            access_expired: sm.is_access_expired(),
            refresh_expired: sm.is_refresh_expired(),
        });
        let body = serde_json::to_string_pretty(&StatusJson {
            pro_active: is_lic_valid,
            trial_active: false,
            hwid: hwid_val,
            account_email: cfg.pro.account_email.clone(),
            api_base: if !cfg.pro.api_base.is_empty() {
                cfg.pro.api_base.clone()
            } else {
                cfg.license.gcc_base_url.clone()
            },
            auto_sync: cfg.pro.auto_sync,
            sync_interval_mins: cfg.pro.sync_interval_mins,
            session: session_json,
            fast_boot_valid: is_lic_valid,
        })
        .map_err(|e| anyhow!("json: {e}"))?;
        println!("{body}");
        Ok(0)
    } else {
        let pro_label = if is_lic_valid {
            let tier = lic.as_ref().map(|l| l.tier.as_str()).unwrap_or("PRO");
            let tier_str = match tier {
                "99" => "PRO (Lifetime)",
                "24" => "PRO (2-Year)",
                "12" => "PRO (Annual)",
                "01" => "PRO (Monthly)",
                _ => "PRO",
            };
            format!("★ {tier_str} ACTIVE").green().to_string()
        } else {
            "○ FREE TIER (unlicensed)".yellow().to_string()
        };
        println!("  Pro status:  {pro_label}");
        println!("  HWID:        {hwid_val}");
        if let Some(l) = &lic {
            println!("  License:     {}", crate::license_gcc::mask_key(&l.key));
            if let Some(exp) = &l.expires_at {
                println!("  Expires:     {exp}");
            } else {
                println!("  Expires:     Lifetime");
            }
        } else {
            println!("  License:     (no active license installed)");
            println!("  Hint:        Use `cashell license activate <KEY>` to unlock Pro");
        }
        let api_base = if !cfg.pro.api_base.is_empty() {
            &cfg.pro.api_base
        } else {
            &cfg.license.gcc_base_url
        };
        println!("  API base:    {api_base}");
        println!(
            "  Auto-sync:   {} ({})",
            if cfg.pro.auto_sync { "ON" } else { "OFF" },
            cfg.pro.sync_interval_mins,
        );
        if let Some(sm) = &session {
            println!(
                "  Session:     token={} (valid)",
                mask_token(sm.access_token())
            );
        } else {
            println!("  Session:     no active Pro license");
        }

        if let Ok(db) = crate::db::Database::open() {
            if let Ok(rows) = db.get_pro_sync_metadata() {
                if !rows.is_empty() {
                    println!();
                    println!("  Last sync events:");
                    for r in rows.iter().take(5) {
                        println!(
                            "    {}/[{}] revision={} — {}",
                            r.entity_type, r.entity_id, r.revision, r.sync_state
                        );
                    }
                }
            }
        }
        Ok(0)
    }
}

// ── pro sync ──────────────────────────────────────────────────────────────────

fn run_sync(args: &ProSyncArgs, cfg: &Config) -> Result<i32> {
    let session = load_active_session(cfg)?;

    let passphrase = resolve_passphrase(args.passphrase.as_deref())?;

    let engine = SyncEngine::new(session.clone(), passphrase);
    let hwid = session.hwid().to_string();

    let remote: Option<String> = engine.pull().unwrap_or_else(|e| {
        eprintln!("warning: pull failed ({e}) — continuing with local-only sync.");
        None
    });

    // TODO(T-07): gate multi-user Telegram alerts behind Pro entitlements
    // (crate::entitlements::current().is_pro).

    let local_vault_path = crate::sync::ZERO_KNOWLEDGE_VAULT_PATH.to_string();
    let local = std::fs::read_to_string(&local_vault_path).unwrap_or_default();
    let remote_text = remote.clone().unwrap_or_default();

    let merged: String = if local.len() >= remote_text.len() {
        local.clone()
    } else {
        remote_text.clone()
    };

    if let Some(parent) = Path::new(&local_vault_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if merged != local {
        std::fs::write(&local_vault_path, &merged)?;
    }

    let pushed = if merged.is_empty() {
        false
    } else {
        match engine.push(&merged) {
            Ok(res) => {
                let etag = format!("local-{hwid}");
                if let Ok(db) = crate::db::Database::open() {
                    let _ = db.upsert_pro_sync_metadata(
                        "vault",
                        &hwid,
                        res.bytes_transferred as i64,
                        Some(&etag),
                        "synced",
                    );
                }
                res.success
            }
            Err(e) => {
                eprintln!("warning: push failed ({e})");
                false
            }
        }
    };

    let cache = FastBootCache {
        hwid: hwid.clone(),
        status: 1,
        session_expires_unix: session.access_expires_unix(),
        api_base: session.api_base.clone(),
        last_sync: now_unix_ts(),
    };
    let _ = cache.save();

    if wants_json(args.json) {
        let body = serde_json::to_string(&SyncResultJson {
            success: pushed,
            pushed,
            pulled: remote.as_ref().map(|r| !r.is_empty()).unwrap_or(false),
            bytes_transferred: merged.len(),
            etag: Some(format!("local-{hwid}")),
            error: None,
        })
        .map_err(|e| anyhow!("json: {e}"))?;
        println!("{body}");
    } else {
        println!(
            "{}",
            format!(
                "✓ GCC sync complete (vault: {} bytes, pushed={pushed})",
                merged.len()
            )
            .green()
            .bold()
        );
        if !remote_text.is_empty() && remote_text != local {
            println!(
                "  pulled: {} bytes remote vault was merged in",
                remote_text.len()
            );
        }
        if !local.is_empty() {
            println!("  local:  {} bytes (at {local_vault_path})", local.len());
        }
    }

    Ok(0)
}

// ── pro push ──────────────────────────────────────────────────────────────────

fn run_push(args: &ProPushArgs, cfg: &Config) -> Result<i32> {
    let session = load_active_session(cfg)?;
    let passphrase = resolve_passphrase(args.passphrase.as_deref())?;
    let engine = SyncEngine::new(session.clone(), passphrase);

    let local_vault_path = crate::sync::ZERO_KNOWLEDGE_VAULT_PATH.to_string();
    let local = std::fs::read_to_string(&local_vault_path).unwrap_or_default();

    if local.is_empty() {
        eprintln!("Local vault file is empty — nothing to push.");
        return Ok(1);
    }

    let hwid = session.hwid().to_string();
    match engine.push(&local) {
        Ok(res) => {
            let etag = format!("push-{hwid}");
            if let Ok(db) = crate::db::Database::open() {
                let _ = db.upsert_pro_sync_metadata(
                    "vault",
                    &hwid,
                    res.bytes_transferred as i64,
                    Some(&etag),
                    "pushed",
                );
            }
            if wants_json(args.json) {
                println!(
                    "{}",
                    serde_json::json!({ "success": true, "bytes": res.bytes_transferred, "etag": etag })
                );
            } else {
                println!(
                    "{} ✓ pushed {} bytes to GCC REST API.",
                    "Vault".cyan(),
                    res.bytes_transferred
                );
            }
            Ok(0)
        }
        Err(e) => {
            output::error(&format!("push failed: {e}"));
            Ok(2)
        }
    }
}

// ── pro pull ──────────────────────────────────────────────────────────────────

fn run_pull(args: &ProPullArgs, cfg: &Config) -> Result<i32> {
    let session = load_active_session(cfg)?;
    let passphrase = resolve_passphrase(args.passphrase.as_deref())?;
    let engine = SyncEngine::new(session.clone(), passphrase);

    match engine.pull() {
        Ok(Some(vault_text)) => {
            let target = args
                .out
                .clone()
                .unwrap_or_else(|| crate::sync::ZERO_KNOWLEDGE_VAULT_PATH.to_string());
            let path = Path::new(&target);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(path, &vault_text)?;
            if wants_json(args.json) {
                println!(
                    "{}",
                    serde_json::json!({ "pulled_bytes": vault_text.len(), "output": target })
                );
            } else {
                println!(
                    "{} {} bytes written to {}",
                    "Pulled".green(),
                    vault_text.len(),
                    target
                );
            }
            Ok(0)
        }
        Ok(None) => {
            eprintln!("No remote vault found on GCC server (404).");
            Ok(1)
        }
        Err(e) => {
            output::error(&format!("pull failed: {e}"));
            Ok(2)
        }
    }
}

// ── pro devices ───────────────────────────────────────────────────────────────

fn run_devices(args: &crate::cli::ProDevicesArgs, _cfg: &Config) -> Result<i32> {
    let db = crate::db::Database::open().map_err(|e| anyhow!("cannot open database: {e}"))?;
    match &args.action {
        ProDevicesAction::List(list_args) => devices_list(list_args),
        ProDevicesAction::Remove { device_id } => {
            let n = db.delete_pro_device(device_id)?;
            if n > 0 {
                println!("{} Device {} removed.", "✓".green().bold(), device_id);
                Ok(0)
            } else {
                eprintln!("No device found with id '{device_id}'");
                Ok(1)
            }
        }
        ProDevicesAction::Rename { device_id, label } => {
            let hwid_now = hwid().unwrap_or_default();
            if device_id.as_str() == hwid_now.as_str() {
                eprintln!("Refusing to rename this device (HWID match).");
                return Ok(2);
            }
            let _ = db.upsert_pro_device(device_id, label, "active", None, None);
            println!(
                "{} Device '{}' renamed to '{}'.",
                "✓".green().bold(),
                device_id,
                label
            );
            Ok(0)
        }
    }
}

fn devices_list(args: &ProDevicesListArgs) -> Result<i32> {
    let db = crate::db::Database::open()?;
    let rows = db.get_pro_devices()?;
    if rows.is_empty() {
        if wants_json(args.json) {
            println!("[]");
        } else {
            eprintln!("No registered devices found.");
        }
        return Ok(0);
    }
    if wants_json(args.json) {
        let json: Vec<DeviceJson> = rows
            .into_iter()
            .map(|d| DeviceJson {
                device_id: d.device_id,
                device_label: d.device_label,
                status: d.status,
                last_seen: d.last_seen,
                last_error: d.last_error,
                updated_at: d.updated_at,
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!("{}", "Registered Pro devices:".bold());
        for d in &rows {
            println!("  [{}] {}  ({})", d.status, d.device_label, d.device_id);
        }
    }
    Ok(0)
}

// ── pro logout ────────────────────────────────────────────────────────────────

fn run_logout(args: &ProLogoutArgs, cfg: &Config) -> Result<i32> {
    let hwid_val = HwidGenerator::new()
        .get_or_create()
        .unwrap_or_else(|_| "unknown".to_string());

    let _ = SessionManager::delete_default();

    if let Ok(db) = crate::db::Database::open() {
        let _ = db.upsert_pro_device(&hwid_val, "", "inactive", None, None);
    }

    let cache = FastBootCache {
        hwid: hwid_val.clone(),
        status: 0,
        session_expires_unix: 0,
        api_base: cfg.pro.api_base.clone(),
        last_sync: now_unix_ts(),
    };
    let _ = cache.save();

    let mut updated_cfg = cfg.clone();
    updated_cfg.pro.account_email.clear();
    if let Err(e) = updated_cfg.save(&Config::default_path()) {
        eprintln!("warning: could not persist config file: {e}");
    }

    if wants_json(args.json) {
        println!(
            "{}",
            serde_json::json!({ "logout": "ok", "hwid": hwid_val })
        );
    } else {
        println!(
            "{} Pro session revoked and fast-boot cache invalidated.",
            "✓".green().bold()
        );
    }
    Ok(0)
}

// ── tests (L-12) ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pro_login_alias_guidance() {
        let args = ProLoginArgs {
            email: None,
            token: None,
            api_base: None,
            trial: false,
            json: true,
        };
        let cfg = Config::default();
        let res = run_login(&args, &cfg);
        assert_eq!(res.unwrap(), 0);
    }

    #[test]
    fn test_load_active_session_unlicensed_fails() {
        let tmp =
            std::env::temp_dir().join(format!("cashell-pro-unlicensed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        crate::paths::with_test_home(&tmp, || {
            crate::entitlements::invalidate_cache();
            let cfg = Config::default();
            let res = load_active_session(&cfg);
            assert!(res.is_err());
            let err_msg = res.unwrap_err().to_string();
            assert!(
                err_msg.contains("Fitur Pro memerlukan lisensi aktif"),
                "err: {err_msg}"
            );
        });
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_load_active_session_with_license() {
        let tmp = std::env::temp_dir().join(format!("cashell-pro-licensed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        crate::paths::with_test_home(&tmp, || {
            crate::entitlements::invalidate_cache();
            let lic = crate::license_local::LicenseFile {
                schema: 2,
                key: "CASH99-FOUNDR-3".to_string(),
                product: "SH".to_string(),
                tier: "99".to_string(),
                expires_at: None,
                activated_at: "2026-11-13T00:00:00Z".to_string(),
                last_verified_at: "2026-11-13T00:00:00Z".to_string(),
                activation_token: None, // V-04: live GCC fallback when token is absent
                devices_used: 1,
                devices_max: 3,
                offline_grace_until: None,
                revoked: false,
                expired: false,
            };
            crate::license_local::save_license(&lic).unwrap();
            crate::entitlements::invalidate_cache();

            let cfg = Config::default();
            let sm = load_active_session(&cfg).expect("active session loaded");
            assert_eq!(sm.access_token(), "CASH99-FOUNDR-3");
            assert_eq!(sm.api_base, "https://gcc.fathforce.com");
        });
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
