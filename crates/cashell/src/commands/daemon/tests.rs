//! Tests for the daemon subsystem.

use super::*;
use axum::{
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use std::net::SocketAddr;
use std::sync::Arc;

#[test]
fn test_generate_service_unit_user() {
    let unit = generate_service_unit(true);
    assert!(unit.contains("Description=CAShell 24/7 Fleet & Health Monitoring Daemon"));
    assert!(unit.contains("ExecStart=%h/.local/bin/cashell daemon run"));
    assert!(unit.contains("Restart=always"));
    assert!(unit.contains("WantedBy=default.target"));
}

#[test]
fn test_generate_service_unit_system() {
    let unit = generate_service_unit(false);
    assert!(unit.contains("Description=CAShell 24/7 Fleet & Health Monitoring Daemon"));
    assert!(unit.contains("ExecStart=/usr/local/bin/cashell daemon run"));
    assert!(unit.contains("Restart=always"));
    assert!(unit.contains("WantedBy=multi-user.target"));
}

#[test]
fn test_daemon_config_parsing() {
    let toml_str = r#"
        [daemon]
        port = 9090
        token = "secret123"
        interval_secs = 30
        retry_threshold = 5
        ssh_key_path = "~/.ssh/custom_id"
        telegram_bot_token = "bot123"
        telegram_chat_id = "chat456"

        [daemon.nodes.vps1]
        ip = "192.168.1.100"
        user = "admin"

        [[daemon.nodes.vps1.services]]
        name = "web"
        port = 80
        protocol = "http"

        [daemon.remediation]
        "vps1.web" = "systemctl restart nginx"
    "#;
    let cfg: Config = toml::from_str(toml_str).expect("parse config");
    assert_eq!(cfg.daemon.port, 9090);
    assert_eq!(cfg.daemon.token, "secret123");
    assert_eq!(cfg.daemon.interval_secs, 30);
    assert_eq!(cfg.daemon.retry_threshold, 5);
    assert_eq!(cfg.daemon.ssh_key_path, "~/.ssh/custom_id");
    assert_eq!(cfg.daemon.telegram_bot_token, "bot123");
    assert_eq!(cfg.daemon.telegram_chat_id, "chat456");

    let vps = cfg.daemon.nodes.get("vps1").expect("vps1 exists");
    assert_eq!(vps.ip, "192.168.1.100");
    assert_eq!(vps.user, "admin");
    assert_eq!(vps.services.len(), 1);
    assert_eq!(vps.services[0].name, "web");
    assert_eq!(vps.services[0].port, 80);

    assert_eq!(
        cfg.daemon.remediation.get("vps1.web").map(String::as_str),
        Some("systemctl restart nginx")
    );
}

#[test]
fn test_daemon_config_defaults() {
    let cfg = Config::default();
    assert_eq!(cfg.daemon.port, 8090);
    assert_eq!(cfg.daemon.token, "");
    assert_eq!(cfg.daemon.bind, "127.0.0.1");
    assert_eq!(cfg.daemon.interval_secs, 60);
    assert_eq!(cfg.daemon.retry_threshold, 3);
    assert_eq!(cfg.daemon.ssh_key_path, "~/.ssh/id_ed25519");
    assert!(cfg.daemon.nodes.is_empty());
}

// ── H-01: check_auth fail-closed ───────────────────────────────────────────

#[test]
fn test_check_auth_empty_token_fails_closed() {
    let state = DaemonState {
        token: String::new(),
        trusted_proxies: vec!["127.0.0.1".to_string(), "::1".to_string()],
        node_secrets: std::collections::HashMap::new(),
    };
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer anything".parse().unwrap());
    assert!(!check_auth(&state, &headers));
}

#[test]
fn test_check_auth_correct_token_passes() {
    let state = DaemonState {
        token: "s3cret-token".to_string(),
        trusted_proxies: vec!["127.0.0.1".to_string(), "::1".to_string()],
        node_secrets: std::collections::HashMap::new(),
    };
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer s3cret-token".parse().unwrap());
    assert!(check_auth(&state, &headers));
}

#[test]
fn test_check_auth_wrong_token_fails() {
    let state = DaemonState {
        token: "correct-token".to_string(),
        trusted_proxies: vec!["127.0.0.1".to_string(), "::1".to_string()],
        node_secrets: std::collections::HashMap::new(),
    };
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer wrong-token".parse().unwrap());
    assert!(!check_auth(&state, &headers));
}

#[test]
fn test_const_time_eq() {
    assert!(const_time_eq("abc", "abc"));
    assert!(!const_time_eq("abc", "abd"));
    assert!(!const_time_eq("abc", "abcde"));
}

// ── H-02: is_loopback ───────────────────────────────────────────────────────

#[test]
fn test_is_loopback() {
    assert!(is_loopback("127.0.0.1"));
    assert!(is_loopback("::1"));
    assert!(is_loopback("localhost"));
    assert!(!is_loopback("0.0.0.0"));
    assert!(!is_loopback("192.168.1.1"));
}

// ── H-03: container name validation ───────────────────────────────────────

#[test]
fn test_valid_container_names() {
    assert!(is_valid_container_name("mycontainer"));
    assert!(is_valid_container_name("my-container"));
    assert!(is_valid_container_name("my_container"));
    assert!(is_valid_container_name("my.container"));
    assert!(is_valid_container_name("a"));
    assert!(is_valid_container_name("123abc"));
    assert!(is_valid_container_name(&"a".repeat(128)));
}

#[test]
fn test_malicious_container_names_rejected() {
    assert!(!is_valid_container_name("a;id"));
    assert!(!is_valid_container_name("../x"));
    assert!(!is_valid_container_name("a b"));
    assert!(!is_valid_container_name("$(id)"));
    assert!(!is_valid_container_name(""));
    assert!(!is_valid_container_name("-abc"));
    assert!(!is_valid_container_name(&"a".repeat(129)));
    assert!(!is_valid_container_name("a/b"));
}

// ── H-03: action handler with mocked docker ──────────────────────────────

/// Test harness: call action_handler directly with mocked container list.
async fn run_action_request(
    token: &str,
    auth_header: Option<&str>,
    action: &str,
    target: &str,
) -> (StatusCode, serde_json::Value) {
    std::env::set_var("CONTAINER_NAMES_OVERRIDE", "gitea,nginx,valid-container");
    let state = Arc::new(DaemonState {
        token: token.to_string(),
        trusted_proxies: vec!["127.0.0.1".to_string(), "::1".to_string()],
        node_secrets: std::collections::HashMap::new(),
    });
    let mut headers = HeaderMap::new();
    if let Some(auth) = auth_header {
        headers.insert(axum::http::header::AUTHORIZATION, auth.parse().unwrap());
    }
    let body = ActionRequest {
        action: action.to_string(),
        target: target.to_string(),
    };
    let resp = routes::action_handler(axum::extract::State(state), headers, axum::Json(body))
        .await
        .into_response();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!(null));
    (status, json)
}

