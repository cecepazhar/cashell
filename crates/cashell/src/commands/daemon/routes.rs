//! Axum router and HTTP route handlers for the CAShell daemon API.

use axum::{
    extract::{connect_info::ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

use super::auth::{check_auth, resolve_client_ip, verify_heartbeat, DaemonState};
use crate::config::Config;

#[derive(Deserialize)]
pub struct ActionRequest {
    pub action: String,
    pub target: String,
}

#[derive(Serialize)]
pub struct ActionResponse {
    pub ok: bool,
    pub output: String,
}

pub async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "app": "cashell",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

pub async fn stats_handler(
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

pub async fn containers_handler(
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

pub async fn action_handler(
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

    let action = match body.action.as_str() {
        "restart" => crate::actions::Action::RestartContainer {
            name: body.target.clone(),
        },
        "stop" => crate::actions::Action::StopContainer {
            name: body.target.clone(),
        },
        "start" => crate::actions::Action::StartContainer {
            name: body.target.clone(),
        },
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("unknown action: {other}")})),
            )
                .into_response();
        }
    };

    // H-03: validate container name — must match ^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$
    let target = &body.target;
    if !is_valid_container_name(target) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("invalid container name: {target}")})),
        )
            .into_response();
    }

    // H-03: container must exist in `docker ps -a` (or podman fallback).
    if !container_exists(target).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("container {target} not found in docker ps -a")})),
        )
            .into_response();
    }

    let mut ctx = crate::actions::ActionContext::new("daemon", "bearer_client");
    if let Ok(db) = crate::db::Database::open() {
        ctx = ctx.with_db(Arc::new(std::sync::Mutex::new(db)));
    }

    match crate::actions::execute(action, &ctx).await {
        Ok(res) => Json(ActionResponse {
            ok: res.ok,
            output: res.output,
        })
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// H-03: validate container name against ^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$
pub fn is_valid_container_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return false;
    }
    match bytes[0] {
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {}
        _ => return false,
    }
    bytes[1..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// H-03: check whether the container exists in `docker ps -a` (or podman fallback).
/// Mockable via `CONTAINER_NAMES_OVERRIDE` env var (comma-separated) for tests.
pub async fn container_exists(name: &str) -> bool {
    if let Ok(override_val) = std::env::var("CONTAINER_NAMES_OVERRIDE") {
        return override_val.split(',').map(|s| s.trim()).any(|s| s == name);
    }
    let mut out = tokio::process::Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
        .await;
    if out.is_err() || matches!(&out, Ok(o) if !o.status.success()) {
        let pod_out = tokio::process::Command::new("podman")
            .args(["ps", "-a", "--format", "{{.Names}}"])
            .output()
            .await;
        if pod_out.is_ok() {
            out = pod_out;
        }
    }
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .any(|line| line.trim() == name),
        Err(_) => false,
    }
}

