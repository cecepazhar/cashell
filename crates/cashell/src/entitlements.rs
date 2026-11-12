//! Entitlements — single source of truth for Pro feature gating (L-10).
//!
//! Read `~/.local/share/cashell/license.json` via [`crate::license_local`],
//! cache the result for 60 seconds (thread-local) so hot TUI loops don't
//! re-read the file on every frame.
//!
//! Callers must only use [`Entitlements::is_pro`] to decide whether a
//! Pro-only feature is allowed. Free features must NOT call this module.

use std::sync::Mutex;
use std::time::SystemTime;

use crate::license_local::{self, LicenseFile};

/// Pro entitlement snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlements {
    /// True only when a valid license is installed AND the tier is not FREE.
    pub is_pro: bool,
    /// License tier code: "99" / "24" / "12" / "01", or "FREE" when no license.
    pub tier: String,
    /// Subscription expiration (None = lifetime or no license).
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// How long the cached snapshot stays fresh.
const CACHE_TTL: u64 = 60;

/// Cached snapshot + fill timestamp, guarded by a process-wide Mutex.
/// `Option<Entitlements>` is `None` when no license file is present.
static CACHE: Mutex<Option<(Entitlements, u64)>> = Mutex::new(None);

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Return the current entitlements, cached for up to [`CACHE_TTL`] per process.
pub fn current() -> Entitlements {
    let now = now_unix();
    {
        let guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((ent, at)) = guard.as_ref() {
            if at + CACHE_TTL >= now {
                return ent.clone();
            }
        }
    }

    let ent = compute();
    let mut guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some((ent.clone(), now));
    ent
}

fn compute() -> Entitlements {
    match license_local::load_license().ok().flatten() {
        Some(lic) => compute_from(&lic),
        None => Entitlements {
            is_pro: false,
            tier: "FREE".to_string(),
            expires_at: None,
        },
    }
}

fn compute_from(lic: &LicenseFile) -> Entitlements {
    let valid = license_local::is_license_valid(lic);
    let is_pro = valid && lic.tier != "FREE";
    let expires_at = parse_ts(lic.expires_at.as_deref());
    Entitlements {
        is_pro,
        tier: lic.tier.clone(),
        expires_at,
    }
}

/// Parse an ISO-8601 or unix-timestamp string to `DateTime<Utc>`.
fn parse_ts(s: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    let s = s?;
    if let Ok(ts) = s.parse::<i64>() {
        return chrono::DateTime::from_timestamp(ts, 0);
    }
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Reset the cache so tests start clean.
    fn reset_cache() {
        *CACHE.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    #[test]
    fn no_license_is_free() {
        // Ensure no license file interferes.
        let _ = crate::license_local::remove_license();
        reset_cache();
        let e = current();
        assert!(!e.is_pro, "no license file → not pro");
        assert_eq!(e.tier, "FREE");
        assert!(e.expires_at.is_none());
    }

    #[test]
    fn pro_node_registration_gate() {
        let mut e = Entitlements {
            is_pro: true,
            tier: "99".to_string(),
            expires_at: None,
        };
        assert!(
            e.is_pro,
            "is_pro = true → remote fleet node registration allowed"
        );

        e.is_pro = false;
        assert!(
            !e.is_pro,
            "is_pro = false → remote fleet node registration denied"
        );
    }

    #[test]
    fn tier_free_is_not_pro_even_when_valid() {
        let _ = crate::license_local::remove_license();
        reset_cache();
        let ent = Entitlements {
            is_pro: false,
            tier: "FREE".to_string(),
            expires_at: None,
        };
        assert!(!ent.is_pro);
        // And the default FREE snapshot matches:
        let free = Entitlements {
            is_pro: false,
            tier: "FREE".to_string(),
            expires_at: None,
        };
        assert_eq!(ent, free);
    }
}
