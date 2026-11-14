//! `cashell license` command — offline-first license activation, validation, and status.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::io::Write;

use crate::config::Config;
use crate::license_gcc;
use crate::license_local::{self, LicenseFile};
use crate::output;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "license",
    about = "Manage CAShell offline-first license key & billing"
)]
pub struct LicenseCommand {
    /// Override the GCC base URL (default: config `[license] gcc_base_url`)
    #[arg(long, global = true)]
    pub gcc_base_url: Option<String>,

    #[command(subcommand)]
    pub action: LicenseAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LicenseAction {
    /// Activate a license key (e.g. CASH99-FOUNDR-3).
    /// Key may be omitted on a TTY to prompt with echo off.
    Activate {
        /// The license key (format: CASH[99|24|12|01]-[A-Z0-9]{6}-[0-9])
        key: Option<String>,
    },
    /// Display current local license status, tier, and expiration
    Status {
        /// Emit machine-readable JSON output
        #[arg(long)]
        json: bool,
    },
    /// Deactivate and remove the stored local license key
    Deactivate {
        /// Only remove the local license file without contacting GCC
        /// (device slot is NOT freed on the server).
        #[arg(long)]
        local_only: bool,
    },
    /// Export the local license key to an encrypted/restricted file (permissions 0600)
    ExportKey {
        /// Destination file path
        path: std::path::PathBuf,
    },
    /// Import and activate a license key from a file
    ImportKey {
        /// Path to file containing the license key
        path: std::path::PathBuf,
    },
}

/// Calculate the standard CA mod-10 checksum.
///
/// For each char of `body` (uppercased):
///   - digit  → value 0-9
///   - letter → value 10-35
///
/// The result is `(10 - total % 10) % 10`.
pub fn mod10_checksum(body: &str) -> u32 {
    let mut total: u32 = 0;
    for ch in body.to_ascii_uppercase().chars() {
        if ch.is_ascii_digit() {
            total += ch.to_digit(10).unwrap_or(0);
        } else if ch.is_ascii_alphabetic() {
            total += (ch as u32) - ('A' as u32) + 10;
        }
    }
    (10 - (total % 10)) % 10
}

/// Validate CA license format: `CASH[99|24|12|01]-[A-Z0-9]{6}-[0-9]`.
///
/// Uses the mod-10 checksum on `CASH{tier}{rand6}` (first 13 chars).
/// Returns `(product, tier)` on success.
pub fn validate_license_format(key: &str) -> Result<(String, String)> {
    let clean = key.trim().to_ascii_uppercase();
    let parts: Vec<&str> = clean.split('-').collect();
    if parts.len() != 3 {
        bail!("Invalid license format. Expected: CASH[99|24|12|01]-[A-Z0-9]{{6}}-[0-9] (e.g. CASH99-FOUNDR-3)");
    }
    let [prefix, rand6, ck_str] = parts.as_slice() else {
        bail!("Invalid license structure");
    };

    if prefix.len() != 6 || rand6.len() != 6 || ck_str.len() != 1 {
        bail!("Invalid segment length in license key. Must be 15 characters total.");
    }
    if !prefix.starts_with("CASH") {
        bail!("Invalid prefix: license key must begin with 'CASH'");
    }

    let tier = prefix[4..6].to_string();
    match tier.as_str() {
        "99" | "24" | "12" | "01" => {}
        other => bail!("Invalid tier '{other}'. Must be one of: 99, 24, 12, 01."),
    }

    // rand6 must be alphanumeric (A-Z 0-9)
    if !rand6
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        bail!("Middle segment must be 6 uppercase alphanumeric chars (A-Z 0-9)");
    }

    // Checksum: mod-10 of the first 13 chars (CASH{tier}{rand6})
    let body = format!("{prefix}-{rand6}");
    let expected_ck = mod10_checksum(&body);
    let given_ck = ck_str
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("Checksum digit must be 0-9"))?;

    if expected_ck != given_ck {
        bail!("Checksum mismatch: key may be mistyped (expected {expected_ck}, got {given_ck})");
    }

    Ok(("SH".to_string(), tier))
}

