//! Comprehensive tests for Uptime Engine (U-01..U-07).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use axum::Router;

use super::heartbeat::send_heartbeat_ping;
use super::prober::probe_endpoint;
use super::ssl::{days_until_expiry, evaluate_ssl_alert_tier, parse_asn1_time_str};
use super::status_page::render_status_page;
use super::types::{
    IncidentStateMachine, MonitorConfig, MonitorState, MonitorStatusSummary, StateTransition,
};
use crate::config::Config;
use crate::db::Database;

#[tokio::test]
async fn test_u01_http_prober_200_500_timeout_keyword() {
    // Spin up mock HTTP test server
    let app = Router::new()
        .route(
            "/ok",
            get(|| async { (axum::http::StatusCode::OK, "Service operational") }),
        )
        .route(
            "/500",
            get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Fatal crash") }),
        )
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                (axum::http::StatusCode::OK, "Finally responded")
            }),
        )
        .route(
            "/keyword",
            get(|| async {
                (
                    axum::http::StatusCode::OK,
                    "Welcome to Sovereign Cloud Engine",
                )
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // 1. Success 200
    let m_ok = MonitorConfig::new_http("TestOK", format!("http://{addr}/ok"));
    let res_ok = probe_endpoint(&m_ok).await;
    assert!(res_ok.is_up, "Expected UP for 200 route");
    assert_eq!(res_ok.status_code, Some(200));
    assert!(res_ok.error.is_none());

    // 2. HTTP 500 failure
    let m_500 = MonitorConfig::new_http("Test500", format!("http://{addr}/500"));
    let res_500 = probe_endpoint(&m_500).await;
    assert!(!res_500.is_up, "Expected DOWN for 500 route");
    assert_eq!(res_500.status_code, Some(500));
    assert!(res_500.error.unwrap().contains("did not match expected"));

    // 3. Timeout failure (timeout_ms: 100, route takes 500ms)
    let mut m_slow = MonitorConfig::new_http("TestSlow", format!("http://{addr}/slow"));
    m_slow.timeout_ms = 100;
    let res_slow = probe_endpoint(&m_slow).await;
    assert!(!res_slow.is_up, "Expected DOWN for timeout");
    assert!(res_slow.error.unwrap().contains("timed out"));

    // 4. Keyword match success
    let mut m_kw_ok = MonitorConfig::new_http("TestKW_OK", format!("http://{addr}/keyword"));
    m_kw_ok.expected_keyword = Some("Sovereign".to_string());
    let res_kw_ok = probe_endpoint(&m_kw_ok).await;
    assert!(res_kw_ok.is_up, "Expected UP when keyword matches");

    // 5. Keyword mismatch failure
    let mut m_kw_fail = MonitorConfig::new_http("TestKW_Fail", format!("http://{addr}/keyword"));
    m_kw_fail.expected_keyword = Some("NonExistentStringXYZ".to_string());
    let res_kw_fail = probe_endpoint(&m_kw_fail).await;
    assert!(!res_kw_fail.is_up, "Expected DOWN when keyword missing");
    assert!(res_kw_fail.error.unwrap().contains("missing in body"));
}

#[tokio::test]
async fn test_u01_tcp_prober() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");

    // 1. Successful TCP connect
    let m_tcp_ok = MonitorConfig::new_tcp("TCP_OK", addr.to_string());
    let res_ok = probe_endpoint(&m_tcp_ok).await;
    assert!(res_ok.is_up);
    assert!(res_ok.latency_ms.is_some());

    // 2. Closed port failure
    drop(listener);
    let m_tcp_fail = MonitorConfig::new_tcp("TCP_Fail", addr.to_string());
    let res_fail = probe_endpoint(&m_tcp_fail).await;
    assert!(!res_fail.is_up);
}

#[test]
fn test_u02_incident_state_machine_and_anti_flapping() {
    let mut sm = IncidentStateMachine::new("PaymentGateway", 3);
    assert_eq!(sm.state, MonitorState::Up);

    // 1st failure: enters Degraded
    let t1 = sm.process_probe(false, Some("Connection refused"));
    assert_eq!(t1, None);
    assert_eq!(sm.state, MonitorState::Degraded);
    assert_eq!(sm.consecutive_failures, 1);

    // 2nd failure: remains Degraded
    let t2 = sm.process_probe(false, Some("Connection refused"));
    assert_eq!(t2, None);
    assert_eq!(sm.state, MonitorState::Degraded);
    assert_eq!(sm.consecutive_failures, 2);

    // 3rd failure (hits threshold 3): transitions to Down, emits IncidentStarted
    let t3 = sm.process_probe(false, Some("Connection refused"));
    assert_eq!(
        t3,
        Some(StateTransition::IncidentStarted {
            cause: "Connection refused".to_string()
        })
    );
    assert_eq!(sm.state, MonitorState::Down);
    assert_eq!(sm.consecutive_failures, 3);

    // Attach incident ID
    sm.open_incident_id = Some(42);

    // Anti-flapping test: 1st success while Down does NOT resolve incident yet!
    let t4 = sm.process_probe(true, None);
    assert_eq!(
        t4, None,
        "1 success should not resolve outage due to anti-flapping"
    );
    assert_eq!(sm.state, MonitorState::Down);
    assert_eq!(sm.consecutive_successes, 1);

    // Flapping occurs: a failure immediately resets success count
    let t5 = sm.process_probe(false, Some("Intermittent timeout"));
    assert_eq!(t5, None);
    assert_eq!(sm.state, MonitorState::Down);
    assert_eq!(sm.consecutive_successes, 0);

    // Now 2 consecutive successes:
    let t6 = sm.process_probe(true, None);
    assert_eq!(t6, None);
    assert_eq!(sm.state, MonitorState::Down);
    assert_eq!(sm.consecutive_successes, 1);

    let t7 = sm.process_probe(true, None);
    assert_eq!(
        t7,
        Some(StateTransition::IncidentResolved {
            incident_id: Some(42)
        }),
        "2 consecutive successes must resolve outage"
    );
    assert_eq!(sm.state, MonitorState::Up);
}

#[test]
fn test_u03_ssl_expiry_parsing_and_alert_deduplication() -> anyhow::Result<()> {
    let db = Database::open_in_memory()?;

    // Parse ASN.1 UTCTime
    let dt = parse_asn1_time_str("270124182325Z", false)?;
    assert_eq!(dt.to_rfc3339(), "2027-01-24T18:23:25+00:00");

    // Days until expiry
    let now = chrono::Utc::now();
    let expiry_10d = now + chrono::Duration::days(10);

    assert_eq!(days_until_expiry(&expiry_10d, &now), 10);
    assert_eq!(evaluate_ssl_alert_tier(30), None);
    assert_eq!(evaluate_ssl_alert_tier(10), Some("14d"));
    assert_eq!(evaluate_ssl_alert_tier(2), Some("3d"));

    // Test DB alert deduplication (only alert once per tier)
    let mon = "https://app.fathforce.com";
    assert!(db.should_alert_ssl(mon, "14d")?);
    db.record_ssl_alert(mon, "14d")?;
    assert!(
        !db.should_alert_ssl(mon, "14d")?,
        "Must not re-alert for same tier"
    );

    // Tier "3d" should still be allowed
    assert!(db.should_alert_ssl(mon, "3d")?);
    db.record_ssl_alert(mon, "3d")?;
    assert!(!db.should_alert_ssl(mon, "3d")?);

    Ok(())
}

#[test]
fn test_u04_uptime_history_rollups_and_percentages() -> anyhow::Result<()> {
    let db = Database::open_in_memory()?;
    let mon = "ApiGateway";

    // Insert 90 UP checks and 10 DOWN checks
    for _ in 0..90 {
        db.record_uptime_check(mon, true, Some(45), Some(200), None, None)?;
    }
    for _ in 0..10 {
        db.record_uptime_check(mon, false, Some(500), Some(500), Some("Crash"), None)?;
    }

    let pct = db.get_uptime_percentage(mon, 30)?;
    assert!((pct - 90.0).abs() < 0.1, "Expected 90.0% uptime, got {pct}");

    // Test history bits
    let bits = db.get_monitor_history_bits(mon, 5)?;
    assert_eq!(bits.len(), 5);
    // Last 5 were false
    assert_eq!(bits, vec![false, false, false, false, false]);

    // Test prune and rollup
    // Simulate older records by backdating in SQLite
    db.conn().execute(
        "UPDATE uptime_checks SET checked_at = datetime('now', '-8 days') WHERE id <= 50",
        [],
    )?;

    db.prune_uptime_checks_and_rollup()?;

    // Verify rollups were populated
    let rollup_count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM uptime_hourly_rollups WHERE monitor = ?1",
        [mon],
        |row| row.get(0),
    )?;
    assert!(rollup_count > 0, "Hourly rollups must be generated");

    // Verify calculated percentage across rollups + raw checks remains ~90%
    let pct_combined = db.get_uptime_percentage(mon, 30)?;
    assert!(
        (pct_combined - 90.0).abs() < 0.1,
        "Combined percentage must remain consistent, got {pct_combined}"
    );

    Ok(())
}