pub fn build_router(cfg: &Config) -> Router {
    let node_secrets = cfg.daemon.node_secrets.clone();
    let state = Arc::new(DaemonState {
        token: cfg.daemon.token.clone(),
        trusted_proxies: cfg.daemon.trusted_proxies.clone(),
        node_secrets,
    });
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

pub async fn collect_cors_preflight() -> impl IntoResponse {
    (
        StatusCode::NO_CONTENT,
        [
            ("access-control-allow-origin", "*"),
            ("access-control-allow-methods", "POST, GET, OPTIONS"),
            (
                "access-control-allow-headers",
                "content-type, user-agent, authorization",
            ),
        ],
    )
}

pub async fn tracker_js_handler() -> impl axum::response::IntoResponse {
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
  function sendPageview(){
    var d = base();
    d.event_type = 'pageview';
    beacon(d);
  }

  var scrollMilestones = [25, 50, 75, 90, 100];
  var scrollSent = {};
  function onScroll(){
    if (document.documentElement.scrollHeight <= document.documentElement.clientHeight) return;
    var pct = Math.min(100, Math.round(
      (document.documentElement.scrollTop + document.documentElement.clientHeight) /
      document.documentElement.scrollHeight * 100
    ));
    for (var i = 0; i < scrollMilestones.length; i++) {
      var m = scrollMilestones[i];
      if (pct >= m && !scrollSent[m]) {
        scrollSent[m] = true;
        var d = base();
        d.event_type = 'scroll';
        d.scroll_depth = m;
        beacon(d);
      }
    }
  }
  window.addEventListener('scroll', onScroll, {passive: true});

  var recentClicks = [];
  document.addEventListener('click', function(e){
    var now = Date.now();
    var x = e.clientX, y = e.clientY;
    recentClicks = recentClicks.filter(function(c){ return now - c.t < 1500; });
    var cluster = recentClicks.filter(function(c){
      return Math.abs(c.x - x) < 30 && Math.abs(c.y - y) < 30;
    });
    recentClicks.push({x: x, y: y, t: now});
    if (cluster.length >= 2) {
      var d = base();
      d.event_type = 'rage_click';
      d.click_x = x;
      d.click_y = y;
      beacon(d);
      recentClicks = [];
      return;
    }

    var target = e.target && e.target.closest ? e.target.closest('a, button, [role="button"]') : null;
    if (target) {
      var ctaText = (target.innerText || target.value || target.getAttribute('aria-label') || '').trim().substr(0, 80);
      var ctaId = target.id || target.name || target.className || 'button';
      var d = base();
      d.event_type = 'cta_click';
      d.event_name = ctaId;
      d.event_data = JSON.stringify({text: ctaText, tag: target.tagName.toLowerCase()});
      d.click_x = x;
      d.click_y = y;
      beacon(d);
    }
  }, {capture: true, passive: true});

  var dwellStart = Date.now();
  var activeDwellMs = 0;
  ['mousemove', 'keydown', 'scroll', 'touchstart'].forEach(function(ev){
    window.addEventListener(ev, function(){
      if (dwellStart === null) dwellStart = Date.now();
    }, {passive: true});
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

pub async fn collect_analytics_handler(
    State(state): State<Arc<DaemonState>>,
    connect: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let website_id = payload
        .get("website_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let url = payload
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("/")
        .to_string();
    let referrer = payload
        .get("referrer")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let title = payload
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_id = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("fallback")
        .to_string();

    let client_ip = resolve_client_ip(&connect, &headers, &state.trusted_proxies);

    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let (browser, os, device) = parse_user_agent(&ua);

    let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();
    let hash_input = format!("{client_ip}_{today_str}");
    let visitor_hash = hex_sha256(&hash_input);

    let event_type = payload
        .get("event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("pageview")
        .to_string();
    let event_name = payload
        .get("event_name")
        .and_then(|v| v.as_str())
        .map(String::from);
    let event_data = payload
        .get("event_data")
        .and_then(|v| v.as_str())
        .map(String::from);
    let click_x = payload
        .get("click_x")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
    let click_y = payload
        .get("click_y")
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
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
        Ok(Ok(())) => (
            StatusCode::OK,
            [
                ("access-control-allow-origin", "*"),
                ("access-control-allow-methods", "POST, GET, OPTIONS"),
                ("access-control-allow-headers", "*"),
            ],
            Json(serde_json::json!({"status": "ok"})),
        )
            .into_response(),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [
                ("access-control-allow-origin", "*"),
                ("access-control-allow-methods", "POST, GET, OPTIONS"),
                ("access-control-allow-headers", "*"),
            ],
            Json(serde_json::json!({"error": "db_write_failure"})),
        )
            .into_response(),
    }
}

pub async fn analytics_summary_handler(
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
                (StatusCode::OK, Json(json_val)).into_response()
            }
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "db_error").into_response(),
        },
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "db_open_error").into_response(),
    }
}

pub fn parse_user_agent(ua: &str) -> (String, String, String) {
    let ua_lower = ua.to_lowercase();
    let device = if ua_lower.contains("mobile")
        || ua_lower.contains("android")
        || ua_lower.contains("iphone")
    {
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

pub fn hex_sha256(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

pub async fn node_heartbeat_handler(
    State(state): State<Arc<DaemonState>>,
    _connect: ConnectInfo<SocketAddr>,
    Json(hb): Json<cashell_common::Heartbeat>,
) -> impl IntoResponse {
    if let Err(reason) = verify_heartbeat(&state, &hb) {
        eprintln!(
            "⚠ [H-05] rejecting heartbeat from node '{}': {}",
            hb.node_id, reason
        );
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "heartbeat_rejected", "reason": reason})),
        )
            .into_response();
    }

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
        signature: hb.signature.clone(),
    };

    if let Ok(db) = crate::db::get_db() {
        let _ = db.store_heartbeat(&db_hb);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "received",
            "node_id": hb.node_id,
        })),
    )
        .into_response()
}