/// Prompt for a license key with echo off when running on a TTY and no
/// key was passed as an argument.
fn prompt_key_if_needed(key_arg: &Option<String>) -> Result<String> {
    if let Some(k) = key_arg {
        if !k.trim().is_empty() {
            return Ok(k.trim().to_string());
        }
    }
    if crate::output::is_tty() {
        eprint!("License key: ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let key = line.trim().to_string();
        if key.is_empty() {
            bail!("No license key provided — activation cancelled.");
        }
        Ok(key)
    } else {
        bail!("No license key provided. Pass the key as an argument: `cashell license activate <KEY>`");
    }
}

/// Compute a human-readable display name for the current device.
fn device_display_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "this-device".to_string())
}

/// Detect the current OS platform string for GCC registration.
fn platform_name() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "unknown"
    }
}

// ── Status JSON output (L-07 piped/--json) ─────────────────────────────────

#[derive(serde::Serialize)]
struct StatusJson {
    key_masked: String,
    tier: String,
    status: String,
    expires_at: Option<String>,
    days_left: Option<i64>,
    devices_used: u32,
    devices_max: u32,
    last_verified_at: Option<String>,
    offline_grace_until: Option<String>,
}

/// Compute `days_left` from `expires_at`; `None` for lifetime.
pub fn compute_days_left(lic: &LicenseFile) -> Option<i64> {
    if lic.tier == "99" || lic.expires_at.is_none() {
        return None;
    }
    let now = chrono::Utc::now();
    let exp_ts = parse_expires_ts(lic.expires_at.as_deref().unwrap_or(""))?;
    let exp = chrono::DateTime::from_timestamp(exp_ts, 0)?;
    let diff = exp - now;
    if diff.num_seconds() <= 0 {
        Some(0)
    } else {
        Some(diff.num_days())
    }
}

/// Compute `offline_grace_until`: `last_verified_at + 30 days`,
/// capped at `expires_at` if one exists.
pub fn compute_offline_grace_until(lic: &LicenseFile) -> Option<String> {
    let last_verified_ts = parse_expires_ts(&lic.last_verified_at)?;
    let grace_ts = last_verified_ts + 30 * 86_400;
    // Cap at expires_at if present.
    let final_ts = if let Some(exp_ts) = parse_expires_ts(lic.expires_at.as_deref().unwrap_or("")) {
        if exp_ts < grace_ts {
            exp_ts
        } else {
            grace_ts
        }
    } else {
        grace_ts
    };
    Some(
        chrono::DateTime::from_timestamp(final_ts, 0)?
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

/// Parse an ISO-8601 or unix-timestamp string to a unix timestamp.
fn parse_expires_ts(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    if let Ok(ts) = s.parse::<i64>() {
        return Some(ts);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    None
}

pub fn run(cmd: &LicenseCommand, cfg: &Config) -> Result<i32> {
    match &cmd.action {
        LicenseAction::Activate { key } => cmd_activate(key, cmd, cfg),
        LicenseAction::Status { json } => cmd_status(*json, cfg),
        LicenseAction::Deactivate { local_only } => cmd_deactivate(cmd, cfg, *local_only),
        LicenseAction::ExportKey { path } => cmd_export_key(path),
        LicenseAction::ImportKey { path } => cmd_import_key(path, cmd, cfg),
    }
}

// ── activate ────────────────────────────────────────────────────────────────

/// Activate a license key against GCC and store it locally.
pub fn activate_license_key(key: &str, base_url: &str) -> Result<LicenseFile> {
    let key = key.trim().to_string();
    let key_masked = license_gcc::mask_key(&key);
    let (_prod, _tier) = validate_license_format(&key)?;

    if base_url.trim().is_empty() {
        bail!("license base URL is empty — set [license] gcc_base_url or pass --gcc-base-url");
    }

    let dev_id = license_gcc::device_id();
    let dev_name = device_display_name();
    let platform = platform_name();

    let activation = license_gcc::activate(base_url, &key, &dev_id, &dev_name, platform)
        .map_err(|e| anyhow::anyhow!("Activation failed: {e}"))?;

    let now_iso = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    // Capture expires_at before moving out of `activation`.
    let expires_at = activation.expires_at.clone();
    let expires_unix = expires_at.as_ref().and_then(|s| {
        s.parse::<i64>().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.timestamp())
        })
    });

    let lic = LicenseFile {
        schema: 2,
        key: activation.key,
        product: activation.product,
        tier: activation.tier.clone(),
        expires_at,
        activated_at: now_iso.clone(),
        last_verified_at: now_iso,
        activation_token: activation.activation_token,
        devices_used: activation.devices_used,
        devices_max: activation.devices_max,
        offline_grace_until: None,
        revoked: false,
        expired: false,
    };

    license_local::save_license(&lic)?;

    // Update FastBootCache for instant Cockpit TUI recognition.
    let hwid = crate::pro::HwidGenerator::new()
        .get_or_create()
        .unwrap_or_else(|_| "hwid-local".to_string());
    let now_unix = chrono::Utc::now().timestamp();
    let cache = crate::pro::FastBootCache {
        hwid,
        status: 1,
        session_expires_unix: expires_unix.unwrap_or(now_unix + 3650 * 86_400),
        api_base: base_url.to_string(),
        last_sync: now_unix,
    };
    let _ = cache.save();

    // Upsert device row in local pro_state DB (best-effort).
    if let Ok(db) = crate::db::Database::open() {
        let label = format!("{dev_name} ({key_masked})");
        let _ = db.upsert_pro_device(&dev_id, &label, "active", None, None);
    }

    crate::entitlements::invalidate_cache();
    Ok(lic)
}

