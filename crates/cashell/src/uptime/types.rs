//! Data types for Uptime Monitoring, Incident Lifecycle, and SSL Tracking (U-01..U-07).

use serde::{Deserialize, Serialize};

/// Supported probe types for uptime monitoring (U-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MonitorType {
    Http,
    Tcp,
    Ping,
}

impl std::str::FromStr for MonitorType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = match s.trim().to_ascii_lowercase().as_str() {
            "tcp" => Self::Tcp,
            "ping" | "icmp" => Self::Ping,
            _ => Self::Http,
        };
        Ok(t)
    }
}

impl MonitorType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Tcp => "tcp",
            Self::Ping => "ping",
        }
    }
}

/// Configuration for an individual uptime endpoint monitor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorConfig {
    pub name: String,
    pub target: String,
    #[serde(default = "default_monitor_type")]
    pub monitor_type: MonitorType,
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub expected_status: Option<u16>,
    #[serde(default)]
    pub expected_keyword: Option<String>,
    #[serde(default)]
    pub public: bool,
    #[serde(default = "default_group")]
    pub group: String,
}

fn default_monitor_type() -> MonitorType {
    MonitorType::Http
}

fn default_interval_secs() -> u64 {
    60
}

fn default_timeout_ms() -> u64 {
    5000
}

fn default_group() -> String {
    "Custom".to_string()
}

impl MonitorConfig {
    pub fn new_http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            target: url.into(),
            monitor_type: MonitorType::Http,
            interval_secs: 60,
            timeout_ms: 5000,
            expected_status: Some(200),
            expected_keyword: None,
            public: false,
            group: "Custom".to_string(),
        }
    }

    pub fn new_tcp(name: impl Into<String>, host_port: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            target: host_port.into(),
            monitor_type: MonitorType::Tcp,
            interval_secs: 60,
            timeout_ms: 5000,
            expected_status: None,
            expected_keyword: None,
            public: false,
            group: "Custom".to_string(),
        }
    }
}

/// Raw result of a single probe execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbeResult {
    pub monitor_name: String,
    pub target: String,
    pub is_up: bool,
    pub latency_ms: Option<u64>,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub cert_expires_at: Option<String>,
    pub cert_days_left: Option<i64>,
}

/// High-level lifecycle state of a monitored service (U-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MonitorState {
    Up,
    Degraded,
    Down,
}

impl MonitorState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Degraded => "degraded",
            Self::Down => "down",
        }
    }
}

/// State machine tracking consecutive probe successes/failures and anti-flapping (U-02).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentStateMachine {
    pub monitor_name: String,
    pub state: MonitorState,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub failure_threshold: u32,
    pub open_incident_id: Option<i64>,
}

impl IncidentStateMachine {
    pub fn new(monitor_name: impl Into<String>, failure_threshold: u32) -> Self {
        Self {
            monitor_name: monitor_name.into(),
            state: MonitorState::Up,
            consecutive_failures: 0,
            consecutive_successes: 0,
            failure_threshold: failure_threshold.max(1),
            open_incident_id: None,
        }
    }

    /// Process a new probe result and return transition events:
    /// - `Some(StateTransition::IncidentStarted { cause })` when transitioning to Down.
    /// - `Some(StateTransition::IncidentResolved { incident_id })` when transitioning from Down back to Up (after 2 consecutive successes).
    /// - `None` if no state change occurred.
    pub fn process_probe(&mut self, is_up: bool, error: Option<&str>) -> Option<StateTransition> {
        if is_up {
            self.consecutive_failures = 0;
            self.consecutive_successes += 1;

            match self.state {
                MonitorState::Up => None,
                MonitorState::Degraded => {
                    self.state = MonitorState::Up;
                    self.consecutive_successes = 0;
                    None
                }
                MonitorState::Down => {
                    // Anti-flapping: requires 2 consecutive successes before marking resolved (U-02)
                    if self.consecutive_successes >= 2 {
                        self.state = MonitorState::Up;
                        let incident_id = self.open_incident_id.take();
                        self.consecutive_successes = 0;
                        Some(StateTransition::IncidentResolved { incident_id })
                    } else {
                        None
                    }
                }
            }
        } else {
            self.consecutive_successes = 0;
            self.consecutive_failures += 1;

            match self.state {
                MonitorState::Up => {
                    if self.consecutive_failures >= self.failure_threshold {
                        self.state = MonitorState::Down;
                        let cause = error.unwrap_or("Probe check failed").to_string();
                        Some(StateTransition::IncidentStarted { cause })
                    } else {
                        self.state = MonitorState::Degraded;
                        None
                    }
                }
                MonitorState::Degraded => {
                    if self.consecutive_failures >= self.failure_threshold {
                        self.state = MonitorState::Down;
                        let cause = error.unwrap_or("Probe check failed").to_string();
                        Some(StateTransition::IncidentStarted { cause })
                    } else {
                        None
                    }
                }
                MonitorState::Down => None,
            }
        }
    }
}

/// Transition event emitted by [`IncidentStateMachine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateTransition {
    IncidentStarted { cause: String },
    IncidentResolved { incident_id: Option<i64> },
}

/// Comprehensive monitor status summary for Cockpit TUI, CLI `--json`, and HTML status page (U-04, U-05, U-07).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStatusSummary {
    pub name: String,
    pub target: String,
    pub group: String,
    pub state: MonitorState,
    pub is_up: bool,
    pub latency_ms: Option<u64>,
    pub status_code: Option<u16>,
    pub uptime_pct_24h: f32,
    pub uptime_pct_30d: f32,
    pub history_20: Vec<bool>,
    pub cert_expires_at: Option<String>,
    pub cert_days_left: Option<i64>,
    pub public: bool,
}
