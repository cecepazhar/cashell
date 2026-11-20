//! GCC license client — activation, verification, and deactivation.
//!
//! Talks to the GCC core license API (`{gcc_base_url}/api/v1/licenses/*`) following the
//! "GCC License v2" standard: responses are `{success, data:{...}}` and carry an Ed25519
//! `signed_token`, which is the only thing that grants Pro (see `license_token`). All calls are
//! bounded by a 10-second timeout so the CLI never hangs on a slow or unreachable GCC.

use crate::license_token::{self, SignedToken};
use ed25519_dalek::VerifyingKey;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;
use thiserror::Error;

/// Errors returned by the GCC license client, mapped from HTTP status codes and messages.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum LicenseError {
    /// The license key or product does not exist on GCC.
    #[error("license not found on GCC")]
    NotFound,
    /// The device limit for this key has been reached.
    #[error("device limit reached for this license key")]
    DeviceLimit,
    /// The key has been revoked (or is otherwise not active) on GCC.
    #[error("license key has been revoked")]
    Revoked,
    /// The license subscription has expired.
    #[error("license subscription has expired")]
    Expired,
    /// Network / transport failure (DNS, TLS, timeout, 5xx, ...).
    #[error("network error: {0}")]
    Network(String),
    /// The server answered but the payload was unusable (including no valid signed token).
    #[error("invalid response from GCC: {0}")]
    InvalidResponse(String),
}

/// Successful activation, already checked: `signed_token` verified for this product + device.
#[derive(Debug, Clone)]
pub struct Activation {
    pub key: String,
    pub product: String,
    pub tier: String,
    /// ISO-8601 licence end; `None` = lifetime. Display only.
    pub expires_at: Option<String>,
    pub signed_token: SignedToken,
    pub devices_used: u32,
    pub devices_max: u32,
    /// End of the current offline window (token `exp`, unix seconds).
    pub token_expires_unix: i64,
}

/// Result of `POST /api/v1/licenses/verify`.
#[derive(Debug, Clone)]
pub struct VerifyResponse {
    pub valid: bool,
    pub status: String,
    pub expires_at: Option<String>,
    pub device_registered: bool,
    pub devices_max: u32,
    /// Fresh 14-day token when the licence is valid on this device.
    pub signed_token: Option<SignedToken>,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct ActivateData {
    license_key: String,
    #[serde(default)]
    product_sku: String,
    #[serde(default)]
    plan_tier: String,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    max_devices: u32,
    #[serde(default)]
    signed_token: Option<SignedToken>,
}

#[derive(Deserialize)]
struct VerifyData {
    #[serde(default)]
    valid: bool,
    #[serde(default)]
    status: String,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    device_registered: bool,
    #[serde(default)]
    max_devices: u32,
    #[serde(default)]
    signed_token: Option<SignedToken>,
}

const TIMEOUT: Duration = Duration::from_secs(10);

fn client() -> std::result::Result<Client, LicenseError> {
    Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| LicenseError::Network(format!("client init failed: {e}")))
}

/// Maps a GCC error (`{success:false, error:{message}}`) to [`LicenseError`].
fn map_error(status: u16, body: &str) -> LicenseError {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message").or(Some(e)))
                .and_then(|m| m.as_str().map(str::to_string))
        })
        .unwrap_or_default();
    let m = message.to_lowercase();
    match status {
        404 => LicenseError::NotFound,
        409 => LicenseError::DeviceLimit,
        403 => LicenseError::Revoked,
        410 => LicenseError::Expired,
        400 if m.contains("device limit") || m.contains("perangkat") => LicenseError::DeviceLimit,
        400 if m.contains("kedaluwarsa") || m.contains("expired") => LicenseError::Expired,
        400 if m.contains("berstatus") || m.contains("revoked") => LicenseError::Revoked,
        400 => LicenseError::InvalidResponse(message),
        _ => LicenseError::Network(format!("unexpected status {status}")),
    }
}

/// Send a JSON request; returns the body of a 2xx response or a mapped [`LicenseError`].
pub fn request_sync(
    client: &Client,
    method: reqwest::Method,
    url: &str,
    body: &str,
) -> std::result::Result<String, LicenseError> {
    let resp = client
        .request(method, url)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .map_err(|e| LicenseError::Network(e.to_string()))?;
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| LicenseError::Network(e.to_string()))?;
    if !status.is_success() {
        return Err(map_error(status.as_u16(), &text));
    }
    Ok(text)
}

fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}/api/v1/licenses/{path}", base_url.trim_end_matches('/'))
}

/// Activate a license key on GCC. Fails closed: without a `signed_token` that verifies for
/// CASHELL on this device, nothing is returned (and nothing gets saved by the caller).
pub fn activate(
    base_url: &str,
    key: &str,
    device_id: &str,
    device_name: &str,
    platform: &str,
) -> std::result::Result<Activation, LicenseError> {
    activate_with_key(base_url, key, device_id, device_name, platform, license_token::public_key)
}

/// [`activate`] with an injectable public key (tests).
pub fn activate_with_key(
    base_url: &str,
    key: &str,
    device_id: &str,
    device_name: &str,
    platform: &str,
    key_for: impl Fn(u32) -> Option<VerifyingKey>,
) -> std::result::Result<Activation, LicenseError> {
    let body = serde_json::json!({
        "license_key": key,
        "device_id": device_id,
        "device_name": device_name,
        "platform": platform,
    })
    .to_string();
    let text = request_sync(&client()?, reqwest::Method::POST, &endpoint(base_url, "activate"), &body)?;
    let data = serde_json::from_str::<Envelope<ActivateData>>(&text)
        .map_err(|e| LicenseError::InvalidResponse(e.to_string()))?
        .data;
    let token = data
        .signed_token
        .ok_or_else(|| LicenseError::InvalidResponse("GCC returned no signed_token".into()))?;
    let now = chrono::Utc::now().timestamp();
    let claims = license_token::verify(Some(&token), key_for, device_id, now, now)
        .map_err(|e| LicenseError::InvalidResponse(e.to_string()))?;
    Ok(Activation {
        key: data.license_key,
        product: data.product_sku,
        tier: data.plan_tier,
        expires_at: data.expires_at,
        signed_token: token,
        devices_used: 0,
        devices_max: data.max_devices,
        token_expires_unix: claims.exp,
    })
}

/// Verify a license key's status on GCC (and receive a refreshed token when valid).
pub fn verify(
    base_url: &str,
    key: &str,
    device_id: &str,
) -> std::result::Result<VerifyResponse, LicenseError> {
    let body = serde_json::json!({ "license_key": key, "device_id": device_id }).to_string();
    let text = request_sync(&client()?, reqwest::Method::POST, &endpoint(base_url, "verify"), &body)?;
    let d = serde_json::from_str::<Envelope<VerifyData>>(&text)
        .map_err(|e| LicenseError::InvalidResponse(e.to_string()))?
        .data;
    Ok(VerifyResponse {
        valid: d.valid,
        status: d.status,
        expires_at: d.expires_at,
        device_registered: d.device_registered,
        devices_max: d.max_devices,
        signed_token: d.signed_token,
    })
}

