//! Webhook receiver for automatic Git deployments with HMAC verification (D-06).
//!
//! Supports:
//! - GitHub: `X-Hub-Signature-256: sha256=<hex>`
//! - Gitea: `X-Gitea-Signature: <hex>`
//! - GitLab: `X-Gitlab-Token: <secret>`

use crate::commands::daemon::auth::const_time_eq;
use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// Verify HMAC-SHA256 webhook signature in constant time (D-06).
pub fn verify_webhook_signature(headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
    if secret.trim().is_empty() {
        return false;
    }

    // 1. Check GitHub / Gitea `X-Hub-Signature-256`
    if let Some(sig_header) = headers.get("x-hub-signature-256") {
        if let Ok(sig_str) = sig_header.to_str() {
            let provided_hex = sig_str.strip_prefix("sha256=").unwrap_or(sig_str);
            if let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
                mac.update(body);
                let expected_bytes = mac.finalize().into_bytes();
                let expected_hex = hex_encode(&expected_bytes);
                if const_time_eq(provided_hex.trim(), &expected_hex) {
                    return true;
                }
            }
        }
    }

    // 2. Check Gitea `X-Gitea-Signature`
    if let Some(sig_header) = headers.get("x-gitea-signature") {
        if let Ok(sig_str) = sig_header.to_str() {
            if let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
                mac.update(body);
                let expected_bytes = mac.finalize().into_bytes();
                let expected_hex = hex_encode(&expected_bytes);
                if const_time_eq(sig_str.trim(), &expected_hex) {
                    return true;
                }
            }
        }
    }

    // 3. Check GitLab `X-Gitlab-Token`
    if let Some(token_header) = headers.get("x-gitlab-token") {
        if let Ok(token_str) = token_header.to_str() {
            if const_time_eq(token_str.trim(), secret.trim()) {
                return true;
            }
        }
    }

    false
}

/// Extract commit SHA from git webhook payload body (GitHub, GitLab, Gitea).
pub fn extract_commit_from_payload(body: &[u8]) -> Option<String> {
    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
        // GitHub / Gitea: "after"
        if let Some(after) = val.get("after").and_then(|v| v.as_str()) {
            if !after.chars().all(|c| c == '0') && !after.is_empty() {
                return Some(after.to_string());
            }
        }
        // GitLab: "checkout_sha"
        if let Some(sha) = val.get("checkout_sha").and_then(|v| v.as_str()) {
            if !sha.is_empty() {
                return Some(sha.to_string());
            }
        }
        // head_commit.id
        if let Some(sha) = val
            .get("head_commit")
            .and_then(|h| h.get("id"))
            .and_then(|v| v.as_str())
        {
            if !sha.is_empty() {
                return Some(sha.to_string());
            }
        }
    }
    None
}

/// Helper to format byte slice as lower-case hex string.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn test_d06_webhook_hmac_verification_and_rejection() {
        let secret = "super-secret-deploy-token-999";
        let payload = br#"{"ref":"refs/heads/main","after":"c0ffee1234567890"}"#;

        // 1. Generate valid GitHub signature
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let valid_hex = hex_encode(&mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-hub-signature-256",
            HeaderValue::from_str(&format!("sha256={valid_hex}")).unwrap(),
        );

        // Valid signature -> passes
        assert!(verify_webhook_signature(&headers, payload, secret));

        // Invalid signature -> rejected (401 simulation)
        let mut bad_headers = HeaderMap::new();
        bad_headers.insert(
            "x-hub-signature-256",
            HeaderValue::from_static("sha256=deadbeefdeadbeefdeadbeef"),
        );
        assert!(!verify_webhook_signature(&bad_headers, payload, secret));

        // Empty / missing signature -> rejected
        assert!(!verify_webhook_signature(
            &HeaderMap::new(),
            payload,
            secret
        ));

        // Test GitLab token
        let mut gl_headers = HeaderMap::new();
        gl_headers.insert("x-gitlab-token", HeaderValue::from_str(secret).unwrap());
        assert!(verify_webhook_signature(&gl_headers, payload, secret));

        // Test commit extraction
        let commit = extract_commit_from_payload(payload);
        assert_eq!(commit.as_deref(), Some("c0ffee1234567890"));
    }
}
