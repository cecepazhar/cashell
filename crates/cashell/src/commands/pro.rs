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

fn iso_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| format!("unix:{ts}"))
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
        let p =
            prompt_secret("Zero-knowledge master passphrase (leave empty for trial): ")?;
        return Ok(p);
    }
    Ok(String::new())
}

fn load_active_session(_cfg: &Config) -> Result<SessionManager> {
    match SessionManager::load_default() {
        Some(sm) => Ok(sm),
        None => bail!("No active Pro session. Run `cashell pro login` first."),
    }
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

fn run_login(args: &ProLoginArgs, cfg: &Config) -> Result<i32> {
    let email = match args.email.as_deref().filter(|e| !e.is_empty()) {
        Some(e) => e.to_string(),
        None => {
            let input = prompt_secret("Pro account email: ")?;
            if input.is_empty() {
                output::error("No email provided — login cancelled.");
                return Ok(3);
            }
            input
        }
    };

    let token = match args.token.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => {
            if args.trial {
                String::new()
            } else {
                prompt_secret("Pro license token (leave empty to activate 7-day trial): ")
                    .map_err(|e| anyhow!("input error: {e}"))?
            }
        }
    };

    let api_base = args
        .api_base
        .clone()
        .unwrap_or_else(|| cfg.pro.api_base.clone());

    let hwid = hwid()?;
    let now_unix = now_unix_ts();

    let sm = if token.is_empty() {
        let secret = format!("cashell-trial-{hwid}");
        SessionManager::create_session(&email, &hwid, &api_base, secret.as_bytes())?
    } else {
        SessionManager::new(
            token.clone(),
            String::new(),
            hwid.clone(),
            api_base.clone(),
        )
    };

    sm.save_default()?;

    let is_trial = args.trial || token.is_empty();
    let expiry_secs = if is_trial {
        7 * 24 * 3600_i64
    } else {
        30 * 24 * 3600_i64
    };
    let cache = FastBootCache {
        hwid: hwid.clone(),
        status: 1,
        session_expires_unix: now_unix + expiry_secs,
        api_base: api_base.clone(),
        last_sync: now_unix,
    };
    cache.save()?;

    let mut updated_cfg = cfg.clone();
    updated_cfg.pro.account_email = email.clone();
    if args.api_base.is_some() {
        updated_cfg.pro.api_base = api_base.clone();
    }
    if let Err(e) = updated_cfg.save(&Config::default_path()) {
        eprintln!("warning: could not persist config file: {e}");
    }

    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "this-device".to_string());
    let device_id = hwid.clone();
    let label = format!("{hostname} ({email})");
    if let Ok(db) = crate::db::Database::open() {
        let extra =
            serde_json::json!({ "email": &email, "api_base": &api_base, "is_trial": is_trial });
        let _ = db.upsert_pro_device(
            &device_id,
            &label,
            "active",
            None,
            Some(&extra.to_string()),
        );
    }

    if wants_json(args.json) {
        println!(
            "{}",
            serde_json::json!({
                "login": "ok",
                "email": email,
                "hwid": hwid,
                "api_base": api_base,
                "trial": is_trial,
                "expires_unix": now_unix + expiry_secs,
            })
        );
    } else {
        let badge = if is_trial {
            "★ TRIAL (7-day)".yellow().to_string()
        } else {
            "★ PRO".green().to_string()
        };
        println!(
            "{}",
            format!("✓ Pro account activated for {email} — {badge}").green().bold()
        );
        println!("  {} {}", "HWID:".cyan().bold(), hwid);
        println!(
            "  {} {}",
            "Expires:".cyan().bold(),
            iso_timestamp(now_unix + expiry_secs)
        );
        println!(
            "  {} {}",
            "Session file:".cyan().bold(),
            SessionManager::default_path().display()
        );
    }

    Ok(0)
}

// ── pro status ────────────────────────────────────────────────────────────────

fn run_status(args: &ProStatusArgs, cfg: &Config) -> Result<i32> {
    let fast = FastBootCache::load();
    let fast_valid = fast.as_ref().map(|c| c.is_valid()).unwrap_or(false);
    let session = SessionManager::load_default();

    let is_pro = fast_valid || session.is_some() || !cfg.pro.account_email.is_empty();
    let hwid_val = fast
        .as_ref()
        .map(|c| c.hwid.clone())
        .unwrap_or_else(|| hwid().unwrap_or_else(|_| "unknown".to_string()));

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
            pro_active: is_pro,
            trial_active: !is_pro,
            hwid: hwid_val,
            account_email: cfg.pro.account_email.clone(),
            api_base: cfg.pro.api_base.clone(),
            auto_sync: cfg.pro.auto_sync,
            sync_interval_mins: cfg.pro.sync_interval_mins,
            session: session_json,
            fast_boot_valid: fast_valid,
        })
        .map_err(|e| anyhow!("json: {e}"))?;
        println!("{body}");
        Ok(0)
    } else {
        let pro_label = if is_pro {
            "★ PRO ACTIVE".green().to_string()
        } else {
            "○ FREE TIER".yellow().to_string()
        };
        println!("  Pro status:  {pro_label}");
        println!("  HWID:        {hwid_val}");
        println!(
            "  Email:       {}",
            if cfg.pro.account_email.is_empty() {
                "(unlicensed)".to_string()
            } else {
                cfg.pro.account_email.clone()
            }
        );
        println!("  API base:    {}", cfg.pro.api_base);
        println!(
            "  Auto-sync:   {} ({})",
            if cfg.pro.auto_sync { "ON".to_string() } else { "OFF".to_string() },
            cfg.pro.sync_interval_mins,
        );
        if let Some(sm) = &session {
            println!(
                "  Session:     access={} refresh={} ({}s left)",
                if sm.is_access_expired() {
                    "EXPIRED".red().to_string()
                } else {
                    "VALID".green().to_string()
                },
                if sm.is_refresh_expired() {
                    "EXPIRED".red().to_string()
                } else {
                    "VALID".green().to_string()
                },
                sm.seconds_until_access_expiry(),
            );
        } else {
            println!("  Session:     not persisted (login required)");
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
            format!("✓ GCC sync complete (vault: {} bytes, pushed={pushed})", merged.len()).green().bold()
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
            .map(
                |d| DeviceJson {
                    device_id: d.device_id,
                    device_label: d.device_label,
                    status: d.status,
                    last_seen: d.last_seen,
                    last_error: d.last_error,
                    updated_at: d.updated_at,
                },
            )
            .collect();
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!("{}", "Registered Pro devices:".bold());
        for d in &rows {
            println!(
                "  [{}] {}  ({})",
                d.status,
                d.device_label,
                d.device_id
            );
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
        println!("{}", serde_json::json!({ "logout": "ok", "hwid": hwid_val }));
    } else {
        println!(
            "{} Pro session revoked and fast-boot cache invalidated.",
            "✓".green().bold()
        );
    }
    Ok(0)
}