/// Deactivate (unbind) this device from a license key on GCC.
pub fn deactivate(
    base_url: &str,
    key: &str,
    device_id: &str,
) -> std::result::Result<(), LicenseError> {
    let body = serde_json::json!({ "license_key": key, "device_id": device_id }).to_string();
    request_sync(&client()?, reqwest::Method::POST, &endpoint(base_url, "deactivate"), &body)?;
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
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use tokio::net::TcpListener;

    async fn start_mock_server(handler: axum::Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, handler).await.unwrap();
        });
        format!("http://127.0.0.1:{port}")
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn test_key(_: u32) -> Option<VerifyingKey> {
        Some(signing_key().verifying_key())
    }

    fn signed(products: &[&str], hwid: &str) -> serde_json::Value {
        let exp = chrono::Utc::now().timestamp() + 14 * 86_400;
        let payload = serde_json::json!({ "v": 2, "lic_id": "l", "sku": "CASHELL",
            "products": products, "tier": "lifetime", "features": ["pro"], "hwid": hwid,
            "max_devices": 1, "iat": 0, "exp": exp, "lic_exp": 0, "nonce": "n" })
        .to_string();
        let signature = base64::engine::general_purpose::STANDARD
            .encode(signing_key().sign(payload.as_bytes()).to_bytes());
        serde_json::json!({ "payload": payload, "signature": signature, "key_version": 1 })
    }

    fn respond(status: u16, body: String) -> Router {
        Router::new()
            .route(
                "/api/v1/licenses/activate",
                post(move || {
                    let body = body.clone();
                    async move {
                        axum::http::Response::builder()
                            .status(status)
                            .header("Content-Type", "application/json")
                            .body(Body::from(body))
                            .unwrap()
                    }
                }),
            )
    }

    fn ok_body(token: Option<serde_json::Value>) -> String {
        serde_json::json!({ "success": true, "data": {
            "license_key": "CASH99-ABCDEF-2", "product_sku": "CASHELL", "plan_tier": "lifetime",
            "max_devices": 1, "expires_at": null, "activation_token": "legacy.hmac",
            "signed_token": token }})
        .to_string()
    }

    async fn run_activate(app: Router) -> Result<Activation, LicenseError> {
        let base = start_mock_server(app).await;
        std::thread::scope(|s| {
            s.spawn(move || activate_with_key(&base, "CASH99-ABCDEF-2", "dev-1", "t", "linux", test_key))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("spawned thread panicked".into())))
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activate_with_valid_signed_token_succeeds() {
        let act = run_activate(respond(200, ok_body(Some(signed(&["CASHELL"], "dev-1"))))).await;
        let act = act.expect("valid signed token must activate");
        assert_eq!(act.product, "CASHELL");
        assert!(act.token_expires_unix > chrono::Utc::now().timestamp());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activate_without_signed_token_fails_closed() {
        let act = run_activate(respond(200, ok_body(None))).await;
        assert!(matches!(act, Err(LicenseError::InvalidResponse(_))), "{act:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activate_with_other_product_token_fails() {
        let act = run_activate(respond(200, ok_body(Some(signed(&["CATERM"], "dev-1"))))).await;
        assert!(matches!(act, Err(LicenseError::InvalidResponse(_))), "{act:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activate_with_token_for_other_device_fails() {
        let act = run_activate(respond(200, ok_body(Some(signed(&["CASHELL"], "dev-2"))))).await;
        assert!(matches!(act, Err(LicenseError::InvalidResponse(_))), "{act:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn activate_maps_gcc_errors() {
        let not_found = r#"{"success":false,"error":{"message":"license tidak ditemukan"}}"#;
        assert!(matches!(run_activate(respond(404, not_found.into())).await, Err(LicenseError::NotFound)));
        let limit = r#"{"success":false,"error":{"message":"maximum device limit reached (max 1 devices)"}}"#;
        assert!(matches!(run_activate(respond(400, limit.into())).await, Err(LicenseError::DeviceLimit)));
        let expired = r#"{"success":false,"error":{"message":"license sudah kedaluwarsa"}}"#;
        assert!(matches!(run_activate(respond(400, expired.into())).await, Err(LicenseError::Expired)));
        let revoked = r#"{"success":false,"error":{"message":"license berstatus: revoked"}}"#;
        assert!(matches!(run_activate(respond(400, revoked.into())).await, Err(LicenseError::Revoked)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn verify_parses_envelope_and_token() {
        let body = serde_json::json!({ "success": true, "data": { "valid": true, "status": "active",
            "device_registered": true, "max_devices": 1, "signed_token": signed(&["CASHELL"], "dev-1") }})
        .to_string();
        let app = Router::new().route(
            "/api/v1/licenses/verify",
            post(move || {
                let body = body.clone();
                async move {
                    axum::http::Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(body))
                        .unwrap()
                }
            }),
        );
        let base = start_mock_server(app).await;
        let v = std::thread::scope(|s| {
            s.spawn(move || verify(&base, "CASH99-ABCDEF-2", "dev-1"))
                .join()
                .unwrap_or_else(|_| Err(LicenseError::Network("panicked".into())))
        })
        .expect("verify ok");
        assert!(v.valid && v.device_registered && v.signed_token.is_some());
    }

    #[test]
    fn network_failure_is_network_error() {
        let r = activate("http://127.0.0.1:9", "CASH99-ABCDEF-2", "d", "t", "linux");
        assert!(matches!(r, Err(LicenseError::Network(_))), "{r:?}");
    }
}
