//! Authentication, constant-time validation, and HMAC signature checks for CAShell daemon (H-01..H-05, H-10).

use axum::http::HeaderMap;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// HTTP API shared state.
#[derive(Clone, Debug)]
pub struct DaemonState {
    pub token: String,
    /// H-10: peer addresses trusted to supply `X-Forwarded-For` / `CF-Connecting-IP` headers.
    pub trusted_proxies: Vec<String>,
    /// H-05: per-node secrets for verifying signed heartbeats, keyed by node_id.
    pub node_secrets: HashMap<String, String>,
    /// A-04: Daily random secret salt for visitor hashing.
    pub salt_manager: DailySaltManager,
    /// A-06: Collector rate limiter per client IP.
    pub rate_limiter: CollectorRateLimiter,
    /// System configuration for app lookups (D-02, D-06).
    pub cfg: crate::config::Config,
    /// Sequential build queue for webhooks (D-05, D-06).
    pub build_queue: Option<Arc<crate::deploy::BuildQueue>>,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            token: String::new(),
            trusted_proxies: Vec::new(),
            node_secrets: HashMap::new(),
            salt_manager: DailySaltManager::new(),
            rate_limiter: CollectorRateLimiter::new(60, 60),
            cfg: crate::config::Config::default(),
            build_queue: None,
        }
    }
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

/// A-04: Daily random secret salt (not derivable from date), discarded after rotation.
#[derive(Clone, Debug)]
pub struct DailySaltManager {
    inner: Arc<Mutex<DailySaltInner>>,
}

#[derive(Debug)]
struct DailySaltInner {
    day: String,
    salt: String,
}

impl Default for DailySaltManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DailySaltManager {
    pub fn new() -> Self {
        let (day, salt) = Self::generate_new_salt();
        Self {
            inner: Arc::new(Mutex::new(DailySaltInner { day, salt })),
        }
    }

    fn generate_new_salt() -> (String, String) {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let salt = hex_encode(&bytes);
        let day = chrono::Local::now().format("%Y-%m-%d").to_string();
        (day, salt)
    }

    pub fn get_or_rotate_salt(&self) -> String {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut inner = self.inner.lock().unwrap();
        if inner.day != today {
            let (_, new_salt) = Self::generate_new_salt();
            inner.day = today;
            inner.salt = new_salt;
        }
        inner.salt.clone()
    }

    pub fn rotate_now(&self) -> String {
        let mut inner = self.inner.lock().unwrap();
        let (_, new_salt) = Self::generate_new_salt();
        inner.salt = new_salt.clone();
        new_salt
    }

    pub fn hash_visitor(&self, client_ip: &str) -> String {
        let salt = self.get_or_rotate_salt();
        let input = format!("{client_ip}_{salt}");
        crate::commands::daemon::routes::hex_sha256(&input)
    }
}

/// A-06: In-memory sliding-window rate limiter per client IP.
#[derive(Clone, Debug)]
pub struct CollectorRateLimiter {
    inner: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    limit: usize,
    window_secs: u64,
}

impl Default for CollectorRateLimiter {
    fn default() -> Self {
        Self::new(60, 60)
    }
}

impl CollectorRateLimiter {
    pub fn new(limit: usize, window_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            limit,
            window_secs,
        }
    }

    /// Check and record an incoming request hit from `ip`.
    /// Returns `true` if allowed, `false` if rate limit exceeded.
    pub fn check_and_record(&self, ip: &str) -> bool {
        let now = Instant::now();
        let cutoff = now
            .checked_sub(std::time::Duration::from_secs(self.window_secs))
            .unwrap_or(now);
        let mut map = self.inner.lock().unwrap();
        let hits = map.entry(ip.to_string()).or_default();
        hits.retain(|&t| t > cutoff);
        if hits.len() >= self.limit {
            false
        } else {
            hits.push(now);
            true
        }
    }
}

/// A-01: resolve country from CF-IPCountry only if request is from a trusted proxy.
pub fn resolve_country(
    peer: &SocketAddr,
    headers: &HeaderMap,
    trusted_proxies: &[String],
) -> String {
    let peer_ip_str = peer.ip().to_string();
    let is_trusted = trusted_proxies
        .iter()
        .any(|t| t == &peer_ip_str || t == &peer.to_string());
    if !is_trusted {
        return String::new();
    }
    if let Some(v) = headers
        .get("cf-ipcountry")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let code = v.to_uppercase();
        if code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()) {
            return code;
        }
    }
    String::new()
}
