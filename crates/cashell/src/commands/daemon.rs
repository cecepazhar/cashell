//! Daemon and multi-node orchestration engine for cashell.
//!
//! Provides 24/7 fleet probing, automated SSH/local remediation,
//! Telegram alerts, Axum HTTP API, and systemd service management.

use crate::config::{Config, NodeConfig};
use anyhow::Result;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Parser, Debug, Clone)]
pub struct DaemonCommand {
    #[command(subcommand)]
    pub action: Option<DaemonAction>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum DaemonAction {
    /// Run the 24/7 background monitoring daemon & HTTP API
    Run {
        /// HTTP API port (overrides config)
        #[arg(short, long)]
        port: Option<u16>,

        /// Probe interval in seconds (overrides config)
        #[arg(short, long)]
        interval: Option<u64>,
    },
    /// Show live status of all services across configured nodes
    Status,
    /// Restart a specific service on a node
    Restart {
        /// Node name (e.g. ypc, hostinger, local)
        node: String,
        /// Service name (e.g. gitea, tailscale-funnel)
        service: String,
    },
    /// Install systemd service unit
    #[command(name = "install-service", alias = "install")]
    InstallService {
        /// Install as user service
        #[arg(long, default_value_t = true)]
        user: bool,
    },
    /// Stop and disable systemd service
    #[command(name = "stop-service", alias = "stop")]
    StopService {
        /// Stop user service
        #[arg(long, default_value_t = true)]
        user: bool,
    },
}

pub fn run(cmd: &DaemonCommand, cfg: &Config) -> Result<()> {
    let action = cmd.action.clone().unwrap_or(DaemonAction::Run {
        port: None,
        interval: None,
    });

    match action {
        DaemonAction::Run { port, interval } => run_daemon(cfg, port, interval),
        DaemonAction::Status => run_status(cfg),
        DaemonAction::Restart { node, service } => run_restart(cfg, &node, &service),
        DaemonAction::InstallService { user } => run_install_service(user),
        DaemonAction::StopService { user } => run_stop_service(user),
    }
}

// ── HTTP API Shared State & Handlers ─────────────────────────────────────────

struct DaemonState {
    token: String,
}

fn check_auth(state: &DaemonState, headers: &HeaderMap) -> bool {
    if state.token.is_empty() {
        return true;
    }
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == state.token)
        .unwrap_or(false)
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "app": "cashell",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn stats_handler(
    State(state): State<Arc<DaemonState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !check_auth(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    let snap = tokio::task::spawn_blocking(crate::systat::snapshot)
        .await
        .unwrap_or_default();

    Json(serde_json::json!({
        "cpu_pct": snap.cpu_pct,
        "ram_used_mb": snap.ram_used_mb,
        "ram_total_mb": snap.ram_total_mb,
        "disk_used_gb": snap.disk_used_gb,
        "disk_total_gb": snap.disk_total_gb
    }))
    .into_response()
}

async fn containers_handler(
    State(state): State<Arc<DaemonState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !check_auth(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    let mut out = tokio::process::Command::new("docker")
        .args(["ps", "-a", "--format", "{{json .}}"])
        .output()
        .await;

    if out.is_err() || matches!(&out, Ok(o) if !o.status.success()) {
        let podman_out = tokio::process::Command::new("podman")
            .args(["ps", "-a", "--format", "{{json .}}"])
            .output()
            .await;
        if podman_out.is_ok() {
            out = podman_out;
        }
    }

    match out {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let list: Vec<serde_json::Value> = stdout
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            Json(serde_json::json!(list)).into_response()
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).to_string();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": err})),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ActionRequest {
    action: String,
    target: String,
}

#[derive(Serialize)]
struct ActionResponse {
    ok: bool,
    output: String,
}

async fn action_handler(
    State(state): State<Arc<DaemonState>>,
    headers: HeaderMap,
    Json(body): Json<ActionRequest>,
) -> impl IntoResponse {
    if !check_auth(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    match body.action.as_str() {
        "restart" | "stop" | "start" => {
            let mut result = tokio::process::Command::new("docker")
                .args([&body.action, &body.target])
                .output()
                .await;
            if result.is_err() || matches!(&result, Ok(o) if !o.status.success()) {
                let pod_result = tokio::process::Command::new("podman")
                    .args([&body.action, &body.target])
                    .output()
                    .await;
                if pod_result.is_ok() {
                    result = pod_result;
                }
            }
            match result {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    let output = if stdout.trim().is_empty() {
                        stderr
                    } else {
                        stdout
                    };
                    Json(ActionResponse {
                        ok: o.status.success(),
                        output,
                    })
                    .into_response()
                }
                Err(e) => Json(ActionResponse {
                    ok: false,
                    output: e.to_string(),
                })
                .into_response(),
            }
        }
        "backup" => {
            let backup_file = format!("/tmp/{}-backup.tar.gz", body.target);
            let res = tokio::process::Command::new("tar")
                .args(["czf", &backup_file, &body.target])
                .output()
                .await;
            match res {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    let output = if stdout.trim().is_empty() {
                        if o.status.success() {
                            format!("Backup created: {backup_file}")
                        } else {
                            stderr
                        }
                    } else {
                        stdout
                    };
                    Json(ActionResponse {
                        ok: o.status.success(),
                        output,
                    })
                    .into_response()
                }
                Err(e) => Json(ActionResponse {
                    ok: false,
                    output: e.to_string(),
                })
                .into_response(),
            }
        }
        other => Json(ActionResponse {
            ok: false,
            output: format!("unknown action: {other}"),
        })
        .into_response(),
    }
}

pub fn build_router(token: String) -> Router {
    let state = Arc::new(DaemonState { token });
    Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/stats", get(stats_handler))
        .route("/api/containers", get(containers_handler))
        .route("/api/action", post(action_handler))
        .route("/api/node/heartbeat", post(node_heartbeat_handler))
        .route("/tracker.js", get(tracker_js_handler))
        .route(
            "/api/collect",
            post(collect_analytics_handler).options(collect_cors_preflight),
        )
        .route("/api/analytics/summary", get(analytics_summary_handler))
        .with_state(state)
}

/// Handle CORS preflight requests for /api/collect so browser-based trackers can send cross-origin telemetry.
async fn collect_cors_preflight() -> impl IntoResponse {
    (
        StatusCode::NO_CONTENT,
        [
            ("access-control-allow-origin", "*"),
            ("access-control-allow-methods", "POST, GET, OPTIONS"),
            ("access-control-allow-headers", "content-type, user-agent, authorization"),
        ],
    )
}

// ── Sovereign Web Analytics / Umami-like Endpoints ──────────────────────────

/// Serve the lightweight JavaScript tracking script for sovereign web analytics.
async fn tracker_js_handler() -> impl axum::response::IntoResponse {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/javascript"),
            (axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        r#"
(function(){
  var host = document.currentScript && document.currentScript.src ? document.currentScript.src.split('/').slice(0, 3).join('/') : (location.protocol + '//' + location.hostname + ':8090');
  var siteId = (document.currentScript && document.currentScript.getAttribute('data-site')) || 'default';
  function beacon(data){
    if (navigator.sendBeacon) {
      navigator.sendBeacon(host + '/api/collect', new Blob([JSON.stringify(data)], {type: 'application/json'}));
    } else if (XMLHttpRequest) {
      var x = new XMLHttpRequest();
      x.open('POST', host + '/api/collect', true);
      x.setRequestHeader('Content-Type', 'application/json');
      x.send(JSON.stringify(data));
    }
  }
  function base(){
    return {
      website_id: siteId,
      url: location.href,
      referrer: document.referrer,
      title: document.title,
      screen: screen.width + 'x' + screen.height,
      session_id: (function(){
        try {
          var s = sessionStorage.getItem('cashell_sid');
          if(!s){ s = 's_' + Math.random().toString(36).substr(2, 10); sessionStorage.setItem('cashell_sid', s); }
          return s;
        } catch(e){ return 's_fallback_' + Math.floor(Math.random()*999999); }
      })()
    };
  }
  // ── Pageview ──────────────────────────────────────────────────────────
  function sendPageview(){
    var d = base();
    d.event_type = 'pageview';
    beacon(d);
  }

  // ── Scroll Depth ─────────────────────────────────────────────────────
  var scrollMilestones = [25, 50, 75, 90, 100];
  var scrollSent = {};
  function onScroll(){
    if (document.documentElement.scrollHeight <= document.documentElement.clientHeight) return;
    var pct = Math.min(100, Math.round(
      (document.documentElement.scrollTop + document.documentElement.clientHeight) /
      document.documentElement.scrollHeight * 100
    ));
    var sent = false;
    for (var i = 0; i < scrollMilestones.length; i++) {
      var m = scrollMilestones[i];
      if (pct >= m && !scrollSent[m]) {
        scrollSent[m] = true;
        var d = base();
        d.event_type = 'scroll_depth';
        d.scroll_depth = m;
        beacon(d);
        sent = true;
      }
    }
  }
  window.addEventListener('scroll', onScroll, { passive: true });

  // ── Click & CTA Tracking + Rage Click Detection ─────────────────────
  var clickLog = {};
  function elementKey(el){
    if (!el) return 'unknown';
    if (el.dataset && el.dataset.cashellEvent) return el.dataset.cashellEvent;
    if (el.tagName) return el.tagName.toLowerCase() + (el.id ? '#' + el.id : '');
    return 'unknown';
  }
  function relativeCoords(clientX, clientY){
    var docW = Math.max(document.documentElement.scrollWidth, document.body ? document.body.scrollWidth : 0);
    var docH = Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0);
    var px = 0, py = 0;
    var docEl = document.documentElement;
    var body = document.body;
    if (docEl && body) {
      var docTop = window.pageYOffset || document.documentElement.scrollTop;
      var docLeft = window.pageXOffset || document.documentElement.scrollLeft;
      var clientRect = docEl.getClientRects && docEl.getClientRects()[0];
      px = clientX - (docLeft - (clientRect ? clientRect.left : 0));
      py = clientY - (docTop - (clientRect ? clientRect.top : 0));
    } else {
      px = clientX; py = clientY;
    }
    return {
      click_x: docW > 0 ? Math.min(100, Math.max(0, px / docW * 100)) : 0,
      click_y: docH > 0 ? Math.min(100, Math.max(0, py / docH * 100)) : 0
    };
  }
  function onClick(e){
    var target = e.target || e.srcElement;
    while (target && target !== document.body && target !== document.documentElement) {
      if (target.dataset && target.dataset.cashellEvent) break;
      if (target.tagName === 'BUTTON' || target.tagName === 'A') break;
      target = target.parentElement;
    }
    if (!target || target === document.body || target === document.documentElement) return;

    var isCta = (target.dataset && target.dataset.cashellEvent) ? true :
                (target.tagName === 'BUTTON' || target.tagName === 'A');
    if (!isCta) return;

    var key = elementKey(target);
    var now = Date.now();
    clickLog[key] = clickLog[key] || [];
    clickLog[key] = clickLog[key].filter(function(t){ return now - t < 1000; });
    clickLog[key].push(now);

    var coords = relativeCoords(e.clientX || 0, e.clientY || 0);
    var event_name = (target.dataset && target.dataset.cashellEvent) ?
                     target.dataset.cashellEvent : key;
    var event_data = (target.dataset && target.dataset.cashellVal) ?
                     target.dataset.cashellVal : null;

    if (clickLog[key].length > 3) {
      var d = base();
      d.event_type = 'rage_click';
      d.event_name = event_name;
      d.event_data = event_data ? JSON.stringify({ val: event_data, clicks: clickLog[key].length }) : null;
      d.click_x = coords.click_x;
      d.click_y = coords.click_y;
      beacon(d);
    } else {
      var d2 = base();
      d2.event_type = 'click';
      d2.event_name = event_name;
      d2.event_data = event_data ? JSON.stringify({ val: event_data }) : null;
      d2.click_x = coords.click_x;
      d2.click_y = coords.click_y;
      beacon(d2);
    }
  }
  document.addEventListener('click', onClick, { passive: true, capture: true });

  // ── Active Dwell Time ─────────────────────────────────────────────────
  var dwellStart = null;
  var activeDwellMs = 0;
  var lastInteraction = Date.now();
  function userInteracted(){ lastInteraction = Date.now(); }
  ['mousemove', 'keydown', 'scroll'].forEach(function(evt){
    document.addEventListener(evt, userInteracted, { passive: true });
  });
  function onVisibilityChange(){
    if (document.visibilityState === 'visible') {
      dwellStart = Date.now();
    } else {
      flushDwell();
    }
  }
  function flushDwell(){
    if (dwellStart !== null && document.visibilityState === 'visible') {
      activeDwellMs += Date.now() - dwellStart;
      dwellStart = null;
    }
    if (activeDwellMs > 0) {
      var durSecs = Math.round(activeDwellMs / 1000);
      var d = base();
      d.event_type = 'dwell_time';
      d.duration_secs = durSecs;
      beacon(d);
    }
  }
  document.addEventListener('visibilitychange', onVisibilityChange);
  window.addEventListener('beforeunload', function(){ flushDwell(); });

  if (document.readyState === 'complete') sendPageview();
  else window.addEventListener('load', sendPageview);
})();
"#,
    )
}

/// Receive and persist a web analytics event to the sovereign SQLite analytics database.
async fn collect_analytics_handler(
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let website_id = payload.get("website_id").and_then(|v| v.as_str()).unwrap_or("default").to_string();
    let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("/").to_string();
    let referrer = payload.get("referrer").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let title = payload.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let session_id = payload.get("session_id").and_then(|v| v.as_str()).unwrap_or("fallback").to_string();

    // Extract X-Forwarded-For or client IP for anonymized hashing
    let forwarded = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).unwrap_or("");
    let client_ip = forwarded.split(',').next().unwrap_or("").trim().to_string();

