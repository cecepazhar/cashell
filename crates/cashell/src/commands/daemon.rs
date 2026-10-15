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
        .with_state(state)
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