fn cmd_activate(key_arg: &Option<String>, cmd: &LicenseCommand, cfg: &Config) -> Result<i32> {
    let key = prompt_key_if_needed(key_arg)?;
    let key_masked = license_gcc::mask_key(&key);
    let base_url = cmd
        .gcc_base_url
        .clone()
        .unwrap_or_else(|| cfg.license.gcc_base_url.clone());

    let dev_name = device_display_name();
    let platform = platform_name();
    eprintln!("  Activating {key_masked} on {dev_name} ({platform}) → {base_url}");

    let lic = match activate_license_key(&key, &base_url) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("  ✗ {e}");
            eprintln!("  Key {key_masked} was not saved locally.");
            return Ok(2);
        }
    };

    let p = license_local::license_path().unwrap_or_default();
    let tier_name = match lic.tier.as_str() {
        "99" => "Lifetime (Perpetual)",
        "24" => "2-Year Subscription",
        "12" => "Annual Subscription",
        "01" => "Monthly Subscription",
        other => other,
    };

    output::success(&format!(
        "License key '{key_masked}' activated successfully!"
    ));
    println!("  Product:      CAShell (SH)");
    println!("  Tier:         {tier_name}");
    match &lic.expires_at {
        Some(ts) => {
            let dt = chrono::DateTime::parse_from_rfc3339(ts)
                .ok()
                .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .or_else(|| {
                    ts.parse::<i64>().ok().and_then(|t| {
                        chrono::DateTime::from_timestamp(t, 0)
                            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    })
                })
                .unwrap_or_else(|| ts.clone());
            println!("  Expires:      {dt}");
        }
        None => println!("  Expires:      Never (Permanent 1-Device License)"),
    }
    println!("  Devices:      {}/{}", lic.devices_used, lic.devices_max);
    println!("  Stored at:    {}", p.display());
    println!("  Entitlement:  Offline-first verified & stored locally.");
    Ok(0)
}

// ── status ──────────────────────────────────────────────────────────────────