    // User-Agent parsing for browser, OS, device
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (browser, os, device) = parse_user_agent(&ua);

    // Daily sovereign salt for PII-protected Visitor Hash (no cookies/PII stored)
    let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();
    let hash_input = format!("{client_ip}_{today_str}");
    let visitor_hash = hex_sha256(&hash_input);

    // ── Advanced event fields ────────────────────────────────────────────
    let event_type = payload
        .get("event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("pageview")
        .to_string();
    let event_name = payload.get("event_name").and_then(|v| v.as_str()).map(String::from);
    let event_data = payload.get("event_data").and_then(|v| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            Some(v.to_string())
        }
        _ => None,
    });
    let click_x = payload.get("click_x").and_then(|v| v.as_f64());
    let click_y = payload.get("click_y").and_then(|v| v.as_f64());
    let scroll_depth = payload
        .get("scroll_depth")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let duration_secs = payload.get("duration_secs").and_then(|v| v.as_i64());

    let db = crate::db::Database::open();
    let result = db.map(|d| {
        d.record_analytics_event(
            &website_id,
            &session_id,
            &url,
            &referrer,
            &title,
            &browser,
            &os,
            &device,
            "",
            &visitor_hash,
            &event_type,
            event_name.as_deref(),
            event_data.as_deref(),
            click_x,
            click_y,
            scroll_depth,
            duration_secs,
        )
    });

    match result {
        Ok(Ok(())) => {
            (
                StatusCode::OK,
                [
                    ("access-control-allow-origin", "*"),
                    ("access-control-allow-methods", "POST, GET, OPTIONS"),
                    ("access-control-allow-headers", "*"),
                ],
                Json(serde_json::json!({"status": "ok"})),
            )
                .into_response()
        }
        _ => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [
                    ("access-control-allow-origin", "*"),
                    ("access-control-allow-methods", "POST, GET, OPTIONS"),
                    ("access-control-allow-headers", "*"),
                ],
                Json(serde_json::json!({"error": "db_write_failure"})),
            )
                .into_response()
        }
    }
}

