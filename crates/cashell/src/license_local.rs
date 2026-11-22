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
    /// Offline grace: when the last GCC verification failed due to a network
    /// error, the license stays usable until `last_verified_at + 30 days`,
    /// capped at `expires_at` when one exists. ISO-8601 (L-08).
    #[serde(default)]
    pub offline_grace_until: Option<String>,
    /// Set when GCC reports the key as revoked: the device reverts to Free
    /// (Pro features off) but the license file is kept, not deleted (L-08).
    #[serde(default)]
    pub revoked: bool,
    /// Set when the local or GCC status reports the subscription expired (L-08).
    #[serde(default)]
    pub expired: bool,
    /// GCC License v2 Ed25519 token (the only thing that grants Pro). `None` = Free.
    #[serde(default)]
    pub signed_token: Option<crate::license_token::SignedToken>,
    /// Latest wall clock this licence was checked at (clock-rollback ratchet).
    #[serde(default)]
    pub last_seen_unix: i64,
}

fn default_schema_v2() -> u32 {
    2
}

/// Return the default license file path: `~/.local/share/cashell/license.json`.
/// Resolved through [`crate::paths`] so the `CASHELL_HOME` test override applies.
pub fn license_path() -> Result<PathBuf> {
    if dirs::home_dir().is_none() {
        return Err(anyhow!("no home directory"));
    }
    Ok(crate::paths::license_path())
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

    // Entitlement is decided by `is_license_valid` (Ed25519 signed_token), not at load time.
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

/// Returns true when this device is entitled to Pro right now.
///
/// GCC License v2: only a valid Ed25519 `signed_token` for this product and device grants Pro.
/// `tier`, `expires_at` and `product` in the file are display-only and editable by the user.
/// `revoked` / `expired` flags (set by periodic verify) can only take Pro away.
pub fn is_license_valid(lic: &LicenseFile) -> bool {
    if lic.revoked || lic.expired {
        return false;
    }
    crate::license_token::verify(
        lic.signed_token.as_ref(),
        crate::license_token::public_key,
        &crate::license_gcc::device_id(),
        lic.last_seen_unix,
        chrono::Utc::now().timestamp(),
    )
    .is_ok()
}

// ── periodic verification (L-08) ───────────────────────────────────────────

/// Days before `last_verified_at` forces a fresh GCC verification.
pub const PERIODIC_VERIFY_DAYS: i64 = 7;

/// Days of offline grace after a failed (network) verification.
pub const OFFLINE_GRACE_DAYS: i64 = 14;

pub fn parse_ts_unix(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    if let Ok(ts) = s.parse::<i64>() {
        return Some(ts);
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// True when `last_verified_at` is more than [`PERIODIC_VERIFY_DAYS`] days
/// old, or when it is missing/empty. The daemon and cockpit call this at
/// startup to decide whether to trigger [`verify_license`].
pub fn should_verify_periodically(lic: &LicenseFile) -> bool {
    let Some(last_ts) = parse_ts_unix(&lic.last_verified_at) else {
        return true;
    };
    let now = chrono::Utc::now().timestamp();
    now - last_ts > PERIODIC_VERIFY_DAYS * 86_400
}

/// Verify the license against GCC and persist the result (GCC License v2).
///
/// - Valid on this device: store the fresh `signed_token` (new 14-day window).
/// - Not valid / device removed: mark revoked or expired and drop the token (Free now).
/// - [`crate::license_gcc::LicenseError::Network`] (offline): leave the file as is; the stored
///   token keeps working until its own `exp`. No error returned.
pub fn verify_license(
    license: &LicenseFile,
    base_url: &str,
) -> std::result::Result<(), crate::license_gcc::LicenseError> {
    use crate::license_gcc::{self, LicenseError};

    let persist = |lic: &LicenseFile| {
        save_license(lic)
            .map(|_| ())
            .map_err(|_| LicenseError::InvalidResponse("failed to persist license".into()))
    };
    let dev_id = license_gcc::device_id();
    let now = chrono::Utc::now();
    let mut lic = license.clone();
    lic.last_seen_unix = lic.last_seen_unix.max(now.timestamp());

    match license_gcc::verify(base_url, &license.key, &dev_id) {
        Ok(resp) if resp.valid && resp.device_registered => {
            lic.last_verified_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            lic.offline_grace_until = None;
            lic.revoked = false;
            lic.expired = false;
            if let Some(exp) = resp.expires_at {
                lic.expires_at = Some(exp);
            }
            if resp.signed_token.is_some() {
                lic.signed_token = resp.signed_token;
            }
            persist(&lic)
        }
        Ok(resp) => {
            lic.last_verified_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            if resp.status == "expired" {
                lic.expired = true;
            } else {
                lic.revoked = true;
            }
            lic.signed_token = None;
            persist(&lic)
        }
        Err(LicenseError::Network(_)) => persist(&lic),
        Err(LicenseError::Revoked) | Err(LicenseError::NotFound) => {
            lic.revoked = true;
            lic.signed_token = None;
            persist(&lic)
        }
        Err(LicenseError::Expired) => {
            lic.expired = true;
            lic.signed_token = None;
            persist(&lic)
        }
        Err(e) => Err(e),
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// Set 0600 permissions on a file (Unix only; no-op elsewhere).
#[cfg(unix)]
pub fn set_permissions_0600(p: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(p)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(p, perms)
}

#[cfg(not(unix))]
pub fn set_permissions_0600(_p: &std::path::Path) -> std::io::Result<()> {
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
            offline_grace_until: None,
            revoked: false,
            expired: false,
            signed_token: None,
            last_seen_unix: 0,
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

    /// Shared guard so tests writing license.json run sequentially.
    static LICENSE_FILE_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn license_file_guard() -> std::sync::MutexGuard<'static, ()> {
        LICENSE_FILE_MUTEX.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Sequential license file round-trip tests.
    /// All tests that write to the shared license.json path must run
    /// sequentially to avoid concurrent file races.
    #[test]
    fn test_license_file_lifecycle() {
        let _guard = license_file_guard();
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

        // 6. Tampered file (no valid signed token) still loads, but never grants Pro.
        let tampered = LicenseFile {
            activation_token: Some("fake-tampered-token".to_string()),
            ..sample_license()
        };
        let data = serde_json::to_string_pretty(&tampered).unwrap();
        std::fs::write(&p, data).expect("write tampered file");
        let result = load_license()
            .expect("load should not error")
            .expect("file loads");
        assert!(!is_license_valid(&result), "tampered file must not be Pro");

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

    /// GCC License v2 regression: tier "99" / lifetime in the JSON file means nothing without
    /// a valid signed token (this used to grant permanent Pro to anyone editing license.json).
    #[test]
    fn test_tier_99_without_signed_token_is_not_pro() {
        let mut lic = sample_license();
        lic.tier = "99".to_string();
        lic.expires_at = None;
        assert!(
            !is_license_valid(&lic),
            "unsigned lifetime file must not be Pro"
        );
    }

    #[test]
    fn test_signed_token_grants_pro_until_exp() {
        use crate::license_token::test_support;
        let mut lic = sample_license();
        lic.signed_token = Some(test_support::valid_token());
        assert!(is_license_valid(&lic), "valid signed token must be Pro");

        let dev = crate::license_gcc::device_id();
        lic.signed_token = Some(test_support::token(
            &["CASHELL"],
            &dev,
            chrono::Utc::now().timestamp() - 1,
        ));
        assert!(!is_license_valid(&lic), "token past exp must not be Pro");

        lic.signed_token = Some(test_support::token(
            &["CATERM"],
            &dev,
            chrono::Utc::now().timestamp() + 3600,
        ));
        assert!(
            !is_license_valid(&lic),
            "CATerm token must not unlock CASHell"
        );
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

    // ── L-08 periodic verify clock scenarios ─────────────────────────────────

    /// 1. Fresh: `last_verified_at` < 7 days → no periodic verify needed.
    #[test]
    fn test_should_verify_periodically_fresh() {
        let mut lic = sample_license();
        lic.last_verified_at =
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert!(
            !should_verify_periodically(&lic),
            "fresh license must not verify"
        );

        // 6 days ago: still fresh.
        let six_days = chrono::Utc::now() - chrono::Duration::days(6);
        lic.last_verified_at = six_days.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert!(!should_verify_periodically(&lic), "6d old must not verify");
    }

    /// 2. Stale: `last_verified_at` > 7 days → verify required.
    #[test]
    fn test_should_verify_periodically_stale() {
        let mut lic = sample_license();
        let eight_days = chrono::Utc::now() - chrono::Duration::days(8);
        lic.last_verified_at = eight_days.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        assert!(should_verify_periodically(&lic), "8d old must verify");

        // Empty timestamp → verify.
        lic.last_verified_at = String::new();
        assert!(
            should_verify_periodically(&lic),
            "missing timestamp must verify"
        );
    }

    /// Case 4, revoked: `is_license_valid` must return false, and a revoked
    /// license must not count as Pro (tier stays but flag forces Free).
    #[test]
    fn test_revoked_license_invalid_and_not_pro() {
        let mut lic = sample_license();
        lic.revoked = true;
        assert!(
            !is_license_valid(&lic),
            "revoked license must be invalid even for tier 99"
        );

        // Expired flag also forces invalid.
        lic = sample_license();
        lic.expired = true;
        assert!(!is_license_valid(&lic), "expired flag must invalidate");

        // Grace window can't rescue a revoked license: validity check runs
        // before any grace logic, and revocation is sticky on disk.
    }

    /// Offline (GCC unreachable): `verify_license` returns Ok and keeps the stored token, so the
    /// licence stays Pro until the token's own 14-day `exp`.
    #[test]
    fn test_verify_license_offline_keeps_token() {
        let _guard = license_file_guard();
        let p = license_path().unwrap();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let mut lic = sample_license();
        lic.signed_token = Some(crate::license_token::test_support::valid_token());
        save_license(&lic).unwrap();

        // Unroutable port → guaranteed network error (no mock server needed).
        let res = verify_license(&lic, "http://127.0.0.1:1");
        assert!(res.is_ok(), "offline verify must not error, got {res:?}");

        let updated = load_license().unwrap().unwrap();
        assert_eq!(
            updated.signed_token, lic.signed_token,
            "offline must keep the token"
        );
        assert!(
            is_license_valid(&updated),
            "within token exp the licence stays Pro"
        );

        let _ = remove_license();
    }
}
