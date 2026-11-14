//! Pro Sync — license, hardware binding & session management (Phase 2).
//!
//! - [`HwidGenerator`] — stable hardware ID for license binding.
//! - [`SessionManager`] — JWT access (15 min) + refresh (60 h) tokens.
//! - [`Ed25519Verifier`] — verifies GCC API response signatures.
//! - [`FastBootCache`] & [`fast_boot_cache`] — on-disk cache for instant Pro-state startup.

use anyhow::{anyhow, bail, Result};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ── GCC Ed25519 public key ──────────────────────────────────────────────────
/// GCC server's Ed25519 public key, hex-encoded (32 bytes = 64 hex characters).
pub const GCC_ED25519_PUBLIC_KEY: &str =
    "4aed277ac7b92ee58778d3c5233ebda741c652c753e3dae7127f367dbb58d92f";

/// Standard token lifetimes:
/// - JWT access token: 15 minutes (900 seconds)
/// - Refresh token: 60 hours (216,000 seconds)
pub const JWT_ACCESS_EXPIRY_SECS: i64 = 15 * 60;
pub const REFRESH_TOKEN_EXPIRY_SECS: i64 = 60 * 3600;

// ── HwidGenerator ───────────────────────────────────────────────────────────

/// Generates and caches a stable hardware ID (HWID).
///
/// The HWID is derived from the machine's unique platform identifier (via
/// `machine-uid`) combined with a domain salt, ensuring it is deterministic
/// across process restarts but strictly machine-bound.
#[derive(Debug, Default, Clone)]
pub struct HwidGenerator {
    cache_path: PathBuf,
}

impl HwidGenerator {
    /// Create a generator that caches the HWID at `~/.local/share/cashell/hwid`
    /// (Linux) or platform-appropriate data directory.
    pub fn new() -> Self {
        Self {
            cache_path: crate::paths::hwid_path(),
        }
    }

    /// Return the cached HWID, or generate and persist one if not present.
    pub fn get_or_create(&self) -> Result<String> {
        if let Ok(cached) = std::fs::read_to_string(&self.cache_path) {
            let cached = cached.trim();
            if !cached.is_empty() {
                return Ok(cached.to_string());
            }
        }
        let hwid = Self::generate_hwid()?;
        if let Some(parent) = self.cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.cache_path, &hwid);
        Ok(hwid)
    }

    /// Generate a HWID string using system attributes:
    /// HMAC-SHA256 of `machine_uid()` with `cashell-hwid-v1` salt.
    pub fn generate(&self) -> Result<String> {
        Self::generate_hwid()
    }

    /// Associated function to generate a HWID directly without filesystem cache.
    pub fn generate_hwid() -> Result<String> {
        let uid = machine_uid::get()
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                // Fallback: check linux /etc/machine-id or dbus machine-id
                std::fs::read_to_string("/etc/machine-id")
                    .ok()
                    .or_else(|| std::fs::read_to_string("/var/lib/dbus/machine-id").ok())
                    .map(|s| s.trim().to_string())
            })
            .unwrap_or_else(|| {
                // Secondary fallback: hostname + system info
                let hostname =
                    std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
                let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
                format!("fallback-{hostname}-{user}")
            });

        let mut m = Hmac::<Sha256>::new_from_slice(b"cashell-hwid-v1")
            .map_err(|e| anyhow!("hmac key init error: {e}"))?;
        m.update(uid.as_bytes());
        let digest = m.finalize().into_bytes();
        Ok(hex::encode(&digest[..16]))
    }
}

// ── hex helpers ─────────────────────────────────────────────────────────────
pub mod hex {
    use super::*;

    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn decode(s: &str) -> Result<Vec<u8>> {
        let s = s.trim().trim_start_matches("0x");
        if s.len() % 2 != 0 {
            bail!("odd hex length: {}", s.len());
        }
        s.as_bytes()
            .chunks(2)
            .map(|c| {
                let text = std::str::from_utf8(c).map_err(|e| anyhow!("invalid utf8: {e}"))?;
                u8::from_str_radix(text, 16).map_err(|e| anyhow!("invalid hex byte: {e}"))
            })
            .collect()
    }
}

// ── JWT Utilities ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JwtHeader {
    pub alg: String,
    pub typ: String,
}