/// Fetch overall web traffic & analytics summary from the SQLite database.
async fn analytics_summary_handler() -> impl IntoResponse {
    let db = crate::db::Database::open();
    match db {
        Ok(d) => match d.get_analytics_summary(None) {
            Ok(summary) => {
                let json_val = serde_json::json!({
                    "pageviews": summary.pageviews,
                    "unique_visitors": summary.unique_visitors,
                    "active_visitors": summary.active_visitors,
                    "bounce_rate": summary.bounce_rate,
                    "avg_session_secs": summary.avg_session_secs,
                    "top_pages": summary.top_pages,
                    "top_referrers": summary.top_referrers,
                    "device_breakdown": summary.device_breakdown,
                    "browser_breakdown": summary.browser_breakdown,
                    "os_breakdown": summary.os_breakdown,
                    "country_breakdown": summary.country_breakdown,
                    "daily_traffic": summary.daily_traffic,
                    "product_analytics": summary.product_analytics,
                    "scroll_funnel": summary.scroll_funnel,
                    "top_cta_events": summary.top_cta_events,
                    "rage_clicks": summary.rage_clicks,
                    "avg_dwell_time_secs": summary.avg_dwell_time_secs,
                });
                (
                    StatusCode::OK,
                    [
                        ("access-control-allow-origin", "*"),
                        ("access-control-allow-methods", "GET, OPTIONS"),
                    ],
                    Json(json_val),
                )
                    .into_response()
            }
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "db_error").into_response(),
        },
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "db_open_error").into_response(),
    }
}