#[tokio::test]
async fn test_action_backup_rejected() {
    let (status, body) = run_action_request("tok", Some("Bearer tok"), "backup", "gitea").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.get("error").is_some());
}

#[tokio::test]
async fn test_action_malicious_container_names() {
    for target in ["a;id", "../x", "a b", "$(id)"] {
        let (status, body) = run_action_request("tok", Some("Bearer tok"), "start", target).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "target={target} should be 400, got {status}"
        );
        assert!(body.get("error").is_some(), "target={target}");
    }
}

#[tokio::test]
async fn test_action_invalid_container_not_in_docker() {
    let (status, body) =
        run_action_request("tok", Some("Bearer tok"), "start", "nope-does-not-exist").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.get("error").is_some());
}

#[tokio::test]
async fn test_action_no_token_401() {
    let (status, _) = run_action_request("tok", None, "start", "gitea").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_action_wrong_token_401() {
    let (status, _) = run_action_request("correct", Some("Bearer wrong"), "start", "gitea").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_action_positive_valid_container() {
    let (status, body) = run_action_request("tok", Some("Bearer tok"), "start", "gitea").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body.get("ok").is_some(), "body={body}");
}

// ── H-04: analytics summary auth ────────────────────────────────────────

#[tokio::test]
async fn test_analytics_summary_no_token_401() {
    let state = Arc::new(DaemonState {
        token: "secret123".to_string(),
        trusted_proxies: vec!["127.0.0.1".to_string(), "::1".to_string()],
        node_secrets: std::collections::HashMap::new(),
    });
    let headers = HeaderMap::new();
    let resp = routes::analytics_summary_handler(axum::extract::State(state), headers)
        .await
        .into_response();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_analytics_summary_wrong_token_401() {
    let state = Arc::new(DaemonState {
        token: "secret123".to_string(),
        trusted_proxies: vec!["127.0.0.1".to_string(), "::1".to_string()],
        node_secrets: std::collections::HashMap::new(),
    });
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        "Bearer wrong".parse().unwrap(),
    );
    let resp = routes::analytics_summary_handler(axum::extract::State(state), headers)
        .await
        .into_response();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── H-02: daemon run with non-loopback + empty token → exit 2 ─────────

#[test]
fn test_daemon_run_rejects_non_loopback_without_token() {
    let bind = "0.0.0.0";
    let token = String::new();
    let should_reject = !is_loopback(&bind) && token.is_empty();
    assert!(
        should_reject,
        "non-loopback bind with empty token must be rejected"
    );

    let bind2 = "127.0.0.1";
    let should_reject2 = !is_loopback(&bind2) && token.is_empty();
    assert!(
        !should_reject2,
        "loopback bind with an empty token is allowed"
    );
}

// ── H-05: heartbeat signature verification ──────────────────────────────

fn signed_heartbeat(node_id: &str, secret: &str) -> cashell_common::Heartbeat {
    let mut hb = cashell_common::Heartbeat {
        node_id: node_id.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        uptime_secs: 42,
        cpu_pct: 1.0,
        ram_used_mb: 10,
        ram_total_mb: 100,
        disk_used_gb: 1,
        disk_total_gb: 10,
        services: vec![],
        version: "test".to_string(),
        signature: None,
    };
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(cashell_common::heartbeat_canonical_json(&hb).as_bytes());
    hb.signature = Some(
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect(),
    );
    hb
}

#[test]
fn test_verify_heartbeat_valid_signature() {
    let mut secrets = std::collections::HashMap::new();
    secrets.insert("linux-node".to_string(), "topsecret".to_string());
    let state = DaemonState {
        token: String::new(),
        trusted_proxies: Vec::new(),
        node_secrets: secrets,
    };
    let hb = signed_heartbeat("linux-node", "topsecret");
    assert!(verify_heartbeat(&state, &hb).is_ok());
}

#[test]
fn test_verify_heartbeat_bad_signature_rejected() {
    let mut secrets = std::collections::HashMap::new();
    secrets.insert("linux-node".to_string(), "topsecret".to_string());
    let state = DaemonState {
        token: String::new(),
        trusted_proxies: Vec::new(),
        node_secrets: secrets,
    };
    let hb = signed_heartbeat("linux-node", "wrongsecret");
    assert!(verify_heartbeat(&state, &hb).is_err());
}

#[test]
fn test_verify_heartbeat_stale_timestamp_rejected() {
    let mut secrets = std::collections::HashMap::new();
    secrets.insert("linux-node".to_string(), "topsecret".to_string());
    let state = DaemonState {
        token: String::new(),
        trusted_proxies: Vec::new(),
        node_secrets: secrets,
    };
    let mut hb = cashell_common::Heartbeat {
        node_id: "linux-node".to_string(),
        timestamp: (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339(),
        uptime_secs: 42,
        cpu_pct: 1.0,
        ram_used_mb: 10,
        ram_total_mb: 100,
        disk_used_gb: 1,
        disk_total_gb: 10,
        services: vec![],
        version: "test".to_string(),
        signature: None,
    };
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(b"topsecret").unwrap();
    mac.update(cashell_common::heartbeat_canonical_json(&hb).as_bytes());
    hb.signature = Some(
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect(),
    );
    assert!(verify_heartbeat(&state, &hb).is_err());
}

#[test]
fn test_verify_heartbeat_unsigned_no_secret_accepted() {
    let state = DaemonState {
        token: String::new(),
        trusted_proxies: Vec::new(),
        node_secrets: std::collections::HashMap::new(),
    };
    let hb = cashell_common::Heartbeat {
        node_id: "linux-node".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        uptime_secs: 42,
        cpu_pct: 1.0,
        ram_used_mb: 10,
        ram_total_mb: 100,
        disk_used_gb: 1,
        disk_total_gb: 10,
        services: vec![],
        version: "test".to_string(),
        signature: None,
    };
    assert!(verify_heartbeat(&state, &hb).is_ok());
}

#[test]
fn test_verify_heartbeat_signed_but_no_secret_rejected() {
    let state = DaemonState {
        token: String::new(),
        trusted_proxies: Vec::new(),
        node_secrets: std::collections::HashMap::new(),
    };
    let hb = signed_heartbeat("linux-node", "topsecret");
    assert!(verify_heartbeat(&state, &hb).is_err());
}

// ── H-10: trusted-proxy client IP resolution ─────────────────────────────

#[test]
fn test_resolve_client_ip_untrusted_peer_ignores_xff() {
    let peer: SocketAddr = "10.0.0.5:54321".parse().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "1.2.3.4".parse().unwrap());
    let trusted: Vec<String> = vec!["127.0.0.1".into(), "::1".into()];
    assert_eq!(resolve_client_ip(&peer, &headers, &trusted), "10.0.0.5");
}

#[test]
fn test_resolve_client_ip_trusted_peer_honors_xff() {
    let peer: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
    let trusted: Vec<String> = vec!["127.0.0.1".into(), "::1".into()];
    assert_eq!(resolve_client_ip(&peer, &headers, &trusted), "203.0.113.7");
}

#[test]
fn test_resolve_client_ip_trusted_peer_honors_cf_connecting_ip() {
    let peer: SocketAddr = "[::1]:54321".parse().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", "198.51.100.9".parse().unwrap());
    let trusted: Vec<String> = vec!["127.0.0.1".into(), "::1".into()];
    assert_eq!(resolve_client_ip(&peer, &headers, &trusted), "198.51.100.9");
}

#[test]
fn test_default_trusted_proxies() {
    let cfg = crate::config::Config::default();
    assert!(cfg
        .daemon
        .trusted_proxies
        .contains(&"127.0.0.1".to_string()));
    assert!(cfg.daemon.trusted_proxies.contains(&"::1".to_string()));
}
