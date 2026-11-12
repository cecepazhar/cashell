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
///
/// `revoked` or `expired` flags (set by L-08 periodic verify) force
/// the license to be invalid even when the raw dates would say "valid".
pub fn is_license_valid(lic: &LicenseFile) -> bool {
    if lic.revoked || lic.expired {
        return false;
    }
    if lic.tier == "99" {
        return true;
    }
    match &lic.expires_at {
        Some(ts_str) => {
            // Accept both ISO-8601 and unix timestamp strings.
            let now = chrono::Utc::now();
            if let Ok(ts) = ts_str.parse::<i64>() {
                now.timestamp() <= ts
            } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
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

// ── periodic verification (L-08) ───────────────────────────────────────────

/// Days before `last_verified_at` forces a fresh GCC verification.
pub const PERIODIC_VERIFY_DAYS: i64 = 7;

/// Days of offline grace after a failed (network) verification.
pub const OFFLINE_GRACE_DAYS: i64 = 30;

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

/// Verify the license against GCC, update `last_verified_at`, and persist
/// the license file back to disk (L-08).
///
/// - Success: `last_verified_at` set to now, `offline_grace_until` cleared.
/// - [`crate::license_gcc::LicenseError::Network`] (offline): extend the
///   offline grace to `last_verified_at + 30 days`, capped at `expires_at`
///   when one exists. No error returned — offline use stays permitted.
/// - [`crate::license_gcc::LicenseError::Revoked`]: mark the license
///   revoked (Pro off), keep the file on disk.
/// - [`crate::license_gcc::LicenseError::Expired`]: mark the license
///   expired, keep the file on disk.
pub fn verify_license(
    license: &LicenseFile,
    base_url: &str,
) -> std::result::Result<(), crate::license_gcc::LicenseError> {
    use crate::license_gcc::{self, LicenseError};

    let dev_id = license_gcc::device_id();
    let now_iso = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    match license_gcc::verify(base_url, &license.key, &dev_id) {
        Ok(resp) => {
            let mut lic = license.clone();
            lic.last_verified_at = now_iso;
            lic.offline_grace_until = None;
            // GCC is authoritative for expiration when it reports one.
            if let Some(exp) = resp.expires_at {
                lic.expires_at = Some(exp);
            }
            save_license(&lic).map_err(|_| {
                crate::license_gcc::LicenseError::InvalidResponse(
                    "failed to persist license".into(),
                )
            })?;
            Ok(())
        }
        Err(LicenseError::Network(_)) => {
            // Offline grace: keep the license usable until
            // last_verified_at + 30 days, capped at expires_at.
            let mut lic = license.clone();
            let last_ts = parse_ts_unix(&license.last_verified_at)
                .unwrap_or_else(|| chrono::Utc::now().timestamp());
            let grace_ts = last_ts + OFFLINE_GRACE_DAYS * 86_400;
            let final_ts = match parse_ts_unix(license.expires_at.as_deref().unwrap_or("")) {
                Some(exp_ts) if exp_ts < grace_ts => exp_ts,
                _ => grace_ts,
            };
            if let Some(dt) = chrono::DateTime::from_timestamp(final_ts, 0) {
                lic.offline_grace_until =
                    Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            }
            save_license(&lic).map_err(|_| {
                crate::license_gcc::LicenseError::InvalidResponse(
                    "failed to persist license".into(),
                )
            })?;
            Ok(())
        }
        Err(LicenseError::Revoked) => {
            // Revert to Free: flag the license, do NOT delete the file.
            let mut lic = license.clone();
            lic.revoked = true;
            save_license(&lic).map_err(|_| {
                crate::license_gcc::LicenseError::InvalidResponse(
                    "failed to persist license".into(),
                )
            })?;
            Ok(())
        }
        Err(LicenseError::Expired) => {
            let mut lic = license.clone();
            lic.expired = true;
            save_license(&lic).map_err(|_| {
                crate::license_gcc::LicenseError::InvalidResponse(
                    "failed to persist license".into(),
                )
            })?;
            Ok(())
        }
        Err(e) => Err(e),
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
            offline_grace_until: None,
            revoked: false,
            expired: false,
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

    /// 3. Offline grace: grace window = last_verified_at + 30 days,
    /// capped at expires_at when it falls earlier.
    #[test]
    fn test_offline_grace_capped_at_expires_at() {
        let now = chrono::Utc::now();
        let mut lic = sample_license();
        lic.tier = "12".to_string();
        lic.last_verified_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        // (a) Expires 10 days from now → grace capped at 10 days, not 30.
        let exp10 = now + chrono::Duration::days(10);
        let exp10_ts = exp10.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        lic.expires_at = Some(exp10_ts.clone());
        let l2 = lic.clone();
        let grace_ts = chrono::DateTime::parse_from_rfc3339(&exp10_ts)
            .unwrap()
            .with_timezone(&chrono::Utc)
            .timestamp();
        assert_eq!(
            offline_grace_ts(&l2),
            Some(grace_ts),
            "grace must be capped at expires_at"
        );

        // (b) Expires 90 days from now → grace stays at 30 days.
        let exp90 = now + chrono::Duration::days(90);
        lic.expires_at = Some(exp90.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        let last_ts = chrono::DateTime::parse_from_rfc3339(&lic.last_verified_at)
            .unwrap()
            .with_timezone(&chrono::Utc)
            .timestamp();
        assert_eq!(
            offline_grace_ts(&lic),
            Some(last_ts + 30 * 86_400),
            "grace must be last_verified_at + 30d when cap is later"
        );

        // (c) No expires_at (lifetime) → grace = 30 days.
        lic.expires_at = None;
        assert_eq!(
            offline_grace_ts(&lic),
            Some(last_ts + 30 * 86_400),
            "lifetime tier grace = 30d"
        );
    }

    /// 4. Revoked: `is_license_valid` must return false, and a revoked
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

    /// Compute the offline-grace unix timestamp the same way
    /// [`verify_license`] does, for assertions in tests.
    fn offline_grace_ts(lic: &LicenseFile) -> Option<i64> {
        let last_ts = parse_ts_unix(&lic.last_verified_at)?;
        let grace_ts = last_ts + OFFLINE_GRACE_DAYS * 86_400;
        let final_ts = match parse_ts_unix(lic.expires_at.as_deref().unwrap_or("")) {
            Some(exp_ts) if exp_ts < grace_ts => exp_ts,
            _ => grace_ts,
        };
        Some(final_ts)
    }

    /// 5. `verify_license` offline path (L-08): unreachable GCC base URL
    /// produces a network error → `offline_grace_until` is persisted as
    /// ISO-8601, grace = last_verified_at + 30 days, and the function
    /// returns `Ok(())` (offline use stays permitted).
    ///
    /// Shares a static mutex with `test_license_file_lifecycle` so both
    /// tests write license.json sequentially, not concurrently.
    #[test]
    fn test_verify_license_offline_sets_grace() {
        let _guard = license_file_guard();
        let p = license_path().unwrap();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let mut lic = sample_license();
        let last = chrono::Utc::now() - chrono::Duration::days(1);
        lic.last_verified_at = last.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        save_license(&lic).unwrap();

        // Unroutable port → guaranteed network error (no mock server needed).
        let res = verify_license(&lic, "http://127.0.0.1:1");
        assert!(res.is_ok(), "offline verify must not error, got {res:?}");

        let updated = load_license().unwrap().unwrap();
        assert!(
            updated.offline_grace_until.is_some(),
            "offline verify must persist offline_grace_until"
        );
        let grace =
            chrono::DateTime::parse_from_rfc3339(&updated.offline_grace_until.as_ref().unwrap())
                .unwrap()
                .with_timezone(&chrono::Utc);
        let expected = last + chrono::Duration::days(30);
        assert_eq!(grace.timestamp(), expected.timestamp());
        assert!(
            is_license_valid(&updated),
            "license within grace window must stay valid"
        );

        let _ = remove_license();
    }
}