// ── Analytics Helpers ──────────────────────────────────────────────────────

fn parse_user_agent(ua: &str) -> (String, String, String) {
    let ua_lower = ua.to_lowercase();
    let device = if ua_lower.contains("mobile") || ua_lower.contains("android") || ua_lower.contains("iphone") {
        "Mobile".to_string()
    } else {
        "Desktop".to_string()
    };

    let browser = if ua_lower.contains("edge") || ua_lower.contains("edg") {
        "Edge".to_string()
    } else if ua_lower.contains("chrome") && !ua_lower.contains("chromium") {
        "Chrome".to_string()
    } else if ua_lower.contains("firefox") {
        "Firefox".to_string()
    } else if ua_lower.contains("safari") && !ua_lower.contains("chrome") {
        "Safari".to_string()
    } else {
        "Unknown".to_string()
    };

    let os = if ua_lower.contains("linux") {
        "Linux".to_string()
    } else if ua_lower.contains("mac os") {
        "MacOS".to_string()
    } else if ua_lower.contains("windows") {
        "Windows".to_string()
    } else if ua_lower.contains("android") {
        "Android".to_string()
    } else if ua_lower.contains("iphone") || ua_lower.contains("ipad") {
        "iOS".to_string()
    } else {
        "Unknown".to_string()
    };

    (browser, os, device)
}

