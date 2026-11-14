//! Fleet probing, multi-node monitoring, and automated remediation.

use crate::config::NodeConfig;
use std::collections::HashMap;

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

pub async fn probe_all(nodes: &HashMap<String, NodeConfig>) -> Vec<(String, String, ProbeResult)> {
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
        let home = crate::paths::home_dir();
        return home.join(stripped).to_string_lossy().to_string();
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
        // Local execution: tokenize into argv, no shell involved (hardened path).
        let tokens: Vec<&str> = cmd.split_whitespace().collect();
        if tokens.is_empty() {
            return Err("empty remediation command".to_string());
        }
        let out = tokio::process::Command::new(tokens[0])
            .args(&tokens[1..])
            .output()
            .await
            .map_err(|e| format!("failed to spawn local command '{}': {e}", tokens[0]))?;
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
        // H-07: CAShell-specific known_hosts file so we never silently accept
        // arbitrary host keys (StrictHostKeyChecking=accept-new + UserKnownHostsFile).
        let known_hosts = dirs::config_dir()
            .map(|d| {
                d.join("cashell")
                    .join("ssh_known_hosts")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_else(|| "~/.config/cashell/ssh_known_hosts".to_string());
        let out = tokio::process::Command::new("ssh")
            .arg("-i")
            .arg(&key)
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg(format!("UserKnownHostsFile={known_hosts}"))
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

pub async fn send_telegram_alert(bot_token: &str, chat_id: &str, text: &str) -> Result<(), String> {
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
