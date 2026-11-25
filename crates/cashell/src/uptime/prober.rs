//! Unified Endpoint Probers for HTTP(S), TCP, and Ping/ICMP (U-01, U-03).

use std::time::{Duration, Instant};

use chrono::Utc;
use reqwest::Client;

use super::ssl::probe_ssl_expiry;
use super::types::{MonitorConfig, MonitorType, ProbeResult};

/// Probe a target endpoint defined by [`MonitorConfig`].
pub async fn probe_endpoint(config: &MonitorConfig) -> ProbeResult {
    match config.monitor_type {
        MonitorType::Http => probe_http(config).await,
        MonitorType::Tcp => probe_tcp(config).await,
        MonitorType::Ping => probe_ping(config).await,
    }
}

/// One HTTP client shared by every probe. Building a `reqwest::Client` loads
/// the root certificate store, so doing it per probe was the dominant CPU cost
/// of the uptime engine. Idle pooling is disabled so each probe still opens a
/// fresh connection (DNS + TCP + TLS) and the measured latency stays honest.
fn shared_probe_client() -> Result<&'static Client, String> {
    static CLIENT: std::sync::OnceLock<Result<Client, String>> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .danger_accept_invalid_certs(false)
                .pool_max_idle_per_host(0)
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// HTTP(S) prober verifying status code, latency, keyword assertion, and SSL validity (U-01, U-03).
pub async fn probe_http(config: &MonitorConfig) -> ProbeResult {
    let timeout = Duration::from_millis(config.timeout_ms.max(500));
    let client = match shared_probe_client() {
        Ok(c) => c,
        Err(e) => {
            return ProbeResult {
                monitor_name: config.name.clone(),
                target: config.target.clone(),
                is_up: false,
                latency_ms: None,
                status_code: None,
                error: Some(format!("Failed to build HTTP client: {e}")),
                cert_expires_at: None,
                cert_days_left: None,
            }
        }
    };

    let start = Instant::now();
    let res = client.get(&config.target).timeout(timeout).send().await;
    let latency_ms = start.elapsed().as_millis() as u64;

    match res {
        Ok(resp) => {
            let status = resp.status().as_u16();

            // Status check
            let status_ok = match config.expected_status {
                Some(exp) => status == exp,
                None => (200..=299).contains(&status),
            };

            if !status_ok {
                return ProbeResult {
                    monitor_name: config.name.clone(),
                    target: config.target.clone(),
                    is_up: false,
                    latency_ms: Some(latency_ms),
                    status_code: Some(status),
                    error: Some(format!("HTTP status {status} did not match expected")),
                    cert_expires_at: None,
                    cert_days_left: None,
                };
            }

            // Keyword assertion if configured
            if let Some(ref keyword) = config.expected_keyword {
                match resp.text().await {
                    Ok(body) => {
                        if !body.contains(keyword) {
                            return ProbeResult {
                                monitor_name: config.name.clone(),
                                target: config.target.clone(),
                                is_up: false,
                                latency_ms: Some(latency_ms),
                                status_code: Some(status),
                                error: Some(format!(
                                    "Expected keyword '{keyword}' missing in body"
                                )),
                                cert_expires_at: None,
                                cert_days_left: None,
                            };
                        }
                    }
                    Err(e) => {
                        return ProbeResult {
                            monitor_name: config.name.clone(),
                            target: config.target.clone(),
                            is_up: false,
                            latency_ms: Some(latency_ms),
                            status_code: Some(status),
                            error: Some(format!("Failed to read response body: {e}")),
                            cert_expires_at: None,
                            cert_days_left: None,
                        };
                    }
                }
            }

            // Optional SSL expiry probe if HTTPS
            let mut cert_expires_at = None;
            let mut cert_days_left = None;
            if config.target.starts_with("https://") {
                if let Ok(url) = reqwest::Url::parse(&config.target) {
                    if let Some(host) = url.host_str() {
                        let port = url.port().unwrap_or(443);
                        if let Ok(expiry) =
                            probe_ssl_expiry(host, port, Duration::from_millis(3000)).await
                        {
                            let now = Utc::now();
                            let days = super::ssl::days_until_expiry(&expiry, &now);
                            cert_expires_at = Some(expiry.to_rfc3339());
                            cert_days_left = Some(days);
                        }
                    }
                }
            }

            ProbeResult {
                monitor_name: config.name.clone(),
                target: config.target.clone(),
                is_up: true,
                latency_ms: Some(latency_ms),
                status_code: Some(status),
                error: None,
                cert_expires_at,
                cert_days_left,
            }
        }
        Err(e) => {
            let err_str = if e.is_timeout() {
                format!("Request timed out after {}ms", config.timeout_ms)
            } else if e.is_connect() {
                format!("Connection error: {e}")
            } else {
                format!("HTTP probe error: {e}")
            };

            ProbeResult {
                monitor_name: config.name.clone(),
                target: config.target.clone(),
                is_up: false,
                latency_ms: Some(latency_ms),
                status_code: None,
                error: Some(err_str),
                cert_expires_at: None,
                cert_days_left: None,
            }
        }
    }
}

/// TCP socket connection prober (U-01).
pub async fn probe_tcp(config: &MonitorConfig) -> ProbeResult {
    let timeout = Duration::from_millis(config.timeout_ms.max(500));
    let target = config.target.trim_start_matches("tcp://");

    let start = Instant::now();
    let conn = tokio::time::timeout(timeout, tokio::net::TcpStream::connect(target)).await;
    let latency_ms = start.elapsed().as_millis() as u64;

    match conn {
        Ok(Ok(_stream)) => ProbeResult {
            monitor_name: config.name.clone(),
            target: config.target.clone(),
            is_up: true,
            latency_ms: Some(latency_ms),
            status_code: None,
            error: None,
            cert_expires_at: None,
            cert_days_left: None,
        },
        Ok(Err(e)) => ProbeResult {
            monitor_name: config.name.clone(),
            target: config.target.clone(),
            is_up: false,
            latency_ms: Some(latency_ms),
            status_code: None,
            error: Some(format!("TCP connect error: {e}")),
            cert_expires_at: None,
            cert_days_left: None,
        },
        Err(_) => ProbeResult {
            monitor_name: config.name.clone(),
            target: config.target.clone(),
            is_up: false,
            latency_ms: Some(latency_ms),
            status_code: None,
            error: Some(format!(
                "TCP connect timed out after {}ms",
                config.timeout_ms
            )),
            cert_expires_at: None,
            cert_days_left: None,
        },
    }
}

/// Ping / ICMP prober with fallback to TCP connect (U-01).
pub async fn probe_ping(config: &MonitorConfig) -> ProbeResult {
    // If target specifies port or IP, attempt socket connect
    if config.target.contains(':') {
        return probe_tcp(config).await;
    }

    // Try default port 80 or 443
    let target_with_port = format!("{}:80", config.target);
    let mut cfg = config.clone();
    cfg.target = target_with_port;
    probe_tcp(&cfg).await
}
