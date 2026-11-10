//! `cashell license` command — offline-first license activation, validation, and status.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use crate::license_local::{is_license_valid, load_license, remove_license, save_license, LocalLicense};
use crate::output;

#[derive(Parser, Debug, Clone)]
#[command(name = "license", about = "Manage CAShell offline-first license key & billing")]
pub struct LicenseCommand {
    #[command(subcommand)]
    pub action: LicenseAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LicenseAction {
    /// Activate a license key (e.g. CASH99-FOUNDR-3)
    Activate {
        /// The 15-character license key (format: CA[PROD][TIER]-[RAND6]-[CK])
        key: String,
    },
    /// Display current local license status, tier, and expiration
    Status,
    /// Deactivate and remove the stored local license key
    Deactivate,
}

/// Calculate the standard CA mod-10 checksum
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

/// Validate CA license format: CA[PROD][TIER]-[RAND6]-[CK]
pub fn validate_license_format(key: &str) -> Result<(String, String)> {
    let clean = key.trim().to_ascii_uppercase();
    let parts: Vec<&str> = clean.split('-').collect();
    if parts.len() != 3 {
        bail!("Invalid license format. Expected: CA[PROD][TIER]-[RAND6]-[CK] (e.g. CASH99-FOUNDR-3)");
    }
    let [prefix, rand6, ck_str] = parts.as_slice() else {
        bail!("Invalid license structure");
    };

    if prefix.len() != 6 || rand6.len() != 6 || ck_str.len() != 1 {
        bail!("Invalid segment length in license key. Must be 15 characters total.");
    }
    if !prefix.starts_with("CA") {
        bail!("Invalid prefix: license key must begin with 'CA'");
    }

    let prod = prefix[2..4].to_string();
    let tier = prefix[4..6].to_string();

    if prod != "SH" && prod != "AA" {
        bail!("Invalid product code '{prod}'. Expected 'SH' (CAShell) or 'AA' (All-Access)");
    }

    let body = format!("{prefix}-{rand6}");
    let expected_ck = mod10_checksum(&body);
    let given_ck = ck_str.parse::<u32>().map_err(|_| anyhow::anyhow!("Invalid checksum digit"))?;

    if expected_ck != given_ck {
        bail!("Checksum mismatch: key may be mistyped (expected {expected_ck}, got {given_ck})");
    }

    Ok((prod, tier))
}

pub fn run(cmd: &LicenseCommand) -> Result<i32> {
    match &cmd.action {
        LicenseAction::Activate { key } => cmd_activate(key),
        LicenseAction::Status => cmd_status(),
        LicenseAction::Deactivate => cmd_deactivate(),
    }
}

fn cmd_activate(key: &str) -> Result<i32> {
    let clean_key = key.trim().to_ascii_uppercase();
    let (prod, tier) = validate_license_format(&clean_key)?;

    let expires_at = match tier.as_str() {
        "99" => 0, // Lifetime
        "12" => {
            let now = chrono::Utc::now().timestamp();
            now + (365 * 86400)
        }
        "01" => {
            let now = chrono::Utc::now().timestamp();
            now + (30 * 86400)
        }
        _ => {
            let now = chrono::Utc::now().timestamp();
            now + (365 * 86400)
        }
    };

    let lic = LocalLicense {
        key: clean_key.clone(),
        product: prod.clone(),
        tier: tier.clone(),
        expires_at,
    };

    save_license("cashell", &lic)?;

    // Also update FastBootCache for instant Cockpit TUI recognition
    let hwid = crate::pro::HwidGenerator::new()
        .get_or_create()
        .unwrap_or_else(|_| "hwid-local".to_string());
    let now = chrono::Utc::now().timestamp();
    let cache = crate::pro::FastBootCache {
        hwid,
        status: 1,
        session_expires_unix: if expires_at == 0 { now + (3650 * 86400) } else { expires_at },
        api_base: "https://gcc.fathforce.com".to_string(),
        last_sync: now,
    };
    let _ = cache.save();

    output::success(&format!("✔ License key '{clean_key}' activated successfully!"));
    let tier_name = if tier == "99" {
        "Founder Lifetime (Perpetual)"
    } else {
        "Annual Subscription"
    };
    println!("  Product:      CAShell ({prod})");
    println!("  Tier:         {tier_name}");
    if expires_at == 0 {
        println!("  Expires:      Never (Permanent 1-Device License)");
    } else {
        let dt = chrono::DateTime::from_timestamp(expires_at, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_default();
        println!("  Expires:      {dt}");
    }
    println!("  Entitlement:  Offline-first verified & stored locally.");
    Ok(0)
}

fn cmd_status() -> Result<i32> {
    match load_license("cashell")? {
        Some(lic) => {
            let valid = is_license_valid(&lic);
            let status_badge = if valid { "● ACTIVE" } else { "✖ EXPIRED" };

            println!("┌────────────────────────────────────────────────────────────┐");
            println!("│ CAShell License & Billing Overview                         │");
            println!("└────────────────────────────────────────────────────────────┘");
            println!("  Status:       {status_badge}");
            println!("  Key:          {}", lic.key);
            println!("  Product:      {}", if lic.product == "SH" { "CAShell" } else { "CA All-Access" });
            println!("  Tier:         {}", if lic.tier == "99" { "Lifetime (Tier 99)" } else { "Subscription" });
            if lic.expires_at == 0 {
                println!("  Expiration:   Never Expires · Single Device Lifetime");
            } else {
                let dt = chrono::DateTime::from_timestamp(lic.expires_at, 0)
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_default();
                println!("  Expiration:   {dt}");
            }
            println!("  Storage:      ~/.local/share/cashell/license.json (Offline)");
            Ok(0)
        }
        None => {
            println!("○ CAShell Free Edition — No local license key installed.");
            println!("  Run 'cashell license activate <KEY>' to unlock Pro features.");
            println!("  Format: CASH99-RAND6-CK (e.g. CASH99-FOUNDR-3)");
            Ok(0)
        }
    }
}

fn cmd_deactivate() -> Result<i32> {
    remove_license("cashell")?;
    output::info("License removed from local storage. CAShell reverted to Free tier.");
    Ok(0)
}
