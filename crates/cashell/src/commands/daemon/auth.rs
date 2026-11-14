//! Authentication, constant-time validation, and HMAC signature checks for CAShell daemon (H-01..H-05, H-10).

use axum::http::HeaderMap;
use std::collections::HashMap;
use std::net::SocketAddr;

/// HTTP API shared state.
#[derive(Clone, Debug)]
pub struct DaemonState {
    pub token: String,
    /// H-10: peer addresses trusted to supply `X-Forwarded-For` / `CF-Connecting-IP` headers.
    pub trusted_proxies: Vec<String>,
    /// H-05: per-node secrets for verifying signed heartbeats, keyed by node_id.
    pub node_secrets: HashMap<String, String>,
}

/// Constant-time comparison of two strings (hard security rule for bearer tokens).
pub fn const_time_eq(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    if x.len() != y.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (ai, bi) in x.iter().zip(y.iter()) {
        diff |= ai ^ bi;
    }
    diff == 0
}

/// Verify bearer token against DaemonState.
pub fn check_auth(state: &DaemonState, headers: &HeaderMap) -> bool {
    if state.token.is_empty() {
        // H-01: empty configured token always fails closed.
        return false;
    }
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| const_time_eq(t, &state.token))
        .unwrap_or(false)
}

/// H-05: hex-encode arbitrary bytes (used for HMAC digest → hex signature).
pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// H-05: verify a signed heartbeat against the per-node secret.
pub fn verify_heartbeat(
    state: &DaemonState,
    hb: &cashell_common::Heartbeat,
) -> std::result::Result<(), String> {
    let secret = state.node_secrets.get(&hb.node_id);

    let Some(provided) = hb.signature.as_ref() else {
        if secret.is_some() {
            eprintln!(
                "⚠ [H-05] heartbeat from node '{}' has no signature although a secret is configured; accepting (legacy agent).",
                hb.node_id
            );
        }
        return Ok(());
    };

    let Some(secret) = secret else {
        return Err(format!(
            "signature provided but no secret configured for node '{}'",
            hb.node_id
        ));
    };

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let expected = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(mut mac) => {
            mac.update(cashell_common::heartbeat_canonical_json(hb).as_bytes());
            hex_encode(&mac.finalize().into_bytes())
        }
        Err(e) => {
            return Err(format!("hmac init failed: {e}"));
        }
    };
    if !const_time_eq(provided, &expected) {
        return Err("signature mismatch".to_string());
    }

    let ts = chrono::DateTime::parse_from_rfc3339(&hb.timestamp)
        .map_err(|e| format!("invalid timestamp '{}': {}", hb.timestamp, e))?
        .to_utc();
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(ts);
    if delta.abs() > chrono::Duration::seconds(120) {
        return Err(format!(
            "timestamp '{}' is {}s outside the 120 s freshness window",
            hb.timestamp,
            delta.num_seconds()
        ));
    }

    Ok(())
}

/// H-10: resolve the effective client IP for an analytics event.
pub fn resolve_client_ip(
    peer: &SocketAddr,
    headers: &HeaderMap,
    trusted_proxies: &[String],
) -> String {
    let peer_ip_str = peer.ip().to_string();
    let is_trusted = trusted_proxies
        .iter()
        .any(|t| t == &peer_ip_str || t == &peer.to_string());
    if !is_trusted {
        return peer_ip_str;
    }
    if let Some(v) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return v.to_string();
    }
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = v.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    peer_ip_str
}
