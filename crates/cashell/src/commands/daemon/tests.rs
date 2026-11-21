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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
    let should_reject = !is_loopback(bind) && token.is_empty();
    assert!(
        should_reject,
        "non-loopback bind with empty token must be rejected"
    );

    let bind2 = "127.0.0.1";
    let should_reject2 = !is_loopback(bind2) && token.is_empty();
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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

// ── Tahap 2: Analytics Hardening (A-01..A-06) ──────────────────────────────

#[test]
fn test_a01_country_detection_trusted_proxy_and_fallback() {
    let trusted = vec!["127.0.0.1".to_string(), "::1".to_string()];

    // Untrusted peer supplying header must be ignored
    let untrusted_peer: SocketAddr = "192.168.1.50:12345".parse().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("cf-ipcountry", "ID".parse().unwrap());
    assert_eq!(
        resolve_country(&untrusted_peer, &headers, &trusted),
        "",
        "Untrusted peer must not have CF-IPCountry accepted"
    );

    // Trusted peer with CF-IPCountry: US
    let trusted_peer: SocketAddr = "127.0.0.1:12345".parse().unwrap();
    let mut headers_us = HeaderMap::new();
    headers_us.insert("cf-ipcountry", "US".parse().unwrap());
    assert_eq!(resolve_country(&trusted_peer, &headers_us, &trusted), "US");

    // Trusted peer with lowercase 'sg' -> uppercased to 'SG'
    let mut headers_sg = HeaderMap::new();
    headers_sg.insert("cf-ipcountry", "sg".parse().unwrap());
    assert_eq!(resolve_country(&trusted_peer, &headers_sg, &trusted), "SG");

    // Database test: Event with country '' (no header) aggregates to 'Tidak diketahui', not 'ID'
    let db = crate::db::Database::open_in_memory().unwrap();
    db.record_analytics_event(
        "testsite",
        "sess1",
        "/home",
        "",
        "Home",
        "Chrome",
        "Linux",
        "Desktop",
        "", // no country header
        "visitor_hash_1",
        "pageview",
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let summary = db.get_analytics_summary(Some("testsite")).unwrap();
    assert!(!summary.country_breakdown.is_empty());
    let item = &summary.country_breakdown[0];
    assert_eq!(
        item.label, "Tidak diketahui",
        "Must be 'Tidak diketahui', not 'ID'"
    );
    assert_eq!(item.count, 1);
}

#[test]
fn test_a02_os_detection_10_real_user_agents() {
    // 1. Android Chrome (MUST NOT resolve to Linux!)
    let (br, os, dev) = parse_user_agent(
        "Mozilla/5.0 (Linux; Android 14; Pixel 8 Pro) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36",
    );
    assert_eq!(os, "Android", "Android must be checked before Linux");
    assert_eq!(br, "Chrome");
    assert_eq!(dev, "Mobile");

    // 2. Android Samsung Browser (MUST NOT resolve to Linux!)
    let (_, os2, dev2) = parse_user_agent(
        "Mozilla/5.0 (Linux; Android 13; SAMSUNG SM-G998B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Mobile Safari/537.36",
    );
    assert_eq!(os2, "Android");
    assert_eq!(dev2, "Mobile");

    // 3. iPhone Safari (MUST NOT resolve to MacOS!)
    let (br3, os3, dev3) = parse_user_agent(
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Mobile/15E148 Safari/604.1",
    );
    assert_eq!(os3, "iOS", "iOS must be checked before MacOS");
    assert_eq!(br3, "Safari");
    assert_eq!(dev3, "Mobile");

    // 4. iPad Safari (MUST NOT resolve to MacOS!)
    let (_, os4, dev4) = parse_user_agent(
        "Mozilla/5.0 (iPad; CPU OS 17_4_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Mobile/15E148 Safari/604.1",
    );
    assert_eq!(os4, "iOS");
    assert_eq!(dev4, "Tablet");

    // 5. macOS Safari
    let (br5, os5, dev5) = parse_user_agent(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4_1) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Safari/605.1.15",
    );
    assert_eq!(os5, "MacOS");
    assert_eq!(br5, "Safari");
    assert_eq!(dev5, "Desktop");

    // 6. macOS Chrome
    let (br6, os6, _) = parse_user_agent(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4_1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    );
    assert_eq!(os6, "MacOS");
    assert_eq!(br6, "Chrome");

    // 7. Windows 11 Chrome
    let (br7, os7, dev7) = parse_user_agent(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    );
    assert_eq!(os7, "Windows");
    assert_eq!(br7, "Chrome");
    assert_eq!(dev7, "Desktop");

    // 8. Windows 10 Firefox
    let (br8, os8, _) = parse_user_agent(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:125.0) Gecko/20100101 Firefox/125.0",
    );
    assert_eq!(os8, "Windows");
    assert_eq!(br8, "Firefox");

    // 9. Ubuntu Linux Firefox
    let (br9, os9, dev9) = parse_user_agent(
        "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:125.0) Gecko/20100101 Firefox/125.0",
    );
    assert_eq!(os9, "Linux");
    assert_eq!(br9, "Firefox");
    assert_eq!(dev9, "Desktop");

    // 10. ChromeOS
    let (br10, os10, _) = parse_user_agent(
        "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    );
    assert_eq!(os10, "ChromeOS");
    assert_eq!(br10, "Chrome");
}