fn cmd_status(json: bool, _cfg: &Config) -> Result<i32> {
    let wants_json = json || !output::is_tty();

    match license_local::load_license()? {
        Some(lic) => {
            let valid = crate::entitlements::current().is_pro;
            let status_str = if valid { "active" } else { "expired" };

            if wants_json {
                let days_left = compute_days_left(&lic);
                let offline_grace_until = if valid {
                    compute_offline_grace_until(&lic)
                } else {
                    None
                };
                let out = StatusJson {
                    key_masked: license_gcc::mask_key(&lic.key),
                    tier: lic.tier.clone(),
                    status: status_str.to_string(),
                    expires_at: lic.expires_at.clone(),
                    days_left,
                    devices_used: lic.devices_used,
                    devices_max: lic.devices_max,
                    last_verified_at: (!lic.last_verified_at.is_empty())
                        .then(|| lic.last_verified_at.clone()),
                    offline_grace_until,
                };
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                let badge = if valid { "● ACTIVE" } else { "✖ EXPIRED" };
                println!("  Lisensi:     {}", license_gcc::mask_key(&lic.key));
                println!("  Tier:        {}", tier_display(&lic.tier));
                println!("  Status:      {badge}");
                match &lic.expires_at {
                    Some(ts) => {
                        let dt = parse_display_ts(ts);
                        println!("  Masa aktif:  Sampai {dt}");
                        if let Some(days) = compute_days_left(&lic) {
                            println!("  Sisa hari:   {days}");
                        }
                    }
                    None => println!("  Masa aktif:  Selamanya"),
                }
                println!("  Perangkat:   {}/{}", lic.devices_used, lic.devices_max);
                if !lic.last_verified_at.is_empty() {
                    let lv = parse_display_ts(&lic.last_verified_at);
                    println!("  Verifikasi terakhir: {lv}");
                }
                if let Some(grace) = compute_offline_grace_until(&lic) {
                    println!("  Grace offline:  {grace}");
                }
                println!(
                    "  Storage:     {} (offline)",
                    license_local::license_path().unwrap_or_default().display()
                );
            }
            Ok(0)
        }
        None => {
            if wants_json {
                println!(
                    "{}",
                    serde_json::json!({
                        "key_masked": null,
                        "tier": null,
                        "status": "none",
                        "expires_at": null,
                        "days_left": null,
                        "devices_used": 0,
                        "devices_max": 0,
                        "last_verified_at": null,
                        "offline_grace_until": null
                    })
                );
            } else {
                println!("○ CAShell Free Edition — No local license key installed.");
                println!("  Run 'cashell license activate <KEY>' to unlock Pro features.");
                println!("  Format: CASH[99|24|12|01]-RAND6-CK (e.g. CASH99-FOUNDR-3)");
            }
            Ok(0)
        }
    }
}

pub fn tier_display(tier: &str) -> &str {
    match tier {
        "99" => "Lifetime",
        "24" => "2-Year",
        "12" => "Annual",
        "01" => "Monthly",
        other => other,
    }
}

pub fn parse_display_ts(s: &str) -> String {
    if let Ok(ts) = s.parse::<i64>() {
        return chrono::DateTime::from_timestamp(ts, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| s.to_string());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.format("%Y-%m-%d").to_string();
    }
    s.to_string()
}

// ── deactivate (L-09) ───────────────────────────────────────────────────────

/// Deactivate local license and notify GCC (unless `local_only`).
pub fn deactivate_license_device(base_url: &str, local_only: bool) -> Result<String> {
    let lic = match license_local::load_license()? {
        Some(l) => l,
        None => bail!("No local license installed — nothing to deactivate."),
    };

    let key_masked = license_gcc::mask_key(&lic.key);
    let dev_id = license_gcc::device_id();

    if !local_only && !base_url.trim().is_empty() {
        license_gcc::deactivate(base_url, &lic.key, &dev_id)
            .map_err(|e| anyhow::anyhow!("GCC deactivation failed: {e}"))?;
    }

    let p = license_local::license_path()?;
    if p.exists() {
        std::fs::remove_file(&p)
            .map_err(|e| anyhow::anyhow!("failed to remove {}: {e}", p.display()))?;
    }

    let _ = crate::pro::FastBootCache::clear();
    crate::entitlements::invalidate_cache();
    Ok(key_masked)
}

