//! Daemon and multi-node orchestration engine for cashell (F-05).
//!
//! Provides 24/7 fleet probing, automated SSH/local remediation,
//! Telegram alerts, Axum HTTP API, and systemd service management.

pub mod auth;
pub mod prober;
pub mod routes;
pub mod server;

#[cfg(test)]
mod tests;

pub use auth::*;
pub use prober::*;
pub use routes::*;
pub use server::*;

use crate::config::Config;
use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;

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

pub fn run_status(cfg: &Config) -> Result<()> {
    periodic_license_verify(cfg);

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
        println!(
            "{:<14} {:<20} {:<10} {:<10}",
            node, svc, status_str, lat_str
        );
    }

    println!(
        "\nFleet Health: {}/{} services operational.",
        up_count, total_count
    );
    Ok(())
}

pub fn run_restart(cfg: &Config, node: &str, service: &str) -> Result<()> {
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
                let _ = db.log_event("INFO", "daemon", &format!("Manual restart of {key}: {cmd}"));
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
