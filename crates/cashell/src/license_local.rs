//! Local license storage — offline-first entitlement.
//!
//! Schema v2: after activation the GCC server returns an `expires_at`
//! (nullable = lifetime) plus an optional HMAC-SHA256 `activation_token`.
//! The file is written atomically (temp + rename) with 0600 permissions so
//! the full license key stays on local disk only, never in logs or config.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// License file on disk. `schema` discriminates v1 (legacy) vs v2 (current).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LicenseFile {
    /// Schema version: 2 = current. Values < 2 are treated as legacy v1.
    #[serde(default = "default_schema_v2")]
    pub schema: u32,
    /// Full license key (never logged; use [`crate::license_gcc::mask_key`]).
    pub key: String,
    /// Product code, e.g. "SH" (CAShell) or "AA" (All-Access).
    #[serde(default)]
    pub product: String,
    /// Tier: "99" = lifetime, "24" = 2-year, "12" = 1-year, "01" = 1-month.
    pub tier: String,
    /// ISO-8601 or unix-timestamp string; `None` / `0` = lifetime.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// When this key was activated on this device (ISO-8601).
    #[serde(default)]
    pub activated_at: String,
    /// When this key was last verified against GCC (ISO-8601).
    #[serde(default)]
    pub last_verified_at: String,
    /// HMAC-SHA256 activation token returned by GCC (V-04 L-03).
    /// `None` = GCC has not yet started returning tokens.
    #[serde(default)]
    pub activation_token: Option<String>,
    #[serde(default)]
    pub devices_used: u32,
    #[serde(default)]
    pub devices_max: u32,
}

fn default_schema_v2() -> u32 {
    2
}

/// Return the default license file path: `~/.local/share/cashell/license.json`.
pub fn license_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?;
    Ok(home.join(".local/share/cashell/license.json"))
}

/// Persist a license atomically (temp file + rename) with 0600 permissions.
pub fn save_license(license: &LicenseFile) -> Result<PathBuf> {
    let p = license_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(license)?;
    // Atomic write: temp file in same dir, then rename.
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, &data)?;
    set_permissions_0600(&tmp)?;
    std::fs::rename(&tmp, &p)?;
    // Ensure final file is 0600 even if rename carried temp perms on some FSes.
    set_permissions_0600(&p)?;
    Ok(p)
}

/// Load and verify a license from disk.
///
/// Returns `Ok(None)` when:
/// - no file exists
/// - the file is schema v1 (legacy) — prints "needs re-activation" message
/// - the file is schema v2 but the `activation_token` fails HMAC verification
/// - the file cannot be parsed
pub fn load_license() -> Result<Option<LicenseFile>> {
    let p = license_path()?;
    if !p.exists() {
        return Ok(None);
    }
    let txt = std::fs::read_to_string(&p)?;
    let lic: LicenseFile =
        serde_json::from_str(&txt).map_err(|e| anyhow!("license.json parse error: {e}"))?;

    if lic.schema < 2 {
        eprintln!(
            "warning: license.json is schema v{} (legacy) — please re-activate with `cashell license activate`",
            lic.schema
        );
        return Ok(None);
    }

    // Verify activation token when present (V-04 L-03).
    if let Some(token) = &lic.activation_token {
        let device = crate::license_gcc::device_id();
        if !crate::license_gcc::verify_activation_token(token, &lic.key, &device) {
            eprintln!(
                "warning: activation token HMAC-SHA256 verification failed for {} — treating as unlicensed; please re-activate",
                crate::license_gcc::mask_key(&lic.key)
            );
            return Ok(None);
        }
    }

    Ok(Some(lic))
}

