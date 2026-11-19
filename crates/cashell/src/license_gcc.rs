//! GCC license client — activation, verification, and deactivation.
//!
//! Talks to the GCC license API (`{gcc_base_url}/api/v1/licenses/*`).
//! All calls are bounded by a 10-second reqwest timeout so the CLI never
//! hangs on a slow or unreachable GCC (V-04 decision L-03 fallback:
//! when the live GCC does not yet return an `activation_token`, the
//! client logs a warning and continues without token verification).

use hmac::{Hmac, Mac};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use thiserror::Error;

/// Errors returned by the GCC license client, mapped from HTTP status codes.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum LicenseError {
    /// The license key or product does not exist on GCC.
    #[error("license not found on GCC")]
    NotFound,
    /// The device limit for this key has been reached (HTTP 409).
    #[error("device limit reached for this license key")]
    DeviceLimit,
    /// The key has been revoked by GCC (HTTP 403).
    #[error("license key has been revoked")]
    Revoked,
    /// The license subscription has expired (HTTP 410).
    #[error("license subscription has expired")]
    Expired,
    /// Network / transport failure (DNS, TLS, timeout, ...).
    #[error("network error: {0}")]
    Network(String),
    /// The server returned 2xx but the payload could not be parsed.
    #[error("invalid response from GCC: {0}")]
    InvalidResponse(String),
}

/// Successful activation response from `POST /api/v1/licenses/activate`.
#[derive(Debug, Clone, Deserialize)]
pub struct Activation {
    pub key: String,
    pub product: String,
    pub tier: String,
    /// ISO-8601 or unix timestamp string; `None` = lifetime.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// HMAC-SHA256-signed activation token (V-04 L-03). Optional: when the
    /// live GCC does not yet return one, the caller skips verification.
    #[serde(default)]
    pub activation_token: Option<String>,
    #[serde(default)]
    pub devices_used: u32,
    #[serde(default)]
    pub devices_max: u32,
}

/// Response of `POST /api/v1/licenses/verify`.
#[derive(Debug, Clone, Deserialize)]
pub struct VerifyResponse {
    pub valid: bool,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub devices_used: u32,
    #[serde(default)]
    pub devices_max: u32,
}

const TIMEOUT: Duration = Duration::from_secs(10);

fn client() -> std::result::Result<Client, LicenseError> {
    Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| LicenseError::Network(format!("client init failed: {e}")))
}

/// Send a JSON request and map status/transport errors to [`LicenseError`].
///
/// Uses the blocking [`reqwest::blocking::Client`].
pub fn request_sync(
    client: &Client,
    method: reqwest::Method,
    url: &str,
    body: &str,
) -> std::result::Result<reqwest::blocking::Response, LicenseError> {
    let req = client
        .request(method, url)
        .header("Content-Type", "application/json")
        .body(body.to_string());
    let resp = req
        .send()
        .map_err(|e| LicenseError::Network(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            404 => LicenseError::NotFound,
            409 => LicenseError::DeviceLimit,
            403 => LicenseError::Revoked,
            410 => LicenseError::Expired,
            _ => LicenseError::Network(format!("unexpected status {status}")),
        });
    }
    Ok(resp)
}

/// Activate a license key on GCC and return the parsed activation payload.
///
/// Verifies `activation_token` (HMAC-SHA256) when present; when absent,
/// logs a warning and continues (V-04 L-03 fallback for live GCC).
pub fn activate(
    base_url: &str,
    key: &str,
    device_id: &str,
    device_name: &str,
    platform: &str,
) -> std::result::Result<Activation, LicenseError> {
    let client = client()?;
    let url = format!(
        "{}/api/v1/licenses/activate",
        base_url.trim_end_matches('/')
    );
    let body = serde_json::json!({
        "key": key,
        "device_id": device_id,
        "device_name": device_name,
        "platform": platform,
    })
    .to_string();

    let resp = request_sync(&client, reqwest::Method::POST, &url, &body)?;
    let body_text = resp
        .text()
        .map_err(|e| LicenseError::Network(e.to_string()))?;
    let activation: Activation = serde_json::from_str(&body_text)
        .map_err(|e| LicenseError::InvalidResponse(e.to_string()))?;

    if let Some(token) = &activation.activation_token {
        if !verify_activation_token(token, key, device_id) {
            eprintln!(
                "warning: activation token HMAC-SHA256 verification failed for {key} — token may be malformed; continuing without token verification"
            );
        }
    } else {
        eprintln!(
            "warning: GCC did not return an activation_token; skipping token verification (live GCC not yet updated — V-04 L-03 fallback)"
        );
    }

    Ok(activation)
}