impl Default for JwtHeader {
    fn default() -> Self {
        Self {
            alg: "HS256".to_string(),
            typ: "JWT".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JwtClaims {
    pub sub: String,
    pub hwid: String,
    pub tier: String,
    pub token_type: String, // "access" | "refresh"
    pub iat: i64,
    pub exp: i64,
}

impl JwtClaims {
    pub fn is_expired(&self) -> bool {
        let now = now_unix();
        self.exp <= now
    }
}

/// URL-safe base64 encoding without padding.
#[doc(hidden)]
pub fn b64_url_encode_public(data: &[u8]) -> String {
    b64_url_encode(data)
}

fn b64_url_encode(data: &[u8]) -> String {
    use base64ct::{Base64UrlUnpadded, Encoding};
    Base64UrlUnpadded::encode_string(data)
}

/// URL-safe base64 decoding without padding.
fn b64_url_decode(s: &str) -> Result<Vec<u8>> {
    use base64ct::{Base64UrlUnpadded, Encoding};
    let trimmed = s.trim_end_matches('=');
    Base64UrlUnpadded::decode_vec(trimmed).map_err(|e| anyhow!("base64url decode failed: {e}"))
}

/// Sign and produce a JWT (HS256).
pub fn sign_jwt(claims: &JwtClaims, secret: &[u8]) -> Result<String> {
    let header = JwtHeader::default();
    let header_json = serde_json::to_vec(&header)?;
    let claims_json = serde_json::to_vec(&claims)?;

    let header_b64 = b64_url_encode(&header_json);
    let claims_b64 = b64_url_encode(&claims_json);
    let signing_input = format!("{header_b64}.{claims_b64}");

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|e| anyhow!("hmac init error: {e}"))?;
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = b64_url_encode(&sig);

    Ok(format!("{signing_input}.{sig_b64}"))
}

/// Decode and verify a JWT (HS256).
pub fn verify_jwt(token: &str, secret: &[u8]) -> Result<JwtClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        bail!("invalid jwt: expected 3 parts, got {}", parts.len());
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|e| anyhow!("hmac init error: {e}"))?;
    mac.update(signing_input.as_bytes());
    let expected_sig = mac.finalize().into_bytes();

    let actual_sig = b64_url_decode(parts[2])?;
    if expected_sig.as_slice() != actual_sig.as_slice() {
        bail!("jwt signature mismatch");
    }

    let claims_bytes = b64_url_decode(parts[1])?;
    let claims: JwtClaims = serde_json::from_slice(&claims_bytes)?;
    if claims.is_expired() {
        bail!("jwt has expired (exp: {}, now: {})", claims.exp, now_unix());
    }
    Ok(claims)
}

/// Decode JWT claims without verifying cryptographic signature.
pub fn decode_jwt_unverified(token: &str) -> Result<JwtClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        bail!("invalid jwt: expected 3 parts, got {}", parts.len());
    }
    let claims_bytes = b64_url_decode(parts[1])?;
    let claims: JwtClaims = serde_json::from_slice(&claims_bytes)?;
    Ok(claims)
}

// ── SessionManager ──────────────────────────────────────────────────────────

/// Manages Pro license session:
/// - JWT access token (15 min validity)
/// - Refresh token (60 hour validity)
/// - Bound hardware ID (HWID)
/// - Automated refresh flow with GCC server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManager {
    access_token: String,
    refresh_token: String,
    access_expires_unix: i64,
    refresh_expires_unix: i64,
    hwid: String,
    pub api_base: String,
}

impl SessionManager {
    /// Build a session manager from existing tokens.
    pub fn new(
        access_token: String,
        refresh_token: String,
        hwid: String,
        api_base: String,
    ) -> Self {
        let now = now_unix();
        // If tokens contain decodable claims, extract exact exp; otherwise use defaults
        let access_exp = decode_jwt_unverified(&access_token)
            .map(|c| c.exp)
            .unwrap_or(now + JWT_ACCESS_EXPIRY_SECS);
        let refresh_exp = decode_jwt_unverified(&refresh_token)
            .map(|c| c.exp)
            .unwrap_or(now + REFRESH_TOKEN_EXPIRY_SECS);

        Self {
            access_token,
            refresh_token,
            access_expires_unix: access_exp,
            refresh_expires_unix: refresh_exp,
            hwid,
            api_base,
        }
    }