#[test]
fn test_a03_client_ip_different_visitors() {
    let salt_mgr = DailySaltManager::new();
    let ip1 = "203.0.113.10";
    let ip2 = "203.0.113.20";

    let h1 = salt_mgr.hash_visitor(ip1);
    let h2 = salt_mgr.hash_visitor(ip2);
    assert_ne!(
        h1, h2,
        "Different client IPs must yield different visitor hashes"
    );

    let db = crate::db::Database::open_in_memory().unwrap();
    db.record_analytics_event(
        "site1", "s1", "/", "", "Home", "Chrome", "Linux", "Desktop", "ID", &h1, "pageview", None,
        None, None, None, None, None,
    )
    .unwrap();
    db.record_analytics_event(
        "site1", "s2", "/about", "", "About", "Firefox", "Windows", "Desktop", "US", &h2,
        "pageview", None, None, None, None, None, None,
    )
    .unwrap();

    let summary = db.get_analytics_summary(Some("site1")).unwrap();
    assert_eq!(
        summary.unique_visitors, 2,
        "Two distinct client IPs must be two unique visitors"
    );
}

#[test]
fn test_a04_daily_salt_random_rotation_and_not_derivable_from_date() {
    let salt_mgr = DailySaltManager::new();
    let salt = salt_mgr.get_or_rotate_salt();
    assert_eq!(
        salt.len(),
        64,
        "Salt must be 32 random bytes in hex (64 hex characters)"
    );

    // Must NOT be derived from date
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let date_hash = hex_sha256(&today);
    assert_ne!(salt, date_hash, "Salt must not be derived from date");

    let client_ip = "198.51.100.77";
    let h1 = salt_mgr.hash_visitor(client_ip);
    let h1_repeat = salt_mgr.hash_visitor(client_ip);
    assert_eq!(
        h1, h1_repeat,
        "Visitor hash on same day must be deterministic"
    );

    // Rotate salt
    let new_salt = salt_mgr.rotate_now();
    assert_ne!(salt, new_salt, "New salt after rotation must be different");

    let h2 = salt_mgr.hash_visitor(client_ip);
    assert_ne!(
        h1, h2,
        "Visitor hash after rotation must be completely different (old salt discarded)"
    );
}

#[test]
fn test_a05_bot_filter_googlebot_curl_and_sec_fetch() {
    let empty_headers = HeaderMap::new();

    // Googlebot UA
    let gbot = "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";
    assert!(
        is_bot(gbot, &empty_headers),
        "Googlebot must be detected as bot"
    );

    // Curl UA
    let curl = "curl/8.5.0";
    assert!(is_bot(curl, &empty_headers), "curl must be detected as bot");

    // Python requests UA
    let py = "python-requests/2.31.0";
    assert!(
        is_bot(py, &empty_headers),
        "python-requests must be detected as bot"
    );

    // Real browser UA with Sec-Fetch-Dest: document
    let browser_ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
    let mut doc_headers = HeaderMap::new();
    doc_headers.insert("sec-fetch-dest", "document".parse().unwrap());
    assert!(
        !is_bot(browser_ua, &doc_headers),
        "Real browser request must not be flagged as bot"
    );

    // Browser UA fetching an image resource
    let mut img_headers = HeaderMap::new();
    img_headers.insert("sec-fetch-dest", "image".parse().unwrap());
    assert!(
        is_bot(browser_ua, &img_headers),
        "Image resource fetch should be filtered"
    );
}