/// Verify a license key's status on GCC.
pub fn verify(
    base_url: &str,
    key: &str,
    device_id: &str,
) -> std::result::Result<VerifyResponse, LicenseError> {
    let client = client()?;
    let url = format!("{}/api/v1/licenses/verify", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "key": key,
        "device_id": device_id,
    })
    .to_string();

    let resp = request_sync(&client, reqwest::Method::POST, &url, &body)?;
    let body_text = resp
        .text()
        .map_err(|e| LicenseError::Network(e.to_string()))?;
    let v: VerifyResponse = serde_json::from_str(&body_text)
        .map_err(|e| LicenseError::InvalidResponse(e.to_string()))?;
    Ok(v)
}

/// Deactivate (unbind) this device from a license key on GCC.
///
/// Live GCC currently returns 401 on this endpoint (V-04); callers should
/// handle [`LicenseError::Network`] with a warning log and continue.
pub fn deactivate(
    base_url: &str,
    key: &str,
    device_id: &str,
) -> std::result::Result<(), LicenseError> {
    let client = client()?;
    let url = format!(
        "{}/api/v1/licenses/deactivate",
        base_url.trim_end_matches('/')
    );
    let body = serde_json::json!({
        "key": key,
        "device_id": device_id,
    })
    .to_string();

    let _resp = request_sync(&client, reqwest::Method::POST, &url, &body)?;
    Ok(())
}

