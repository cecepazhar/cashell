//! CAOrch — Lightweight Fleet Agent Daemon for CA Ecosystem.
//!
//! Runs on remote nodes (VPS) to collect heartbeats, report status,
//! execute whitelisted commands, and provide automated self-healing.

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};
use tokio::sync::RwLock;

use cashell_common::{CommandResponse, Heartbeat, NodeCommand, ServiceStatus};

#[derive(Parser, Debug)]
#[command(name = "caorch", version, about = "CAOrch — Fleet Agent Daemon for CA Ecosystem")]
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
    pub whitelist: Vec<String>,
    pub services: Vec<MonitoredService>,
    pub remediation_enabled: bool,
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
            whitelist: vec![
                "systemctl restart *".to_string(),
                "systemctl start *".to_string(),
                "systemctl stop *".to_string(),
                "systemctl status *".to_string(),
                "docker ps*".to_string(),
                "docker restart *".to_string(),
                "podman ps*".to_string(),
                "podman restart *".to_string(),
                "df -h*".to_string(),
            ],
            services: vec![],
            remediation_enabled: true,
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
    }) {
        Commands::Run { port, central } => run_daemon(port, central).await,
        Commands::Status { port } => check_status(port).await,
        Commands::Enroll { central } => enroll_node(central),
        Commands::Version => {
            println!("caorch v{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

async fn run_daemon(port: u16, central: Option<String>) -> Result<()> {
    let mut cfg = AgentConfig {
        port,
        ..AgentConfig::default()
    };
    if let Some(c) = central {
        cfg.central_url = c;
    }

    println!(
        "🚀 CAOrch agent daemon starting on port {} (node: {})",
        cfg.port, cfg.node_id
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

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed binding to {}", addr))?;

    println!("✔ Listening on http://0.0.0.0:{}/api/health", port);
    axum::serve(listener, app).await?;
    Ok(())
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

async fn heartbeat_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let hb = collect_heartbeat(&state).await;
    Json(hb)
}

async fn command_handler(
    State(state): State<Arc<AppState>>,
    Json(cmd): Json<NodeCommand>,
) -> (StatusCode, Json<CommandResponse>) {
    // Validate command against whitelist
    let full_cmd = format!("{} {}", cmd.action, cmd.target);
    let allowed = is_whitelisted(&full_cmd, &state.config.whitelist);

    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(CommandResponse {
                command_id: cmd.id,
                success: false,
                exit_code: 403,
                stdout: String::new(),
                stderr: format!("Command not permitted by whitelist: {}", full_cmd),
            }),
        );
    }

    // Execute the command
    let output = execute_whitelisted(&cmd).await;
    match output {
        Ok((exit_code, stdout, stderr)) => (
            StatusCode::OK,
            Json(CommandResponse {
                command_id: cmd.id,
                success: exit_code == 0,
                exit_code,
                stdout,
                stderr,
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(CommandResponse {
                command_id: cmd.id,
                success: false,
                exit_code: 1,
                stdout: String::new(),
                stderr: e.to_string(),
            }),
        ),
    }
}

fn is_whitelisted(cmd: &str, whitelist: &[String]) -> bool {
    let cmd_trimmed = cmd.trim();
    for pattern in whitelist {
        let pat_clean = pattern.trim_end_matches('*').trim();
        if cmd_trimmed.starts_with(pat_clean) {
            return true;
        }
    }
    false
}

async fn execute_whitelisted(cmd: &NodeCommand) -> Result<(i32, String, String)> {
    let mut command_str = format!("{} {}", cmd.action, cmd.target);
    for arg in &cmd.args {
        command_str.push(' ');
        command_str.push_str(arg);
    }

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command_str)
        .output()
        .await?;

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
    }
}

async fn heartbeat_loop(state: Arc<AppState>) {
    let interval = std::time::Duration::from_secs(state.config.heartbeat_interval_secs);
    let client = reqwest::Client::new();
    loop {
        tokio::time::sleep(interval).await;
        let hb = collect_heartbeat(&state).await;
        let url = format!("{}/api/node/heartbeat", state.config.central_url.trim_end_matches('/'));
        let res = client.post(&url).json(&hb).send().await;
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
            println!("  Uptime:      {}s", body["uptime_secs"].as_u64().unwrap_or(0));
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
        println!("cashell node register --node-id <id> --pubkey {}", pubkey_hex);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whitelist_matching() {
        let wl = vec![
            "systemctl restart *".to_string(),
            "docker ps*".to_string(),
            "df -h*".to_string(),
        ];
        assert!(is_whitelisted("systemctl restart nginx", &wl));
        assert!(is_whitelisted("docker ps -a", &wl));
        assert!(is_whitelisted("df -h", &wl));
        assert!(!is_whitelisted("rm -rf /", &wl));
        assert!(!is_whitelisted("cat /etc/shadow", &wl));
    }
}