/// Remove the local license file.
pub fn remove_license() -> Result<()> {
    let p = license_path()?;
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

/// Returns true when the license is valid right now.
/// Lifetime tier (99) or `expires_at = None` never expires;
/// others are checked against the current timestamp.
pub fn is_license_valid(lic: &LicenseFile) -> bool {
    if lic.tier == "99" {
        return true;
    }
    match &lic.expires_at {
        Some(ts_str) => {
            // Accept both ISO-8601 and unix timestamp strings.
            if let Ok(ts) = ts_str.parse::<i64>() {
                let now = chrono::Utc::now().timestamp();
                now <= ts
            } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                let now = chrono::Utc::now();
                now <= dt.with_timezone(&chrono::Utc)
            } else {
                // Unparseable → treat as valid (fail-open for offline use).
                true
            }
        }
        // None = lifetime
        None => true,
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// Set 0600 permissions on a file (Unix only; no-op elsewhere).
#[cfg(unix)]
fn set_permissions_0600(p: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(p)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(p, perms)
}

#[cfg(not(unix))]
fn set_permissions_0600(_p: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

// ── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid v2 license for testing.
    fn sample_license() -> LicenseFile {
        LicenseFile {
            schema: 2,
            key: "CASH99-ABCDEF-2".to_string(),
            product: "SH".to_string(),
            tier: "99".to_string(),
            expires_at: None,
            activated_at: "2026-11-12T00:00:00Z".to_string(),
            last_verified_at: "2026-11-12T00:00:00Z".to_string(),
            activation_token: None,
            devices_used: 1,
            devices_max: 1,
        }
    }

    #[test]
    fn test_license_path_is_under_local_share() {
        let p = license_path().unwrap();
        assert!(
            p.to_str()
                .unwrap()
                .contains(".local/share/cashell/license.json"),
            "unexpected path: {p:?}"
        );
    }

    /// Sequential license file round-trip tests.
    /// All tests that write to the shared license.json path must run
    /// sequentially to avoid concurrent file races.
    #[test]
    fn test_license_file_lifecycle() {
        let p = license_path().unwrap();
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // 1. No file → load returns None
        let _ = remove_license();
        let result = load_license().expect("load should not error");
        assert!(result.is_none(), "no file → None");

        // 2. Save + load round-trip
        let lic = sample_license();
        save_license(&lic).expect("save should succeed");
        assert!(p.exists(), "file must exist after save");

        // 3. Verify 0600 permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "license file must be 0600, got {mode:o}");
        }

        // 4. Load back and verify fields
        let loaded = load_license().expect("load should succeed");
        let loaded = loaded.expect("license should be present");
        assert_eq!(loaded.key, "CASH99-ABCDEF-2");
        assert_eq!(loaded.tier, "99");
        assert!(
            loaded.expires_at.is_none(),
            "lifetime tier → expires_at None"
        );

        // 5. Schema v1 → load returns None (needs re-activation)
        let v1 =
            r#"{"schema":1,"key":"CASH99-ABCDEF-2","product":"SH","tier":"99","expires_at":"0"}"#;
        std::fs::write(&p, v1).expect("write v1 file");
        let result = load_license().expect("load should not error");
        assert!(
            result.is_none(),
            "schema v1 must be treated as needs-re-activation (None)"
        );

        // 6. Tampered activation_token → load returns None
        let tampered = LicenseFile {
            activation_token: Some("fake-tampered-token".to_string()),
            ..sample_license()
        };
        let data = serde_json::to_string_pretty(&tampered).unwrap();
        std::fs::write(&p, data).expect("write tampered file");
        let result = load_license().expect("load should not error");
        assert!(
            result.is_none(),
            "tampered activation_token must cause load to return None"
        );

        // 7. Valid v2 without token → load succeeds
        let data = serde_json::to_string_pretty(&sample_license()).unwrap();
        std::fs::write(&p, data).expect("write v2 file");
        let loaded = load_license().expect("load should not error");
        assert!(
            loaded.is_some(),
            "v2 without activation_token should load successfully"
        );

        // Cleanup
        let _ = remove_license();
    }

    #[test]
    fn test_is_license_valid_lifetime() {
        let mut lic = sample_license();
        lic.tier = "99".to_string();
        lic.expires_at = None;
        assert!(is_license_valid(&lic));

        // Even with a past expires_at, tier 99 is always valid.
        lic.expires_at = Some("0".to_string());
        assert!(is_license_valid(&lic), "tier 99 must always be valid");
    }

    #[test]
    fn test_is_license_valid_expires_at() {
        let mut lic = sample_license();
        lic.tier = "12".to_string();

        // Future timestamp → valid.
        let future_ts = (chrono::Utc::now().timestamp() + 3600).to_string();
        lic.expires_at = Some(future_ts.clone());
        assert!(is_license_valid(&lic), "future expires_at must be valid");

        // Past timestamp → invalid.
        let past_ts = (chrono::Utc::now().timestamp() - 3600).to_string();
        lic.expires_at = Some(past_ts.clone());
        assert!(!is_license_valid(&lic), "past expires_at must be invalid");
    }

    #[test]
    fn test_mask_key_in_license_module() {
        let k = "CASH99-ABCDEF-2";
        assert_eq!(crate::license_gcc::mask_key(k), "CASH99…");
    }

    /// Verify no test helper prints the full key.
    #[test]
    fn test_full_key_never_in_masked_output() {
        let k = "CASH24-SECRET-7";
        let masked = crate::license_gcc::mask_key(k);
        assert!(
            !masked.contains("SECRET"),
            "masked output must not contain key tail"
        );
    }
}