    /// Create a session directly from an active license file and GCC API base URL (L-12).
    /// Uses `activation_token` when present, or falls back to license key (per V-04).
    pub fn from_license(lic: &crate::license_local::LicenseFile, api_base: &str) -> Result<Self> {
        let token = lic
            .activation_token
            .as_deref()
            .unwrap_or(&lic.key)
            .to_string();
        let hwid = HwidGenerator::new().get_or_create()?;
        let now = now_unix();
        let exp = lic
            .expires_at
            .as_deref()
            .and_then(|exp_str| {
                if let Ok(ts) = exp_str.parse::<i64>() {
                    return Some(ts);
                }
                chrono::DateTime::parse_from_rfc3339(exp_str)
                    .ok()
                    .map(|dt| dt.timestamp())
            })
            .unwrap_or(now + 365 * 24 * 3600); // 1-year default if lifetime/unspecified

        Ok(Self {
            access_token: token.clone(),
            refresh_token: token,
            access_expires_unix: exp,
            refresh_expires_unix: exp,
            hwid,
            api_base: api_base.trim_end_matches("/").to_string(),
        })
    }

    /// Return the current JWT access token.
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Return the refresh token.
    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    /// Return the bound HWID.
    pub fn hwid(&self) -> &str {
        &self.hwid
    }

    /// Unix timestamp when the access token expires.
    pub fn access_expires_unix(&self) -> i64 {
        self.access_expires_unix
    }

    /// Unix timestamp when the refresh token expires.
    pub fn refresh_expires_unix(&self) -> i64 {
        self.refresh_expires_unix
    }

    /// Returns `true` if the 15-minute access token has expired.
    pub fn is_access_expired(&self) -> bool {
        self.access_expires_unix <= now_unix()
    }

    /// Returns `true` if the 60-hour refresh token has expired.
    pub fn is_refresh_expired(&self) -> bool {
        self.refresh_expires_unix <= now_unix()
    }

    /// Returns `true` if the refresh token is still valid to renew the session.
    pub fn can_refresh(&self) -> bool {
        !self.is_refresh_expired()
    }

    /// Seconds remaining until access token expiration.
    pub fn seconds_until_access_expiry(&self) -> i64 {
        (self.access_expires_unix - now_unix()).max(0)
    }

    /// Exchange the refresh token for a fresh 15-minute access token via the GCC server.
    pub fn refresh(&mut self) -> Result<()> {
        if self.is_refresh_expired() {
            bail!("refresh token expired (60h lifetime exceeded); re-authentication required");
        }

        let url = format!("{}/api/v1/pro/refresh", self.api_base.trim_end_matches('/'));
        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(&url)
            .json(&serde_json::json!({
                "refresh_token": self.refresh_token,
                "hwid": self.hwid
            }))
            .send()
            .map_err(|e| anyhow!("refresh request failed: {e}"))?;

        if !resp.status().is_success() {
            bail!("refresh failed with HTTP status: {}", resp.status());
        }

        let body: serde_json::Value = resp
            .json()
            .map_err(|e| anyhow!("invalid json response: {e}"))?;
        let new_access = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("refresh response missing access_token"))?;

        self.access_token = new_access.to_string();
        self.access_expires_unix = decode_jwt_unverified(&self.access_token)
            .map(|c| c.exp)
            .unwrap_or(now_unix() + JWT_ACCESS_EXPIRY_SECS);

        if let Some(rt) = body.get("refresh_token").and_then(|v| v.as_str()) {
            self.refresh_token = rt.to_string();
            self.refresh_expires_unix = decode_jwt_unverified(&self.refresh_token)
                .map(|c| c.exp)
                .unwrap_or(now_unix() + REFRESH_TOKEN_EXPIRY_SECS);
        }