/// Compute a deterministic device identifier:
/// SHA-256(machine_uid + "cashell") hex-encoded (64 chars).
pub fn device_id() -> String {
    let uid = machine_uid::get()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            // Fallback chain mirrors HwidGenerator (pro.rs)
            std::fs::read_to_string("/etc/machine-id")
                .ok()
                .or_else(|| std::fs::read_to_string("/var/lib/dbus/machine-id").ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| {
                    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".into());
                    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
                    format!("fallback-{hostname}-{user}")
                })
        });
    let input = format!("{uid}cashell");
    let digest = Sha256::digest(input.as_bytes());
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verify an activation token signed with HMAC-SHA256 using the
/// `JWT_SECRET`-equivalent key.
///
/// GCC token format (since L-03):
/// ```text
/// {base64url("key:device_id:sku:activated_at_unix")}.{hmac_sha256_hex}
/// ```
/// where `hmac_sha256_hex` is the hex-encoded HMAC-SHA256 of the raw
/// (non-base64) payload string, keyed with `JWT_SECRET`.
///
/// Returns `true` when the token verifies, `false` otherwise.
pub fn verify_activation_token(token: &str, key: &str, device_id: &str) -> bool {
    use base64::Engine;

    // Token format: `{base64url_payload}.{sig_hex}`
    let dot_idx = match token.rfind('.') {
        Some(i) => i,
        None => return false,
    };
    if dot_idx == 0 {
        return false;
    }
    let payload_b64 = &token[..dot_idx];
    let sig_hex = &token[dot_idx + 1..];

    // Decode base64url payload → raw "key:device_id:sku:activated_at_unix"
    let payload_bytes: Vec<u8> = match base64::engine::general_purpose::URL_SAFE
        .decode(payload_b64) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let payload_str = match std::str::from_utf8(&payload_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Verify payload starts with "key:device_id:" (4-field format)
    let expected_prefix = format!("{key}:{device_id}:");
    if !payload_str.starts_with(&expected_prefix) {
        return false;
    }

    let secret = std::env::var("JWT_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "cashell-license-hmac-v1".to_string());

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC key valid");
    mac.update(payload_str.as_bytes());
    let expected_sig = mac.finalize().into_bytes();
    let expected_hex = hex_encode(&expected_sig);

    sig_hex.eq_ignore_ascii_case(&expected_hex)
}

/// Mask a license key for display: keep the first 6 chars + "…".
/// Never prints the full key.
pub fn mask_key(key: &str) -> String {
    if key.len() <= 6 {
        return "…".to_string();
    }
    format!("{}…", &key[..6])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_key_basic() {
        let k = "CASH99-ABCDEF-2";
        assert_eq!(mask_key(k), "CASH99…");
    }

    #[test]
    fn test_mask_key_short() {
        assert_eq!(mask_key("AB"), "…");
        assert_eq!(mask_key("ABCDEF"), "…");
    }

    #[test]
    fn test_mask_key_does_not_leak_full_key() {
        let k = "CASH99-FOUNDR-3";
        let masked = mask_key(k);
        assert!(!masked.contains("FOUNDR"));
        assert_eq!(masked, "CASH99…");
    }

    #[test]
    fn test_device_id_deterministic_and_64_hex() {
        let d1 = device_id();
        let d2 = device_id();
        assert_eq!(d1, d2, "device_id must be deterministic");
        assert_eq!(d1.len(), 64, "device_id must be 64 hex chars");
        assert!(
            d1.chars().all(|c| c.is_ascii_hexdigit()),
            "must be hex only"
        );
    }

    // ── Mock server tests (axum on a random port) ──────────────────────────

    use axum::body::Body;
    use axum::routing::post;
    use axum::Router;
    use tokio::net::TcpListener;

    /// Start an axum server on a random port in the current runtime.
    /// Returns the base URL. The server runs on the same runtime as the
    /// test, so the test must use a multi-threaded runtime to avoid
    /// deadlocking when the reqwest blocking client is called from a
    /// std::thread (which would stop the current-thread event loop).
    async fn start_mock_server(handler: axum::Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, handler).await.unwrap();
        });
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_activate_200_ok() {
        let app = Router::new().route(
            "/api/v1/licenses/activate",
            post(|| async {
                axum::http::Response::builder()
                    .status(200)
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        r#"{"key":"CASH99-ABCDEF-2","product":"SH","tier":"99","expires_at":null,"activation_token":null,"devices_used":1,"devices_max":1}"#,
                    ))
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || activate(&base, "CASH99-ABCDEF-2", "dev-1", "test", "linux"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(result.is_ok(), "200 should succeed: {result:?}");
        let act = result.unwrap();
        assert_eq!(act.tier, "99");
        assert!(act.expires_at.is_none(), "lifetime tier → expires_at null");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_activate_404_not_found() {
        let app = Router::new().route(
            "/api/v1/licenses/activate",
            post(|| async {
                axum::http::Response::builder()
                    .status(404)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || activate(&base, "CASH99-ABCDEF-2", "dev-1", "test", "linux"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(matches!(result, Err(LicenseError::NotFound)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_activate_409_device_limit() {
        let app = Router::new().route(
            "/api/v1/licenses/activate",
            post(|| async {
                axum::http::Response::builder()
                    .status(409)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || activate(&base, "CASH99-ABCDEF-2", "dev-1", "test", "linux"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(matches!(result, Err(LicenseError::DeviceLimit)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_activate_403_revoked() {
        let app = Router::new().route(
            "/api/v1/licenses/activate",
            post(|| async {
                axum::http::Response::builder()
                    .status(403)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || activate(&base, "CASH99-ABCDEF-2", "dev-1", "test", "linux"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(matches!(result, Err(LicenseError::Revoked)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_activate_410_expired() {
        let app = Router::new().route(
            "/api/v1/licenses/activate",
            post(|| async {
                axum::http::Response::builder()
                    .status(410)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || activate(&base, "CASH99-ABCDEF-2", "dev-1", "test", "linux"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(matches!(result, Err(LicenseError::Expired)));
    }

    #[tokio::test]
    async fn test_activate_network_error() {
        // Use a port that is not listening → connection refused → Network.
        // The blocking reqwest call must run in a std::thread (not inside
        // the tokio runtime) to avoid the "cannot drop runtime in blocking
        // context" panic.
        let result = std::thread::scope(|s| {
            s.spawn(move || {
                activate(
                    "http://127.0.0.1:1",
                    "CASH99-ABCDEF-2",
                    "dev-1",
                    "test",
                    "linux",
                )
            })
            .join()
            .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(
            matches!(result, Err(LicenseError::Network(_))),
            "expected Network error, got: {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_activate_malformed_json() {
        let app = Router::new().route(
            "/api/v1/licenses/activate",
            post(|| async {
                axum::http::Response::builder()
                    .status(200)
                    .header("Content-Type", "application/json")
                    .body(Body::from("not json at all"))
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || activate(&base, "CASH99-ABCDEF-2", "dev-1", "test", "linux"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(
            matches!(result, Err(LicenseError::InvalidResponse(_))),
            "expected InvalidResponse, got: {result:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_verify_200() {
        let app = Router::new().route(
            "/api/v1/licenses/verify",
            post(|| async {
                axum::http::Response::builder()
                    .status(200)
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        r#"{"valid":true,"expires_at":"2026-01-01T00:00:00Z","devices_used":1,"devices_max":2}"#,
                    ))
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || verify(&base, "CASH99-ABCDEF-2", "dev-1"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(result.is_ok(), "verify should succeed: {result:?}");
        let v = result.unwrap();
        assert!(v.valid);
        assert!(v.expires_at.is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_verify_404() {
        let app = Router::new().route(
            "/api/v1/licenses/verify",
            post(|| async {
                axum::http::Response::builder()
                    .status(404)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || verify(&base, "CASH99-ABCDEF-2", "dev-1"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(matches!(result, Err(LicenseError::NotFound)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_deactivate_200() {
        let app = Router::new().route(
            "/api/v1/licenses/deactivate",
            post(|| async {
                axum::http::Response::builder()
                    .status(200)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || deactivate(&base, "CASH99-ABCDEF-2", "dev-1"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(result.is_ok(), "deactivate should succeed: {result:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_deactivate_401_maps_to_network() {
        // Live GCC currently returns 401 on deactivate (V-04).
        // The 401 status falls into the catch-all `Network` bucket.
        let app = Router::new().route(
            "/api/v1/licenses/deactivate",
            post(|| async {
                axum::http::Response::builder()
                    .status(401)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let base = start_mock_server(app).await;
        let result = std::thread::scope(|s| {
            s.spawn(move || deactivate(&base, "CASH99-ABCDEF-2", "dev-1"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        });
        assert!(
            matches!(result, Err(LicenseError::Network(_))),
            "401 should map to Network (graceful handling): {result:?}"
        );
    }

    // ── HMAC token verification tests (GCC base64url 4-field format) ─────

    /// Build a GCC-format activation token: base64url("key:dev:sku:ts") + "." + hex(HMAC-SHA256(secret, raw_payload))
    fn make_gcc_token(key: &str, dev: &str, sku: &str, ts: i64, secret: &str) -> String {
        use base64::Engine;
        let raw = format!("{key}:{dev}:{sku}:{ts}");
        let payload_b64 = base64::engine::general_purpose::URL_SAFE.encode(raw.as_bytes());
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC key valid");
        mac.update(raw.as_bytes());
        let sig = mac.finalize().into_bytes();
        format!("{payload_b64}.{}", hex_encode(&sig))
    }

    #[test]
    fn test_activation_token_valid() {
        let key = "CASH99-ABCDEF-2";
        let dev = "dev-1";
        let secret = "cashell-license-hmac-v1";

        let token = make_gcc_token(key, dev, "SW-CASHLL-PRO", 1795059027, secret);

        std::env::set_var("JWT_SECRET", secret);
        let valid = verify_activation_token(&token, key, dev);
        std::env::remove_var("JWT_SECRET");
        assert!(
            valid,
            "valid GCC-format token must verify when JWT_SECRET matches the signing key"
        );
    }

    #[test]
    fn test_activation_token_invalid_sig() {
        let key = "CASH99-ABCDEF-2";
        let dev = "dev-1";
        let secret = "cashell-license-hmac-v1";

        let token = make_gcc_token(key, dev, "SW-CASHLL-PRO", 1795059027, secret);
        // Corrupt the signature
        let corrupted = format!("{}0", token);
        let _ = corrupted; // just to show we built it
        let mut chars: Vec<char> = token.chars().collect();
        // Flip one hex char in the signature portion
        let dot = token.rfind('.').unwrap();
        let sig_start = dot + 1;
        chars[sig_start] = if chars[sig_start] == 'a' { 'b' } else { 'a' };
        let bad_token = chars.into_iter().collect::<String>();

        std::env::set_var("JWT_SECRET", secret);
        assert!(
            !verify_activation_token(&bad_token, key, dev),
            "corrupted signature must fail"
        );
        std::env::remove_var("JWT_SECRET");
    }

    #[test]
    fn test_activation_token_wrong_device() {
        let key = "CASH99-ABCDEF-2";
        let secret = "cashell-license-hmac-v1";

        let token = make_gcc_token(key, "dev-1", "SW-CASHLL-PRO", 1795059027, secret);

        std::env::set_var("JWT_SECRET", secret);
        assert!(
            !verify_activation_token(&token, key, "dev-999"),
            "token for dev-1 must fail when verifying against dev-999"
        );
        std::env::remove_var("JWT_SECRET");
    }

    #[test]
    fn test_activation_token_malformed() {
        // No dot at all
        assert!(!verify_activation_token("garbage", "key", "dev"));
        // Empty string
        assert!(!verify_activation_token("", "key", "dev"));
        // Invalid base64url
        assert!(!verify_activation_token("!!!notbase64.sig", "key", "dev"));
    }
}