#[test]
fn test_u05_status_page_html_security_omission() {
    let public_mon = MonitorStatusSummary {
        name: "Public Web".to_string(),
        target: "https://public.example.com".to_string(),
        group: "Production".to_string(),
        state: MonitorState::Up,
        is_up: true,
        latency_ms: Some(25),
        status_code: Some(200),
        uptime_pct_24h: 100.0,
        uptime_pct_30d: 99.9,
        history_20: vec![true; 20],
        cert_expires_at: None,
        cert_days_left: None,
        public: true, // PUBLIC
    };

    let secret_mon = MonitorStatusSummary {
        name: "Super Secret Internal Node".to_string(),
        target: "http://10.240.0.99:9999/admin_classified".to_string(),
        group: "SecretInternal".to_string(),
        state: MonitorState::Down,
        is_up: false,
        latency_ms: None,
        status_code: None,
        uptime_pct_24h: 0.0,
        uptime_pct_30d: 0.0,
        history_20: vec![false; 20],
        cert_expires_at: None,
        cert_days_left: None,
        public: false, // PRIVATE
    };

    let html = render_status_page("CA Cloud", &[public_mon, secret_mon], &[]);

    // Security assertions:
    assert!(html.contains("Public Web"), "Must include public monitor");
    assert!(
        !html.contains("Super Secret Internal Node"),
        "Must NOT include private monitor name"
    );
    assert!(
        !html.contains("admin_classified"),
        "Must NOT leak private monitor target URL"
    );
    assert!(
        !html.contains("10.240.0.99"),
        "Must NOT leak private monitor IP"
    );
    assert!(
        !html.contains("<script src="),
        "Must be self-contained with no external JS dependencies"
    );
}