        Ok(())
    }

    /// Persist the session to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let payload = serde_json::to_string_pretty(&self)?;
        std::fs::write(path, payload)?;
        Ok(())
    }

    /// Load a session from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let sm: SessionManager = serde_json::from_str(&raw)?;
        Ok(sm)
    }

    /// Return the standard path for persisting the Pro session (`~/.config/cashell/pro_session.json`).
    pub fn default_path() -> PathBuf {
        crate::paths::pro_session_path()
    }

    /// Load the session from the default file path.
    pub fn load_default() -> Option<Self> {
        if let Ok(Some(lic)) = crate::license_local::load_license() {
            if crate::license_local::is_license_valid(&lic) {
                let api_base = "https://gcc.fathforce.com";
                if let Ok(sm) = Self::from_license(&lic, api_base) {
                    return Some(sm);
                }
            }
        }
        Self::load(&Self::default_path()).ok()
    }

    /// Save the session to the default file path.
    pub fn save_default(&self) -> Result<()> {
        self.save(&Self::default_path())
    }

    /// Delete the default session file (used on `pro logout`).
    pub fn delete_default() -> Result<()> {
        let path = Self::default_path();
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(anyhow!("failed to remove {}: {e}", path.display())),
        }
    }
}

// ── Ed25519Verifier ─────────────────────────────────────────────────────────

/// Verifies Ed25519 signatures produced by the GCC server.
/// Default initialized with GCC public key:
/// `4aed277ac7b92ee58778d3c5233ebda741c652c753e3dae7127f367dbb58d92f`
#[derive(Debug, Clone)]
pub struct Ed25519Verifier {
    key: VerifyingKey,
    pub public_key_hex: String,
}

impl Default for Ed25519Verifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Ed25519Verifier {
    /// Create a verifier configured with the GCC public key.
    pub fn new() -> Self {
        Self::from_hex(GCC_ED25519_PUBLIC_KEY).expect("GCC public key constant is valid")
    }

    /// Create a verifier from a 64-character hex-encoded public key.
    pub fn from_hex(hex_key: &str) -> Result<Self> {
        let bytes = hex::decode(hex_key)?;
        if bytes.len() != 32 {
            bail!(
                "invalid public key length: expected 32 bytes, got {}",
                bytes.len()
            );
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        let key = VerifyingKey::from_bytes(&arr)
            .map_err(|e| anyhow!("invalid ed25519 public key: {e}"))?;
        Ok(Self {
            key,
            public_key_hex: hex_key.trim().to_lowercase(),
        })
    }

    /// Verify a raw Ed25519 signature (64 bytes) against `data`.
    pub fn verify(&self, data: &[u8], signature: &[u8]) -> bool {
        if signature.len() != 64 {
            return false;
        }
        let sig: Signature = match Signature::from_slice(signature) {
            Ok(s) => s,
            Err(_) => return false,
        };
        self.key.verify(data, &sig).is_ok()
    }

    /// Verify a hex-encoded Ed25519 signature (128 hex chars) against `data`.
    pub fn verify_hex(&self, data: &[u8], signature_hex: &str) -> bool {
        match hex::decode(signature_hex) {
            Ok(sig_bytes) => self.verify(data, &sig_bytes),
            Err(_) => false,
        }
    }
}

// ── FastBootCache & fast_boot_cache ──────────────────────────────────────────

/// On-disk cache that enables instant (< 5ms) CAShell startup without network latency.
///
/// Stores HWID, Pro license status (0=unlicensed, 1=pro, 2=expired),
/// session expiry Unix timestamp, and last sync timestamp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FastBootCache {
    pub hwid: String,
    pub status: u8, // 0=unlicensed, 1=pro, 2=expired
    pub session_expires_unix: i64,
    pub api_base: String,
    pub last_sync: i64,
}

impl FastBootCache {
    pub fn path() -> PathBuf {
        crate::paths::fastboot_cache_path()
    }

    /// Load the cache from disk; returns `None` if absent or corrupt.
    pub fn load() -> Option<Self> {
        let p = Self::path();
        let raw = std::fs::read_to_string(&p).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Remove the cache from disk.
    pub fn clear() -> Result<()> {
        let p = Self::path();
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
        Ok(())
    }

    /// Persist this cache entry to disk.
    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(self)?;
        std::fs::write(&p, json)?;
        Ok(())
    }

    /// Construct a cache entry from an active [`SessionManager`].
    pub fn from_session(sm: &SessionManager) -> Self {
        let now = now_unix();
        Self {
            hwid: sm.hwid().to_string(),
            status: 1,
            session_expires_unix: sm.access_expires_unix(),
            api_base: sm.api_base.clone(),
            last_sync: now,
        }
    }

    /// True if the cached Pro session is currently valid and unexpired.
    pub fn is_valid(&self) -> bool {
        let now = now_unix();
        self.status == 1 && self.session_expires_unix > now
    }
}