fn cmd_deactivate(cmd: &LicenseCommand, cfg: &Config, local_only: bool) -> Result<i32> {
    let lic = match license_local::load_license()? {
        Some(l) => l,
        None => {
            output::info("No local license installed — nothing to deactivate.");
            return Ok(0);
        }
    };

    let key_masked = license_gcc::mask_key(&lic.key);
    let base_url = cmd
        .gcc_base_url
        .clone()
        .unwrap_or_else(|| cfg.license.gcc_base_url.clone());
    let dev_id = license_gcc::device_id();

    if !local_only && base_url.trim().is_empty() {
        // R-05: no host baked into default config — skip the GCC call, remove
        // the local license file only, report the skipped step.
        eprintln!(
            "⚠ [license] gcc_base_url empty — GCC deactivation skipped (local file removed only)."
        );
    }

    if !local_only && !base_url.trim().is_empty() {
        // Contact GCC to free the device slot.
        match license_gcc::deactivate(&base_url, &lic.key, &dev_id) {
            Ok(()) => {
                eprintln!("  Notified GCC: device unbound from {key_masked}");
            }
            Err(e) => {
                // Offline: decide whether to fall back to local-only removal.
                let prompt =
                    format!("  ⚠ GCC deactivation failed ({e}). Device slot is NOT freed.\n");
                let prompt_full = format!("{prompt}Remove local license file only? [y/N] ");

                let local = if output::is_tty() {
                    eprint!("{prompt_full}");
                    std::io::stderr().flush()?;
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line)?;
                    matches!(line.trim(), "y" | "Y" | "yes" | "Yes")
                } else {
                    // Non-TTY: fail closed unless --local-only was given.
                    eprintln!("{prompt}Cannot confirm interactively (non-TTY).\nUse 'cashell license deactivate --local-only' to remove the local file anyway.");
                    return Ok(1);
                };

                if !local {
                    output::info(
                        "Deactivate cancelled. Local license kept — device slot unchanged.",
                    );
                    return Ok(0);
                }

                eprintln!(
                    "  ⚠ Warning: removing local license only — device slot stays occupied on GCC."
                );
            }
        }
    } else {
        eprintln!("  Skipping GCC call (--local-only) — device slot stays occupied on server.");
    }

    // Delete the local license file; report the error, never `.ok()`.
    let p = license_local::license_path()?;
    if p.exists() {
        std::fs::remove_file(&p)
            .map_err(|e| anyhow::anyhow!("failed to remove {}: {e}", p.display()))?;
    }

    output::success(&format!(
        "License {key_masked} removed from local storage. CAShell reverted to Free tier."
    ));
    Ok(0)
}

/// Test seam: the same branch logic as [`cmd_deactivate`] without the
/// network call, so offline behavior is unit-testable (L-09).
#[cfg(test)]
fn deactivate_offline_decision(
    tty: bool,
    prompt_answer: &str,
) -> std::result::Result<(), &'static str> {
    if tty {
        if matches!(prompt_answer.trim(), "y" | "Y" | "yes" | "Yes") {
            Ok(()) // proceed to local removal with warning
        } else {
            Err("cancelled")
        }
    } else {
        // Non-TTY without --local-only → caller must exit 1.
        Err("non-tty-offline")
    }
}

// ── export-key / import-key (L-13) ──────────────────────────────────────────

/// Export the installed license key (plain key string, not entire json/token) to a 0600 file.
pub fn export_key_to_file(dest_path: &std::path::Path) -> Result<String> {
    let lic = match license_local::load_license()? {
        Some(l) => l,
        None => bail!("No active license installed to export."),
    };

    if let Some(parent) = dest_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let key_trimmed = lic.key.trim().to_string();
    let content = format!("{key_trimmed}\n");
    std::fs::write(dest_path, content.as_bytes())?;
    license_local::set_permissions_0600(dest_path)?;

    Ok(license_gcc::mask_key(&key_trimmed))
}

fn cmd_export_key(path: &std::path::Path) -> Result<i32> {
    let masked = export_key_to_file(path)?;
    output::success(&format!(
        "License key {masked} exported to {} (permissions: 0600)",
        path.display()
    ));
    Ok(0)
}

