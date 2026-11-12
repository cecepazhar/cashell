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

/// Typed node action sent from cashell central to a node agent.
/// Replaces the old string-based `NodeCommand { action, target, args }` which allowed
/// arbitrary shell injection (finding S1 / task H-06). Each variant maps to a fixed
/// program + argv in the agent; no shell is ever used.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum NodeAction {
    /// `systemctl restart <name>` — `name` must match `^[a-zA-Z0-9][a-zA-Z0-9_.@-]{0,127}$`
    /// and must be present in the agent's configured `services` list.
    ServiceRestart { id: String, name: String },
    /// `systemctl start <name>` — same validation as `ServiceRestart`.
    ServiceStart { id: String, name: String },
    /// `systemctl stop <name>` — same validation as `ServiceRestart`.
    ServiceStop { id: String, name: String },
    /// `systemctl status <name>` — same validation as `ServiceRestart`.
    ServiceStatus { id: String, name: String },
    /// `docker ps -a` — no arguments, nothing to validate.
    ContainerList { id: String },
    /// `docker restart <name>` — `name` must match `^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$`
    /// and must be present in `docker ps -a` output.
    ContainerRestart { id: String, name: String },
    /// `df -h` — no arguments, nothing to validate.
    DiskUsage { id: String },
}

impl NodeAction {
    /// The command id, shared by every variant.
    pub fn id(&self) -> &str {
        match self {
            Self::ServiceRestart { id, .. } => id,
            Self::ServiceStart { id, .. } => id,
            Self::ServiceStop { id, .. } => id,
            Self::ServiceStatus { id, .. } => id,
            Self::ContainerList { id } => id,
            Self::ContainerRestart { id, .. } => id,
            Self::DiskUsage { id } => id,
        }
    }
}

/// Response after executing a node action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    pub command_id: String,
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