fn hex_sha256(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

async fn node_heartbeat_handler(
    Json(hb): Json<cashell_common::Heartbeat>,
) -> impl IntoResponse {
    let services_json = serde_json::to_string(&hb.services).unwrap_or_else(|_| "[]".to_string());
    let db_hb = crate::db::NodeHeartbeat {
        node_id: hb.node_id.clone(),
        cpu_percent: hb.cpu_pct,
        ram_used_mb: hb.ram_used_mb as i64,
        ram_total_mb: hb.ram_total_mb as i64,
        disk_used_gb: hb.disk_used_gb as i64,
        disk_total_gb: hb.disk_total_gb as i64,
        services_json,
        uptime_secs: hb.uptime_secs as i64,
        version: hb.version,
        received_at: chrono::Local::now().to_rfc3339(),
        display_name: None,
        ip: None,
        signature: None,
    };

    if let Ok(db) = crate::db::get_db() {
        let _ = db.store_heartbeat(&db_hb);
    }

    (StatusCode::OK, Json(serde_json::json!({
        "status": "received",
        "node_id": hb.node_id,
    })))
}

// ── Service Probing & Multi-Node Logic ───────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    Up,
    Down,
    Timeout,
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub status: ProbeStatus,
    pub latency_ms: Option<u128>,
}

pub async fn probe_service(host: &str, port: u16, timeout_ms: u64) -> ProbeResult {
    let addr = format!("{host}:{port}");
    let start = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        tokio::net::TcpStream::connect(&addr),
    )
    .await;

    match outcome {
        Ok(Ok(_)) => ProbeResult {
            status: ProbeStatus::Up,
            latency_ms: Some(start.elapsed().as_millis()),
        },
        Ok(Err(_)) => ProbeResult {
            status: ProbeStatus::Down,
            latency_ms: None,
        },
        Err(_) => ProbeResult {
            status: ProbeStatus::Timeout,
            latency_ms: None,
        },
    }
}

pub async fn probe_all(
    nodes: &std::collections::HashMap<String, NodeConfig>,
) -> Vec<(String, String, ProbeResult)> {
    let mut set = tokio::task::JoinSet::new();
    for (node_name, node) in nodes {
        for svc in &node.services {
            let n_name = node_name.clone();
            let s_name = svc.name.clone();
            let host = node.ip.clone();
            let port = svc.port;
            set.spawn(async move {
                let r = probe_service(&host, port, 5000).await;
                (n_name, s_name, r)
            });
        }
    }
    let mut out = Vec::new();
    while let Some(res) = set.join_next().await {
        if let Ok(item) = res {
            out.push(item);
        }
    }
    out.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));
    out
}