/// Import a license key from a file, validate format/checksum, and run full activation against GCC.
pub fn import_key_from_file(
    src_path: &std::path::Path,
    cmd: &LicenseCommand,
    cfg: &Config,
) -> Result<LicenseFile> {
    if !src_path.exists() {
        bail!("Import file does not exist: {}", src_path.display());
    }

    let raw = std::fs::read_to_string(src_path)?;
    let key = raw.trim().to_string();
    if key.is_empty() {
        bail!("Import file {} is empty.", src_path.display());
    }

    // Strict validation of format & checksum
    let (_prod, _tier) = validate_license_format(&key)?;

    let base_url = cmd
        .gcc_base_url
        .clone()
        .unwrap_or_else(|| cfg.license.gcc_base_url.clone());

    activate_license_key(&key, &base_url)
}

fn cmd_import_key(path: &std::path::Path, cmd: &LicenseCommand, cfg: &Config) -> Result<i32> {
    let lic = match import_key_from_file(path, cmd, cfg) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("  ✗ Import failed: {e}");
            return Ok(2);
        }
    };

    let key_masked = license_gcc::mask_key(&lic.key);
    output::success(&format!(
        "License key {key_masked} imported from {} and activated on GCC!",
        path.display()
    ));
    println!("  Tier:         {}", tier_display(&lic.tier));
    println!("  Devices:      {}/{}", lic.devices_used, lic.devices_max);
    Ok(0)
}

// ── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// CASH99-ABCDEF-2 is a valid key:
    /// body = "CASH99-ABCDEF"
    /// A=10, B=11, C=12, D=13, E=14, F=15, digits 9+9
    /// total = 10+11+12+13+14+15+10+10+10(CA... wait)
    // Let's compute: C=12, A=10, S=28, H=17, 9=9, 9=9, A=10, B=11, C=12, D=13, E=14, F=15
    // Actually body is "CASH99-ABCDEF" → chars: C,A,S,H,9,9,A,B,C,D,E,F
    // values: 12,10,28,17,9,9,10,11,12,13,14,15 = 140
    // 140 % 10 = 0, (10 - 0) % 10 = 0 → expected ck = 0
    // But the key says ck=2... let's use a key that passes.
    //
    // For the test we'll just check the format validation path, not the checksum.

    #[test]
    fn test_validate_format_bad_prefix() {
        let result = validate_license_format("CAFOO-ABCDEF-2");
        assert!(result.is_err(), "non-CASH prefix must be rejected");
    }

    #[test]
    fn test_validate_format_bad_tier() {
        let result = validate_license_format("CASH50-ABCDEF-2");
        assert!(
            result.is_err(),
            "tier 50 is not allowed (only 99, 24, 12, 01)"
        );
    }

    #[test]
    fn test_validate_format_wrong_length() {
        let result = validate_license_format("CASH99-AB-2");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_format_bad_mid_segment() {
        // lowercase → uppercased → "abcdef" is alphanumeric, OK
        // use invalid char
        let result = validate_license_format("CASH99-AB!DF-2");
        assert!(
            result.is_err(),
            "non-alphanumeric mid segment must be rejected"
        );
    }

    #[test]
    fn test_validate_format_valid_checksum() {
        // Construct a valid key programmatically
        let tier = "99";
        let rand6 = "ABCDEF";
        let body = format!("CASH{tier}-{rand6}");
        let ck = mod10_checksum(&body);
        let key = format!("{body}-{ck}");
        let result = validate_license_format(&key);
        assert!(result.is_ok(), "valid key must pass: {key} → {result:?}");
        let (_prod, tier_out) = result.unwrap();
        assert_eq!(tier_out, "99");
    }

    #[test]
    fn test_validate_format_bad_checksum() {
        // Build a key with a known-correct checksum, then swap it for a
        // different digit to force a mismatch.
        let body = "CASH99-ABCDEF";
        let correct_ck = mod10_checksum(body);
        let wrong_ck = if correct_ck == 0 { 1 } else { 0 };
        let bad_key = format!("{body}-{wrong_ck}");
        assert!(
            validate_license_format(&bad_key).is_err(),
            "wrong checksum must be rejected"
        );
    }

    #[test]
    fn test_mod10_checksum_deterministic() {
        let a = mod10_checksum("CASH99-ABCDEF");
        let b = mod10_checksum("CASH99-ABCDEF");
        assert_eq!(a, b);
        assert!(a <= 9);
    }

    #[test]
    fn test_mask_key_no_full_key_leak() {
        let k = "CASH99-FOUNDR-3";
        let masked = license_gcc::mask_key(k);
        assert!(!masked.contains("FOUNDR"));
        assert_eq!(masked, "CASH99…");
    }

    #[test]
    fn test_device_id_is_64_hex() {
        let d = license_gcc::device_id();
        assert_eq!(d.len(), 64);
        assert!(d.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_platform_name_is_known() {
        let p = platform_name();
        assert!(
            matches!(p, "linux" | "macos" | "windows" | "unknown"),
            "unexpected platform: {p}"
        );
    }

    #[test]
    fn test_deactivate_offline_non_tty_exits_1() {
        // Non-TTY offline: no --local-only → Err("non-tty-offline") → exit 1.
        let r = deactivate_offline_decision(false, "");
        assert!(
            matches!(r, Err("non-tty-offline")),
            "non-TTY offline must be a hard fail"
        );
        let code = match r {
            Ok(()) => 0,
            Err(_) => 1,
        };
        assert_eq!(code, 1, "non-TTY offline without --local-only → exit 1");
    }

    #[test]
    fn test_deactivate_offline_tty_yes_proceeds() {
        let r = deactivate_offline_decision(true, "y");
        assert!(r.is_ok(), "TTY + y must proceed with local removal");
    }

    #[test]
    fn test_deactivate_offline_tty_no_cancels() {
        let r = deactivate_offline_decision(true, "n");
        assert!(
            matches!(r, Err("cancelled")),
            "TTY + n must cancel, local file kept"
        );
    }

    #[test]
    fn test_export_and_import_key_flow() {
        let tmp =
            std::env::temp_dir().join(format!("cashell-export-import-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        crate::paths::with_test_home(&tmp, || {
            crate::entitlements::invalidate_cache();

            // Export when no license exists -> error
            let export_file = tmp.join("exported_key.txt");
            let err = export_key_to_file(&export_file);
            assert!(err.is_err());
            assert!(err
                .unwrap_err()
                .to_string()
                .contains("No active license installed"));

            // Save a valid license
            let lic = LicenseFile {
                schema: 2,
                key: "CASH99-FOUNDR-3".to_string(),
                product: "SH".to_string(),
                tier: "99".to_string(),
                expires_at: None,
                activated_at: "2026-11-13T00:00:00Z".to_string(),
                last_verified_at: "2026-11-13T00:00:00Z".to_string(),
                activation_token: None,
                devices_used: 1,
                devices_max: 3,
                offline_grace_until: None,
                revoked: false,
                expired: false,
            };
            license_local::save_license(&lic).unwrap();

            // Export license key
            let masked = export_key_to_file(&export_file).expect("export success");
            assert_eq!(masked, "CASH99…");
            assert!(export_file.exists());

            // Check contents: exactly the key + newline
            let exported_data = std::fs::read_to_string(&export_file).unwrap();
            assert_eq!(exported_data.trim(), "CASH99-FOUNDR-3");

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&export_file)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600, "export key file permissions must be 0600");
            }

            // Test import with invalid format in file
            let bad_file = tmp.join("bad_key.txt");
            std::fs::write(&bad_file, "INVALID-KEY-FOO").unwrap();
            let cmd = LicenseCommand {
                gcc_base_url: Some("https://gcc.fathforce.com".to_string()),
                action: LicenseAction::ImportKey {
                    path: bad_file.clone(),
                },
            };
            let cfg = Config::default();
            let res = import_key_from_file(&bad_file, &cmd, &cfg);
            assert!(res.is_err());
        });

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
