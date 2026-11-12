//! CAOrch — Lightweight Fleet Agent Daemon for CA Ecosystem.
//!
//! Runs on remote nodes (VPS) to collect heartbeats, report status,
//! execute whitelisted commands, and provide automated self-healing.

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};
use tokio::sync::RwLock;

use cashell_common::{
    heartbeat_canonical_json, CommandResponse, Heartbeat, NodeAction, ServiceStatus,
};

/// 30 second timeout for any executed node action.
const COMMAND_TIMEOUT_SECS: u64 = 30;
/// 64 KB cap for captured stdout/stderr of any executed node action.
const OUTPUT_CAP_BYTES: usize = 64 * 1024;

/// Validate a systemd service name or docker container name.
/// Rejects anything that could be a shell metacharacter, path traversal,
/// or whitespace-separated argument injection (hard security rule 3).
fn is_valid_identifier(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return false;
    }
    let first = bytes[0];
    if !(first.is_ascii_alphanumeric()) {
        return false;
    }
    for &b in &bytes[1..] {
        if !b.is_ascii_alphanumeric() && !matches!(b, b'_' | b'.' | b'@' | b'-') {
            return false;
        }
    }
    true
}

/// Constant-time comparison of two ASCII strings (hard security rule for bearer tokens).
fn const_time_eq(a: &str, b: &str) -> bool {
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

/// Check the `Authorization: Bearer <token>` header against the configured token.
/// Returns `true` if authenticated, `false` otherwise.
fn is_authorized(headers: &axum::http::HeaderMap, expected_token: &str) -> bool {
    if expected_token.is_empty() {
        // Empty configured token means the agent was started without auth; that state
        // is refused at startup for non-loopback binds, so reaching here with an empty
        // expected token on a loopback bind is the only case, and we fail closed.
        return false;
    }
    let Some(header) = headers.get(axum::http::header::AUTHORIZATION) else {
        return false;
    };
    let Some(value) = header.to_str().ok() else {
        return false;
    };
    let Some(bearer) = value.strip_prefix("Bearer ") else {
        return false;
    };
    const_time_eq(bearer, expected_token)
}

fn truncate_output(s: &str) -> String {
    if s.len() <= OUTPUT_CAP_BYTES {
        s.to_string()
    } else {
        let mut out = s[..OUTPUT_CAP_BYTES].to_string();
        out.push_str("…[truncated]");
        out
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "caorch",
    version,
    about = "CAOrch — Fleet Agent Daemon for CA Ecosystem"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the agent daemon (default)
    Run {
        /// Port to listen on (default: 8091)
        #[arg(short, long, default_value = "8091")]
        port: u16,

        /// Cashell central URL for heartbeat reporting
        #[arg(long)]
        central: Option<String>,

        /// Bearer token required on /api/command and /api/heartbeat.
        /// If empty, the agent refuses to start unless `--bind` is a loopback address.
        #[arg(long, default_value = "")]
        token: String,

        /// H-05: per-node secret for HMAC-SHA256 heartbeat signatures.
        /// Overridable via env `CAORCH_NODE_SECRET`; stored in config with 0600.
        /// TODO(F-02): proper per-node secret distribution/rotation.
        #[arg(long, default_value = "")]
        secret: String,

        /// Address to bind the listener to (default: 127.0.0.1).
        /// A non-loopback bind requires a non-empty `--token`.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },
    /// Check daemon health status
    Status {
        #[arg(short, long, default_value = "8091")]
        port: u16,
    },
    /// Generate Ed25519 keypair and show node enrollment info
    Enroll {
        #[arg(long)]
        central: Option<String>,
    },
    /// Print version information
    Version,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub node_id: String,
    pub port: u16,
    pub central_url: String,
    pub heartbeat_interval_secs: u64,
    pub services: Vec<MonitoredService>,
    pub remediation_enabled: bool,
    pub token: String,
    pub bind: String,
    /// H-05: per-node secret for HMAC-SHA256 heartbeat signatures. Empty = unsigned
    /// (legacy behavior). TODO(F-02): per-node secret stored in config with 0600.
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredService {
    pub name: String,
    pub port: Option<u16>,
    pub check_cmd: Option<String>,
    pub restart_cmd: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        let node_id = format!("{}-node", std::env::consts::OS);
        Self {
            node_id,
            port: 8091,
            central_url: String::new(),
            heartbeat_interval_secs: 30,
            services: vec![],
            remediation_enabled: true,
            token: String::new(),
            bind: "127.0.0.1".to_string(),
            secret: String::new(),
        }
    }
}

pub struct AppState {
    pub config: AgentConfig,
    pub start_time: std::time::Instant,
    pub failure_counts: RwLock<std::collections::HashMap<String, u32>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    match args.command.unwrap_or(Commands::Run {
        port: 8091,
        central: None,
        token: String::new(),
        bind: "127.0.0.1".to_string(),
        secret: String::new(),
    }) {
        Commands::Run {
            port,
            central,
            token,
            bind,
            secret: secret_cli,
        } => {
            // H-05: env var takes precedence over CLI flag.
            let secret = std::env::var("CAORCH_NODE_SECRET").unwrap_or(secret_cli);
            run_daemon(port, central, token, bind, secret).await
        }
        Commands::Status { port } => check_status(port).await,
        Commands::Enroll { central } => enroll_node(central),
        Commands::Version => {
            println!("caorch v{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn is_loopback(bind: &str) -> bool {
    match bind.parse::<Ipv4Addr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => bind == "::1" || bind == "localhost",
    }
}

async fn run_daemon(
    port: u16,
    central: Option<String>,
    token: String,
    bind: String,
    secret: String,
) -> Result<()> {
    if !is_loopback(&bind) && token.is_empty() {
        eprintln!(
            "✖ Refusing to start: bind address '{}' is not loopback but --token is empty. \
             caorch requires authentication (a non-empty --token) whenever it listens \
             on a non-loopback address. Set --token to a secret, or use --bind 127.0.0.1.",
            bind
        );
        std::process::exit(2);
    }

    let mut cfg = AgentConfig {
        port,
        token,
        bind: bind.clone(),
        secret,
        ..AgentConfig::default()
    };
    if let Some(c) = central {
        cfg.central_url = c;
    }

    println!(
        "🚀 CAOrch agent daemon starting on {}: {} (node: {})",
        bind, port, cfg.node_id
    );

    let state = Arc::new(AppState {
        config: cfg.clone(),
        start_time: std::time::Instant::now(),
        failure_counts: RwLock::new(std::collections::HashMap::new()),
    });

    // Start background heartbeat reporter if central_url configured
    if !cfg.central_url.is_empty() {
        let reporter_state = Arc::clone(&state);
        tokio::spawn(async move {
            heartbeat_loop(reporter_state).await;
        });
        println!("📡 Heartbeat reporter active -> {}", cfg.central_url);
    }

    // Build Axum router
    let app = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/heartbeat", get(heartbeat_handler))
        .route("/api/command", post(command_handler))
        .with_state(state);

    let ip: std::net::IpAddr = bind
        .parse()
        .with_context(|| format!("failed to parse bind address '{}'", bind))?;
    let addr = SocketAddr::new(ip, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed binding to {}", addr))?;

    println!("✔ Listening on http://{}/api/health", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Returns a 401 `Response` with a JSON error body.
fn unauthorized_response(command_id_hint: &str) -> Response {
    let body = CommandResponse {
        command_id: command_id_hint.to_string(),
        success: false,
        exit_code: 401,
        stdout: String::new(),
        stderr: "Authentication required".to_string(),
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

/// Returns a 422 `Response` with a JSON error body, for actions that are rejected
/// because of an invalid or unauthorised identifier (not an auth failure).
fn unprocessable_response(command_id: &str, reason: &str) -> Response {
    let body = CommandResponse {
        command_id: command_id.to_string(),
        success: false,
        exit_code: 422,
        stdout: String::new(),
        stderr: reason.to_string(),
    };
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    Json(serde_json::json!({
        "status": "ok",
        "app": "caorch",
        "node_id": state.config.node_id,
        "uptime_secs": uptime,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn heartbeat_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    if !is_authorized(&headers, &state.config.token) {
        return unauthorized_response("heartbeat");
    }
    let hb = collect_heartbeat(&state).await;
    (StatusCode::OK, Json(hb)).into_response()
}

async fn command_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(action): Json<NodeAction>,
) -> Response {
    if !is_authorized(&headers, &state.config.token) {
        return unauthorized_response(action.id());
    }

    // Validate the identifier before deciding which fixed program+argv to run.
    let validation_err = match &action {
        NodeAction::ServiceRestart { name, .. }
        | NodeAction::ServiceStart { name, .. }
        | NodeAction::ServiceStop { name, .. }
        | NodeAction::ServiceStatus { name, .. } => {
            if !is_valid_identifier(name) {
                Some(
                    "Service name rejected: failed identifier validation (must match ^[a-zA-Z0-9][a-zA-Z0-9_.@-]{0,127}$)"
                        .to_string(),
                )
            } else if !state.config.services.is_empty()
                && !state
                    .config
                    .services
                    .iter()
                    .any(|s| s.name.as_str() == name.as_str())
            {
                Some(format!(
                    "Service '{}' is not in the configured services allowlist",
                    name
                ))
            } else {
                None
            }
        }
        NodeAction::ContainerRestart { name, .. } => {
            if !is_valid_identifier(name) {
                Some(
                    "Container name rejected: failed identifier validation (must match ^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$)"
                        .to_string(),
                )
            } else {
                None
            }
        }
        NodeAction::ContainerList { .. } | NodeAction::DiskUsage { .. } => None,
    };
    if let Some(reason) = validation_err {
        return unprocessable_response(action.id(), &reason);
    }

    // ContainerRestart additionally requires the name to exist in `docker ps -a` output
    // before we attempt to restart it (hard security rule 3, task H-06).
    if let NodeAction::ContainerRestart { name, .. } = &action {
        if let Err(e) = container_exists(name).await {
            return unprocessable_response(action.id(), &format!("Container check failed: {}", e));
        }
    }

    let output = execute_node_action(&action).await;
    match output {
        Ok((exit_code, stdout, stderr)) => {
            let body = CommandResponse {
                command_id: action.id().to_string(),
                success: exit_code == 0,
                exit_code,
                stdout: truncate_output(&stdout),
                stderr: truncate_output(&stderr),
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => {
            let body = CommandResponse {
                command_id: action.id().to_string(),
                success: false,
                exit_code: 1,
                stdout: String::new(),
                stderr: e.to_string(),
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
        }
    }
}

/// Check that a container name appears in `docker ps -a` output, without executing
/// any user-controlled input as a shell argument (the name was already validated by
/// `is_valid_identifier` before this is called).
async fn container_exists(name: &str) -> Result<()> {
    let output = tokio::process::Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
        .await
        .with_context(|| "failed to run docker ps")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let found = stdout.lines().any(|line| line.trim() == name);
    if !found {
        anyhow::bail!("container '{}' not found in docker ps -a", name);
    }
    Ok(())
}

/// Run a fixed program + argv for the given `NodeAction`. No shell is ever invoked;
/// this is the entire execution surface of the agent (hard security rule 1, task H-06).
async fn execute_node_action(action: &NodeAction) -> Result<(i32, String, String)> {
    let (program, args) = match action {
        NodeAction::ServiceRestart { name, .. } => ("systemctl", vec!["restart", name.as_str()]),
        NodeAction::ServiceStart { name, .. } => ("systemctl", vec!["start", name.as_str()]),
        NodeAction::ServiceStop { name, .. } => ("systemctl", vec!["stop", name.as_str()]),
        NodeAction::ServiceStatus { name, .. } => ("systemctl", vec!["status", name.as_str()]),
        NodeAction::ContainerList { .. } => ("docker", vec!["ps", "-a"]),
        NodeAction::ContainerRestart { name, .. } => ("docker", vec!["restart", name.as_str()]),
        NodeAction::DiskUsage { .. } => ("df", vec!["-h"]),
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(COMMAND_TIMEOUT_SECS),
        tokio::process::Command::new(program).args(&args).output(),
    )
    .await;

    let output = match result {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(anyhow::anyhow!("failed to spawn '{}': {}", program, e)),
        Err(_) => {
            return Err(anyhow::anyhow!(
                "{} timed out after {}s",
                program,
                COMMAND_TIMEOUT_SECS
            ))
        }
    };

    let exit_code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((exit_code, stdout, stderr))
}

async fn collect_heartbeat(state: &AppState) -> Heartbeat {
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::new().with_cpu_usage())
            .with_memory(MemoryRefreshKind::new().with_ram()),
    );
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let cpu_pct = sys.global_cpu_usage();
    let ram_used_mb = sys.used_memory() / (1024 * 1024);
    let ram_total_mb = sys.total_memory() / (1024 * 1024);

    let disks = Disks::new_with_refreshed_list();
    let mut disk_used_bytes: u64 = 0;
    let mut disk_total_bytes: u64 = 0;
    for d in disks.list() {
        disk_total_bytes += d.total_space();
        disk_used_bytes += d.total_space().saturating_sub(d.available_space());
    }
    let disk_used_gb = disk_used_bytes / (1024 * 1024 * 1024);
    let disk_total_gb = disk_total_bytes / (1024 * 1024 * 1024);

    let services = state
        .config
        .services
        .iter()
        .map(|s| ServiceStatus {
            name: s.name.clone(),
            running: true, // simplified probe
            port: s.port,
            message: "operational".to_string(),
        })
        .collect();

    Heartbeat {
        node_id: state.config.node_id.clone(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        uptime_secs: state.start_time.elapsed().as_secs(),
        cpu_pct,
        ram_used_mb,
        ram_total_mb,
        disk_used_gb,
        disk_total_gb,
        services,
        version: env!("CARGO_PKG_VERSION").to_string(),
        signature: None,
    }
}

/// H-05: sign a heartbeat with HMAC-SHA256 using the per-node secret.
/// Returns `None` when no secret is configured (unsigned legacy heartbeat).
fn sign_heartbeat(hb: &mut Heartbeat, secret: &str) {
    if secret.is_empty() {
        return;
    }
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("⚠ Failed to initialize HMAC with secret: {e}");
            return;
        }
    };
    mac.update(heartbeat_canonical_json(hb).as_bytes());
    hb.signature = Some(hex::encode(mac.finalize().into_bytes()));
}

async fn heartbeat_loop(state: Arc<AppState>) {
    let interval = std::time::Duration::from_secs(state.config.heartbeat_interval_secs);
    let client = reqwest::Client::new();
    loop {
        tokio::time::sleep(interval).await;
        let mut hb = collect_heartbeat(&state).await;
        sign_heartbeat(&mut hb, &state.config.secret);
        let url = format!(
            "{}/api/node/heartbeat",
            state.config.central_url.trim_end_matches('/')
        );
        let mut req = client.post(&url).json(&hb);
        if !state.config.token.is_empty() {
            req = req.header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", state.config.token),
            );
        }
        let res = req.send().await;
        match res {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => eprintln!("⚠ Heartbeat report returned status {}", r.status()),
            Err(e) => eprintln!("⚠ Failed sending heartbeat to {}: {}", url, e),
        }
    }
}

async fn check_status(port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/api/health", port);
    match reqwest::get(&url).await {
        Ok(res) if res.status().is_success() => {
            let body: serde_json::Value = res.json().await?;
            println!("✔ CAOrch agent is RUNNING on port {}", port);
            println!("  Node ID:     {}", body["node_id"].as_str().unwrap_or("-"));
            println!(
                "  Uptime:      {}s",
                body["uptime_secs"].as_u64().unwrap_or(0)
            );
            println!("  Version:     {}", body["version"].as_str().unwrap_or("-"));
            Ok(())
        }
        _ => {
            println!("✖ CAOrch agent is NOT running on port {}", port);
            std::process::exit(1);
        }
    }
}

fn enroll_node(central: Option<String>) -> Result<()> {
    use rand::rngs::OsRng;
    let signing_key = ed25519_dalek::SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let pubkey_hex = hex::encode(verifying_key.as_bytes());

    println!("=== CAOrch Node Enrollment ===");
    println!("Public Key: {}", pubkey_hex);
    if let Some(c) = central {
        println!("Central Server: {}", c);
        println!("\nTo register this node on central cashell, run:");
        println!(
            "cashell node register --node-id <id> --pubkey {}",
            pubkey_hex
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn identifier_rejects_shell_metacharacters() {
        assert!(!is_valid_identifier("nginx; id"));
        assert!(!is_valid_identifier("a;id"));
        assert!(!is_valid_identifier("../x"));
        assert!(!is_valid_identifier("a b"));
        assert!(!is_valid_identifier("$(id)"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("-h"));
        assert!(!is_valid_identifier("foo|bar"));
        assert!(!is_valid_identifier("foo&bar"));
        assert!(!is_valid_identifier("foo$bar"));
        let s = "a".repeat(128); // first char + 127 more = 128 total, still ok boundary
        assert!(is_valid_identifier(&s));
        let s = "a".repeat(129);
        assert!(!is_valid_identifier(&s));
    }

    #[test]
    fn identifier_accepts_valid_names() {
        assert!(is_valid_identifier("nginx"));
        assert!(is_valid_identifier("my-svc"));
        assert!(is_valid_identifier("my_svc"));
        assert!(is_valid_identifier("9001-proxy"));
        assert!(is_valid_identifier("a.b.c@-"));
    }

    #[test]
    fn is_authorized_requires_bearer_and_exact_match() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!is_authorized(&headers, "secret"));

        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert!(!is_authorized(&headers, "secret"));

        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret"),
        );
        assert!(is_authorized(&headers, "secret"));

        // empty configured token always fails closed, even if a header is present
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer anything"),
        );
        assert!(!is_authorized(&headers, ""));
    }

    #[test]
    fn const_time_eq_refuses_length_mismatch() {
        assert!(!const_time_eq("abc", "abcd"));
        assert!(const_time_eq("", ""));
        assert!(const_time_eq("token", "token"));
        assert!(!const_time_eq("token", "tokena"));
    }

    #[test]
    fn is_loopback_detects_common_values() {
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("::1"));
        assert!(is_loopback("localhost"));
        assert!(!is_loopback("0.0.0.0"));
        assert!(!is_loopback("10.0.0.5"));
    }

    #[tokio::test]
    async fn old_format_payload_rejected_as_bad_json() {
        // Simulates the exact legacy/attack payload from V-05:
        //   {"id":"t","action":"df","target":"-h","args":["; id"]}
        // Since NodeAction is now a serde-tagged enum, this old shape has no
        // `action` tag that matches any variant and must fail deserialization
        // (400/422 at the axum level, and no process is ever spawned).
        let raw = r#"{"id":"t","action":"df","target":"-h","args":["; id"]}"#;
        let res: Result<NodeAction, _> = serde_json::from_str(raw);
        assert!(res.is_err());

        // A well-formed new-format action must parse fine.
        let raw_ok = r#"{"id":"t","action":"disk_usage"}"#;
        let action: NodeAction = serde_json::from_str(raw_ok).unwrap();
        assert_eq!(action.id(), "t");
    }

    #[tokio::test]
    async fn malicious_service_name_rejected() {
        let raw = r#"{"id":"t","action":"service_restart","name":"nginx; id"}"#;
        let action: NodeAction = serde_json::from_str(raw).unwrap();
        let name = match action {
            NodeAction::ServiceRestart { name, .. } => name,
            _ => panic!("wrong variant"),
        };
        assert!(!is_valid_identifier(&name));
    }
}