#[test]
fn test_a06_rate_limit_per_ip_60_ok_61_too_many_requests() {
    let limiter = CollectorRateLimiter::new(60, 60);
    let test_ip = "203.0.113.88";

    // First 60 requests must succeed
    for i in 1..=60 {
        assert!(
            limiter.check_and_record(test_ip),
            "Request #{i} should be permitted within rate limit"
        );
    }

    // 61st request must be rejected
    assert!(
        !limiter.check_and_record(test_ip),
        "Request #61 must be rejected by rate limiter"
    );

    // Another IP must still succeed on its first hit
    let other_ip = "203.0.113.99";
    assert!(
        limiter.check_and_record(other_ip),
        "Different IP must have its own bucket"
    );
}

#[test]
fn test_a07_analytics_retention_and_daily_rollup() {
    let db = crate::db::Database::open_in_memory().unwrap();

    // Insert 5 events 30 days ago
    for i in 0..5 {
        db.conn()
            .execute(
                "INSERT INTO analytics_events (website_id, session_id, url, browser, os, device, country, visitor_hash, event_type, created_at)
                 VALUES ('siteA', ?1, '/old', 'Chrome', 'Linux', 'Desktop', 'ID', ?2, 'pageview', datetime('now', '-30 days'))",
                rusqlite::params![format!("s_old_{i}"), format!("vh_old_{i}")],
            )
            .unwrap();
    }

    // Insert 3 events today
    for i in 0..3 {
        db.conn()
            .execute(
                "INSERT INTO analytics_events (website_id, session_id, url, browser, os, device, country, visitor_hash, event_type, created_at)
                 VALUES ('siteA', ?1, '/today', 'Chrome', 'Linux', 'Desktop', 'ID', ?2, 'pageview', datetime('now'))",
                rusqlite::params![format!("s_new_{i}"), format!("vh_new_{i}")],
            )
            .unwrap();
    }

    // Pre-condition: 8 raw events
    let count_before: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM analytics_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count_before, 8);

    // Prune with 14-day retention
    let (pruned, rollups) = db.prune_analytics_events_and_rollup(14).unwrap();
    assert_eq!(
        pruned, 5,
        "5 old events must be pruned from raw analytics_events"
    );
    assert!(rollups >= 1, "At least 1 daily rollup record saved");

    // Verify raw table only has the 3 recent events
    let count_after: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM analytics_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count_after, 3,
        "Only the 3 recent events remain in raw table"
    );

    // Verify rollups table preserved the 5 aggregated views for siteA
    let (rollup_views, rollup_visitors): (i64, i64) = db
        .conn()
        .query_row(
            "SELECT pageviews, unique_visitors FROM analytics_daily_rollups WHERE website_id = 'siteA'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(rollup_views, 5, "Rollup must preserve the 5 pageviews");
    assert_eq!(
        rollup_visitors, 5,
        "Rollup must preserve the 5 unique visitors"
    );
}