/// Convenience function to load the fast boot cache.
pub fn fast_boot_cache() -> Option<FastBootCache> {
    FastBootCache::load()
}

/// Fast boot cache helper module.
pub mod fast_boot_cache {
    use super::*;

    pub fn load() -> Option<FastBootCache> {
        FastBootCache::load()
    }

    pub fn save(cache: &FastBootCache) -> Result<()> {
        cache.save()
    }

    pub fn is_valid() -> bool {
        FastBootCache::load().map(|c| c.is_valid()).unwrap_or(false)
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    #[test]
    fn test_hwid_generation_deterministic() {
        let gen = HwidGenerator::new();
        let id1 = gen.generate().expect("generate hwid");
        let id2 = gen.generate().expect("generate hwid second time");
        assert_eq!(id1, id2, "hwid must be deterministic");
        assert_eq!(id1.len(), 32, "hwid should be 32 hex chars (16 bytes)");
    }

    #[test]
    fn test_session_manager_from_license() {
        let lic = crate::license_local::LicenseFile {
            schema: 2,
            key: "CASH99-FOUNDR-3".to_string(),
            product: "SH".to_string(),
            tier: "99".to_string(),
            expires_at: None,
            activated_at: "2026-11-13T00:00:00Z".to_string(),
            last_verified_at: "2026-11-13T00:00:00Z".to_string(),
            activation_token: Some("sample-activation-token-123".to_string()),
            devices_used: 1,
            devices_max: 3,
            offline_grace_until: None,
            revoked: false,
            expired: false,
        };

        let sm =
            SessionManager::from_license(&lic, "https://gcc.fathforce.com").expect("from_license");
        assert_eq!(sm.access_token(), "sample-activation-token-123");
        assert_eq!(sm.refresh_token(), "sample-activation-token-123");
        assert_eq!(sm.api_base, "https://gcc.fathforce.com");
        assert!(!sm.is_access_expired());
        assert!(!sm.is_refresh_expired());

        // Test fallback when activation_token is None (per V-04)
        let mut lic_no_token = lic.clone();
        lic_no_token.activation_token = None;
        let sm2 = SessionManager::from_license(&lic_no_token, "https://gcc.fathforce.com/")
            .expect("from_license no token");
        assert_eq!(sm2.access_token(), "CASH99-FOUNDR-3");
        assert_eq!(sm2.api_base, "https://gcc.fathforce.com");
    }

    #[test]
    fn test_ed25519_gcc_verifier() {
        let verifier = Ed25519Verifier::new();
        assert_eq!(verifier.public_key_hex, GCC_ED25519_PUBLIC_KEY);

        // Test with a freshly generated keypair to check signing & verification logic
        let secret_bytes = [42u8; 32];
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();
        let pub_hex = hex::encode(verifying_key.as_bytes());

        let custom_verifier = Ed25519Verifier::from_hex(&pub_hex).expect("parse custom verifier");
        let message = b"GCC-VAULT-SYNC-TRANSACTION-1001";
        let signature = signing_key.sign(message);
        let sig_bytes = signature.to_bytes();
        let sig_hex = hex::encode(&sig_bytes);

        assert!(custom_verifier.verify(message, &sig_bytes));
        assert!(custom_verifier.verify_hex(message, &sig_hex));
        assert!(!custom_verifier.verify(b"TAMPERED-MESSAGE", &sig_bytes));
    }

    #[test]
    fn test_fast_boot_cache_validation() {
        let now = now_unix();
        let valid_cache = FastBootCache {
            hwid: "hwid-123".to_string(),
            status: 1,
            session_expires_unix: now + 300,
            api_base: "https://api.fathforce.com".to_string(),
            last_sync: now,
        };
        assert!(valid_cache.is_valid());

        let expired_cache = FastBootCache {
            hwid: "hwid-123".to_string(),
            status: 1,
            session_expires_unix: now - 10,
            api_base: "https://api.fathforce.com".to_string(),
            last_sync: now - 3600,
        };
        assert!(!expired_cache.is_valid());

        let unlicensed_cache = FastBootCache {
            hwid: "hwid-123".to_string(),
            status: 0,
            session_expires_unix: now + 300,
            api_base: "https://api.fathforce.com".to_string(),
            last_sync: now,
        };
        assert!(!unlicensed_cache.is_valid());
    }
}