#[tokio::test]
async fn test_u06_outgoing_heartbeat_dead_man_switch() {
    let hit_count = Arc::new(AtomicUsize::new(0));
    let hit_count_clone = hit_count.clone();

    let app = Router::new().route(
        "/ping",
        get(move || {
            hit_count_clone.fetch_add(1, Ordering::SeqCst);
            async { "OK" }
        }),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let ping_url = format!("http://{addr}/ping");
    let status = send_heartbeat_ping(&ping_url)
        .await
        .expect("ping should succeed");
    assert_eq!(status, 200);
    assert_eq!(hit_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_u07_uptime_consistency_tui_cli_db() -> anyhow::Result<()> {
    let db = Database::open_in_memory()?;
    let cfg = Config::default();

    // Add monitor in DB
    db.add_uptime_monitor_record(&crate::db::UptimeMonitorRecord {
        id: 0,
        name: "ConsistencyMonitor".to_string(),
        target: "https://caterm.fathforce.com".to_string(),
        monitor_type: "http".to_string(),
        interval_secs: 60,
        expected_status: Some(200),
        expected_keyword: None,
        public: true,
        group_name: "Production".to_string(),
        created_at: String::new(),
    })?;

    // Record check
    db.record_uptime_check(
        "ConsistencyMonitor",
        true,
        Some(35),
        Some(200),
        None,
        Some("2027-01-01T00:00:00Z"),
    )?;

    // Fetch summary (used by both CLI --json and Cockpit TUI)
    let summaries = super::get_all_monitors_summary(&cfg, &db)?;
    let mon_summary = summaries
        .iter()
        .find(|m| m.name == "ConsistencyMonitor")
        .expect("Monitor should exist in summary");

    assert_eq!(mon_summary.name, "ConsistencyMonitor");
    assert!(mon_summary.is_up);
    assert_eq!(mon_summary.status_code, Some(200));
    assert_eq!(mon_summary.latency_ms, Some(35));
    assert_eq!(mon_summary.state, MonitorState::Up);
    assert_eq!(mon_summary.uptime_pct_30d, 100.0);

    // Verify DB direct check matches
    let db_pct = db.get_uptime_percentage("ConsistencyMonitor", 30)?;
    assert_eq!(mon_summary.uptime_pct_30d, db_pct);

    Ok(())
}