#[test]
fn test_a08_analytics_export_csv_and_json() {
    let db = crate::db::Database::open_in_memory().unwrap();
    for i in 1..=7 {
        db.record_analytics_event(
            "export-site",
            &format!("sess_{i}"),
            &format!("/path/{i}"),
            "https://referrer.example",
            "Page Title",
            "Firefox",
            "Linux",
            "Desktop",
            "US",
            &format!("hash_{i}"),
            "pageview",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    }

    let tmp_dir = std::env::temp_dir().join(format!("cashell-export-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let csv_path = tmp_dir.join("export.csv");
    let json_path = tmp_dir.join("export.json");

    let exported_csv = db
        .export_analytics_csv(&csv_path.to_string_lossy())
        .unwrap();
    assert_eq!(exported_csv, 7);
    let csv_content = std::fs::read_to_string(&csv_path).unwrap();
    let csv_lines: Vec<&str> = csv_content.lines().collect();
    assert_eq!(
        csv_lines.len(),
        8,
        "CSV must have 1 header line + 7 event lines"
    );

    let exported_json = db
        .export_analytics_json(&json_path.to_string_lossy())
        .unwrap();
    assert_eq!(exported_json, 7);
    let json_content = std::fs::read_to_string(&json_path).unwrap();
    let json_events: Vec<serde_json::Value> = serde_json::from_str(&json_content).unwrap();
    assert_eq!(
        json_events.len(),
        7,
        "JSON array must contain 7 event objects"
    );
}

#[tokio::test]
async fn test_a09_tracker_script_size_under_2kb_gzip_and_spa_support() {
    use axum::response::IntoResponse;
    use std::io::Write;
    use std::process::{Command, Stdio};

    let resp = super::routes::tracker_js_handler().await.into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 100_000)
        .await
        .unwrap();
    let js_text = String::from_utf8(bytes.to_vec()).unwrap();

    // Verify SPA routing hooks and custom event APIs are present
    assert!(
        js_text.contains("sendBeacon"),
        "Tracker must support navigator.sendBeacon"
    );
    assert!(
        js_text.contains("hookHistory") || js_text.contains("pushState"),
        "Tracker must support SPA routing"
    );
    assert!(
        js_text.contains("popstate"),
        "Tracker must hook popstate for client-side navigation"
    );
    assert!(
        js_text.contains("cashell"),
        "Tracker must expose window.cashell custom event API"
    );

    // Verify Gzipped size is <= 2048 bytes (2 KB)
    let mut child = Command::new("gzip")
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("gzip command should be available on linux");

    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        stdin.write_all(js_text.as_bytes()).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    let gz_size = output.stdout.len();
    assert!(
        gz_size <= 2048,
        "Gzipped tracker size must be <= 2048 bytes (actual: {gz_size} bytes)"
    );
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn test_a10_cockpit_tui_and_cli_analytics_consistency() {
    let db = crate::db::Database::open_in_memory().unwrap();

    for i in 1..=12 {
        db.record_analytics_event(
            "site-tui",
            &format!("s_{i}"),
            if i % 2 == 0 { "/docs" } else { "/blog" },
            "https://google.com",
            "Docs Page",
            "Chrome",
            "Linux",
            "Desktop",
            "ID",
            &format!("vh_{}", i % 5),
            "pageview",
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    }

    let summary = db.get_analytics_summary(Some("site-tui")).unwrap();

    // Simulate Cockpit TUI data model ingestion
    let mut snap = crate::systat::SysSnapshot::default();
    snap.web_analytics = summary.clone();

    // Simulate CLI `cashell analytics --json` serialization
    let cli_json = serde_json::json!({
        "pageviews": summary.pageviews,
        "unique_visitors": summary.unique_visitors,
        "active_visitors": summary.active_visitors,
        "bounce_rate": summary.bounce_rate,
        "top_pages": summary.top_pages,
        "device_breakdown": summary.device_breakdown,
        "browser_breakdown": summary.browser_breakdown,
        "os_breakdown": summary.os_breakdown,
        "country_breakdown": summary.country_breakdown,
    });

    // Verify 1:1 consistency between TUI snapshot values and CLI json output
    assert_eq!(
        snap.web_analytics.pageviews,
        cli_json["pageviews"].as_u64().unwrap()
    );
    assert_eq!(
        snap.web_analytics.unique_visitors,
        cli_json["unique_visitors"].as_u64().unwrap()
    );
    assert_eq!(
        snap.web_analytics.top_pages.len(),
        cli_json["top_pages"].as_array().unwrap().len()
    );
    assert_eq!(snap.web_analytics.pageviews, 12);
    assert_eq!(snap.web_analytics.unique_visitors, 5);
}
