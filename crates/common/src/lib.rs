//! cashell-common — Shared types for the CA ecosystem.
//!
//! Used by both `cashell` (central) and `caorch` (agent).

use serde::{Deserialize, Serialize};

/// Heartbeat payload sent from a node agent to cashell central.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub node_id: String,
    pub timestamp: String,
    pub uptime_secs: u64,
    pub cpu_pct: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub disk_used_gb: u64,
    pub disk_total_gb: u64,
    pub services: Vec<ServiceStatus>,
    pub version: String,
}

/// Status of a single monitored service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub name: String,
    pub running: bool,
    pub port: Option<u16>,
    pub message: String,
}

/// Command sent from cashell central to a node agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCommand {
    pub id: String,
    pub action: String,
    pub target: String,
    pub args: Vec<String>,
}

/// Response after executing a command on the node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    pub command_id: String,
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