pub fn expand_tilde(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

pub async fn execute_remediation(
    ssh_key_path: &str,
    node: &NodeConfig,
    node_name: &str,
    cmd: &str,
) -> Result<String, String> {
    if node_name == "local" || node.ip == "127.0.0.1" || node.ip == "localhost" {
        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
            .map_err(|e| format!("failed to spawn local command: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "exit {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let key = expand_tilde(ssh_key_path);
        let out = tokio::process::Command::new("ssh")
            .arg("-i")
            .arg(&key)
            .arg("-o")
            .arg("StrictHostKeyChecking=no")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg(format!("{}@{}", node.user, node.ip))
            .arg(cmd)
            .output()
            .await
            .map_err(|e| format!("failed to spawn ssh: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "ssh exit {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

pub async fn send_telegram_alert(
    bot_token: &str,
    chat_id: &str,
    text: &str,
) -> Result<(), String> {
    if bot_token.is_empty() || chat_id.is_empty() {
        return Ok(());
    }
    let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
    let client = reqwest::Client::new();
    let res = client
        .post(&url)
        .json(&serde_json::json!({ "chat_id": chat_id, "text": text }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("telegram status {}", res.status()));
    }
    Ok(())
}

// ── Subcommand Implementations ───────────────────────────────────────────────

fn run_daemon(cfg: &Config, port_opt: Option<u16>, interval_opt: Option<u64>) -> Result<()> {
    let port = port_opt.unwrap_or(cfg.daemon.port);
    let interval = interval_opt.unwrap_or(cfg.daemon.interval_secs);
    let threshold = cfg.daemon.retry_threshold;

    println!(
        "{} starting on 0.0.0.0:{} (interval={}s, threshold={})",
        "CAShell daemon".green().bold(),
        port,
        interval,
        threshold
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let app = build_router(cfg.daemon.token.clone());
        let addr = format!("0.0.0.0:{port}");
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                println!(
                    "{} HTTP API listening on http://0.0.0.0:{port}/api/health",
                    "✔".green()
                );
                Some(l)
            }
            Err(e) => {
                eprintln!("{} Failed to bind HTTP API to {}: {e}", "✖".red(), addr);
                None
            }
        };

        if let Some(l) = listener {
            tokio::spawn(async move {
                if let Err(e) = axum::serve(l, app).await {
                    eprintln!("HTTP API server error: {e}");
                }
            });
        }

        let mut fails: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
        println!("{} Service probe loop initialized.", "✔".green());

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\nShutdown signal received. CAShell daemon stopping.");
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(interval)) => {
                    let results = probe_all(&cfg.daemon.nodes).await;
                    for (node_name, svc_name, r) in results {
                        let key = format!("{node_name}.{svc_name}");
                        if r.status == ProbeStatus::Up {
                            if fails.remove(&key).is_some() {
                                println!("{} {key} recovered (UP)", "✔".green());
                                let rec_msg = format!("Service {key} recovered (UP)");
                                if let Ok(db) = crate::db::get_db() {
                                    let _ = db.log_event("INFO", "daemon", &rec_msg);
                                }
                            }
                        } else {
                            let count = fails.entry(key.clone()).or_insert(0);
                            *count += 1;
                            println!("{} {key} down ({count} consecutive failures)", "⚠".yellow());
                            if *count == threshold {
                                let msg = format!("❌ {key} DOWN after {count} consecutive failures");

                                // 1. Telegram alert
                                if let Err(e) = send_telegram_alert(
                                    &cfg.daemon.telegram_bot_token,
                                    &cfg.daemon.telegram_chat_id,
                                    &msg,
                                ).await {
                                    eprintln!("  Telegram alert error: {e}");
                                } else if !cfg.daemon.telegram_bot_token.is_empty() && !cfg.daemon.telegram_chat_id.is_empty() {
                                    println!("  Telegram alert sent");
                                }

                                // 2. SQLite system_events log
                                if let Ok(db) = crate::db::get_db() {
                                    if let Err(e) = db.log_event("ERROR", "daemon", &msg) {
                                        eprintln!("  DB log error: {e}");
                                    }
                                }

                                // 3. Automated remediation
                                if let Some(cmd) = cfg.daemon.remediation.get(&key) {
                                    if let Some(node) = cfg.daemon.nodes.get(&node_name) {
                                        println!("  Executing remediation for {key}: {cmd}");
                                        match execute_remediation(&cfg.daemon.ssh_key_path, node, &node_name, cmd).await {
                                            Ok(out) => {
                                                println!("  Remediated {key}: {}", out.trim());
                                                let rem_log = format!("Remediated {key} with `{cmd}`: {}", out.trim());
                                                if let Ok(db) = crate::db::get_db() {
                                                    let _ = db.log_event("INFO", "daemon", &rem_log);
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("  Remediation for {key} failed: {e}");
                                                let rem_fail = format!("Remediation for {key} failed: {e}");
                                                if let Ok(db) = crate::db::get_db() {
                                                    let _ = db.log_event("ERROR", "daemon", &rem_fail);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    Ok(())
}

fn run_status(cfg: &Config) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;
    let api_url = format!("http://127.0.0.1:{}/api/health", cfg.daemon.port);
    let (daemon_status, daemon_ver) = match client.get(&api_url).send() {
        Ok(resp) if resp.status().is_success() => {
            let v = resp.json::<serde_json::Value>().ok();
            let ver = v
                .as_ref()
                .and_then(|j| j.get("version"))
                .and_then(|s| s.as_str())
                .unwrap_or("unknown")
                .to_string();
            ("RUNNING".green().bold().to_string(), ver)
        }
        _ => ("OFFLINE".red().bold().to_string(), String::new()),
    };

    println!(
        "Daemon API (127.0.0.1:{}): {} {}",
        cfg.daemon.port,
        daemon_status,
        if daemon_ver.is_empty() {
            String::new()
        } else {
            format!("(v{daemon_ver})")
        }
    );

    println!("\nProbing fleet services across configured nodes...");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let results = rt.block_on(probe_all(&cfg.daemon.nodes));

    println!(
        "\n{:<14} {:<20} {:<10} {:<10}",
        "NODE", "SERVICE", "STATUS", "LATENCY"
    );
    println!("{:-<14} {:-<20} {:-<10} {:-<10}", "", "", "", "");

    let mut up_count = 0;
    let total_count = results.len();
    for (node, svc, res) in &results {
        let status_str = match res.status {
            ProbeStatus::Up => {
                up_count += 1;
                "OK".green().bold().to_string()
            }
            ProbeStatus::Down => "DOWN".red().bold().to_string(),
            ProbeStatus::Timeout => "TIMEOUT".yellow().bold().to_string(),
        };
        let lat_str = match res.latency_ms {
            Some(ms) => format!("{ms}ms"),
            None => "-".to_string(),
        };
        println!("{:<14} {:<20} {:<10} {:<10}", node, svc, status_str, lat_str);
    }

    println!(
        "\nFleet Health: {}/{} services operational.",
        up_count, total_count
    );
    Ok(())
}

fn run_restart(cfg: &Config, node: &str, service: &str) -> Result<()> {
    let key = format!("{node}.{service}");
    let cmd = match cfg.daemon.remediation.get(&key) {
        Some(c) => c.clone(),
        None => {
            eprintln!(
                "{} No remediation command configured for '{}'",
                "Error:".red(),
                key
            );
            return Ok(());
        }
    };
    let node_cfg = match cfg.daemon.nodes.get(node) {
        Some(n) => n,
        None => {
            eprintln!(
                "{} Node '{}' not found in configuration",
                "Error:".red(),
                node
            );
            return Ok(());
        }
    };

    println!("Executing remediation for {}: `{}`", key.cyan(), cmd);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    match rt.block_on(execute_remediation(
        &cfg.daemon.ssh_key_path,
        node_cfg,
        node,
        &cmd,
    )) {
        Ok(out) => {
            println!("{} Remediation output:\n{}", "✔".green(), out.trim());
            if let Ok(db) = crate::db::get_db() {
                let _ =
                    db.log_event("INFO", "daemon", &format!("Manual restart of {key}: {cmd}"));
            }
        }
        Err(e) => {
            eprintln!("{} Remediation failed: {e}", "✖".red());
            if let Ok(db) = crate::db::get_db() {
                let _ = db.log_event(
                    "ERROR",
                    "daemon",
                    &format!("Manual restart of {key} failed: {e}"),
                );
            }
        }
    }
    Ok(())
}

pub fn service_unit_path(user: bool) -> std::path::PathBuf {
    if user {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from(".config"))
            .join("systemd")
            .join("user")
            .join("cashell-daemon.service")
    } else {
        std::path::PathBuf::from("/etc/systemd/system/cashell-daemon.service")
    }
}

pub fn generate_service_unit(user: bool) -> String {
    if user {
        r#"[Unit]
Description=CAShell 24/7 Fleet & Health Monitoring Daemon
After=network.target tailscaled.service

[Service]
Type=simple
ExecStart=%h/.local/bin/cashell daemon run
Restart=always
RestartSec=10
Environment=PATH=%h/.local/bin:/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=default.target
"#
        .to_string()
    } else {
        r#"[Unit]
Description=CAShell 24/7 Fleet & Health Monitoring Daemon
After=network.target tailscaled.service

[Service]
Type=simple
ExecStart=/usr/local/bin/cashell daemon run
Restart=always
RestartSec=10
Environment=PATH=/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=multi-user.target
"#
        .to_string()
    }
}

fn run_install_service(user: bool) -> Result<()> {
    let unit_path = service_unit_path(user);
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = generate_service_unit(user);
    std::fs::write(&unit_path, &content)?;
    println!(
        "{} Wrote systemd service unit to: {}",
        "✔".green(),
        unit_path.display()
    );

    if user {
        let username = std::env::var("USER").unwrap_or_else(|_| "cecepazhar".to_string());
        println!("Enabling lingering for user '{}'...", username);
        let _ = std::process::Command::new("loginctl")
            .args(["enable-linger", &username])
            .status();

        println!("Reloading systemd user daemon and enabling service...");
        let reload = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        if let Err(e) = reload {
            eprintln!("Warning: failed to reload user daemon: {e}");
        }

        let enable = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "cashell-daemon.service"])
            .status();
        match enable {
            Ok(s) if s.success() => {
                println!(
                    "{} cashell-daemon.service enabled and started.",
                    "✔".green()
                );
            }
            Ok(s) => eprintln!("systemctl returned exit code: {:?}", s.code()),
            Err(e) => eprintln!("Failed to start service: {e}"),
        }

        println!("\nService management commands:");
        println!("  systemctl --user status cashell-daemon.service");
        println!("  journalctl --user -u cashell-daemon.service -f");
    } else {
        println!("Reloading systemd and enabling system service...");
        let _ = std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status();
        let _ = std::process::Command::new("systemctl")
            .args(["enable", "--now", "cashell-daemon.service"])
            .status();

        println!("\nService management commands:");
        println!("  sudo systemctl status cashell-daemon.service");
        println!("  sudo journalctl -u cashell-daemon.service -f");
    }
    Ok(())
}

fn run_stop_service(user: bool) -> Result<()> {
    let args_stop = if user {
        vec!["--user", "stop", "cashell-daemon.service"]
    } else {
        vec!["stop", "cashell-daemon.service"]
    };
    let args_disable = if user {
        vec!["--user", "disable", "cashell-daemon.service"]
    } else {
        vec!["disable", "cashell-daemon.service"]
    };

    println!("Stopping cashell-daemon.service...");
    let _ = std::process::Command::new("systemctl")
        .args(&args_stop)
        .status();

    println!("Disabling cashell-daemon.service...");
    let _ = std::process::Command::new("systemctl")
        .args(&args_disable)
        .status();

    println!(
        "{} cashell-daemon.service stopped and disabled.",
        "✔".green()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(cfg.daemon.interval_secs, 60);
        assert_eq!(cfg.daemon.retry_threshold, 3);
        assert_eq!(cfg.daemon.ssh_key_path, "~/.ssh/id_ed25519");
        assert!(cfg.daemon.nodes.contains_key("ypc"));
        assert!(cfg.daemon.nodes.contains_key("hostinger"));
        assert!(cfg.daemon.nodes.contains_key("local"));
        assert_eq!(
            cfg.daemon.remediation.get("ypc.gitea").map(String::as_str),
            Some("podman restart gitea")
        );
    }
}
