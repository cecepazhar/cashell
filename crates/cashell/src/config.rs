//! Configuration management for cashell.
//!
//! Config file: ~/.config/cashell/config.toml
//!
//! ```toml
//! [auth]
//! token = ""
//! api_base = "https://api.fathforce.com"
//!
//! [output]
//! theme = "auto"
//!
//! [products]
//! default_catalog = "fathstore"
//!
//! [deploy.coolify]
//! base_url = "https://vps-hostinger.tail2a1bee.ts.net/api/v1"
//! api_token = "env:COOLIFY_API_TOKEN"
//! timeout_seconds = 60
//!
//! [deploy.apps]
//! caterm = { uuid = "caterm-app-uuid", type = "application" }
//!
//! [workspace]
//! scan_dirs = ["~/Product", "~/Project"]
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub products: ProductsConfig,
    #[serde(default)]
    pub deploy: DeployConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub doctor: DoctorConfig,
    #[serde(default)]
    pub persona: PersonaConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub uptime: UptimeConfig,
    #[serde(default)]
    pub docker: DockerConfig,
    #[serde(default)]
    pub security: SecurityConfig,
}

/// Daemon & multi-node orchestration configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_daemon_port")]
    pub port: u16,
    #[serde(default = "default_daemon_token")]
    pub token: String,
    #[serde(default = "default_daemon_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_daemon_retry_threshold")]
    pub retry_threshold: u32,
    #[serde(default = "default_ssh_key_path")]
    pub ssh_key_path: String,
    #[serde(default)]
    pub telegram_bot_token: String,
    #[serde(default)]
    pub telegram_chat_id: String,
    #[serde(default = "default_nodes")]
    pub nodes: std::collections::HashMap<String, NodeConfig>,
    #[serde(default = "default_remediation")]
    pub remediation: std::collections::HashMap<String, String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            port: default_daemon_port(),
            token: default_daemon_token(),
            interval_secs: default_daemon_interval(),
            retry_threshold: default_daemon_retry_threshold(),
            ssh_key_path: default_ssh_key_path(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            nodes: default_nodes(),
            remediation: default_remediation(),
        }
    }
}

/// Remote node configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub ip: String,
    pub user: String,
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
    /// If false, this node is marked as disabled/non-active and is skipped
    /// during probes, latencies, and fleet management.
    #[serde(default = "default_node_active")]
    pub active: bool,
}

fn default_node_active() -> bool {
    true
}

/// Service configuration on a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_daemon_port() -> u16 {
    8090
}

fn default_daemon_token() -> String {
    String::new()
}

fn default_daemon_interval() -> u64 {
    60
}

fn default_daemon_retry_threshold() -> u32 {
    3
}

fn default_ssh_key_path() -> String {
    "~/.ssh/id_ed25519".to_string()
}

fn default_protocol() -> String {
    "tcp".to_string()
}

fn default_nodes() -> std::collections::HashMap<String, NodeConfig> {
    let mut m = std::collections::HashMap::new();
    m.insert(
        "ypc".to_string(),
        NodeConfig {
            ip: "100.127.232.32".to_string(),
            user: "cecep".to_string(),
            services: vec![
                ServiceConfig {
                    name: "gitea".to_string(),
                    port: 3000,
                    protocol: "http".to_string(),
                },
                ServiceConfig {
                    name: "nextcloud".to_string(),
                    port: 443,
                    protocol: "https".to_string(),
                },
                ServiceConfig {
                    name: "tailscaled".to_string(),
                    port: 22,
                    protocol: "tcp".to_string(),
                },
            ],
            active: true,
        },
    );
    m.insert(
        "hostinger".to_string(),
        NodeConfig {
            ip: "100.76.150.46".to_string(),
            user: "root".to_string(),
            services: vec![ServiceConfig {
                name: "tailscale-funnel".to_string(),
                port: 8443,
                protocol: "https".to_string(),
            }],
            active: true,
        },
    );
    m.insert(
        "local".to_string(),
        NodeConfig {
            ip: "127.0.0.1".to_string(),
            user: "cecepazhar".to_string(),
            services: vec![ServiceConfig {
                name: "9router".to_string(),
                port: 20128,
                protocol: "http".to_string(),
            }],
            active: true,
        },
    );
    m.insert(
        "tencent".to_string(),
        NodeConfig {
            ip: "100.96.8.82".to_string(),
            user: "root".to_string(),
            services: vec![
                ServiceConfig {
                    name: "caorch-agent".to_string(),
                    port: 8091,
                    protocol: "http".to_string(),
                },
                ServiceConfig {
                    name: "tailscaled".to_string(),
                    port: 22,
                    protocol: "tcp".to_string(),
                },
            ],
            active: true,
        },
    );
    m
}

fn default_remediation() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    m.insert("ypc.gitea".to_string(), "podman restart gitea".to_string());
    m.insert(
        "hostinger.tailscale-funnel".to_string(),
        "systemctl restart cloudflared".to_string(),
    );
    m
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    pub token: Option<String>,
    pub api_base: Option<String>,
}

pub const PRESET_THEMES: &[&str] = &[
    "ca-classic",
    "tokyo-night",
    "monokai",
    "nord",
    "high-contrast",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
        }
    }
}

fn default_theme() -> String {
    "ca-classic".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductsConfig {
    pub default_catalog: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployConfig {
    pub coolify: Option<CoolifyConfig>,
    #[serde(default = "default_deploy_apps")]
    pub apps: std::collections::HashMap<String, AppDeployConfig>,
}

fn default_deploy_apps() -> std::collections::HashMap<String, AppDeployConfig> {
    let mut m = std::collections::HashMap::new();
    // Default CA ecosystem apps. Replace UUIDs with real Coolify instance UUIDs.
    m.insert(
        "caterm".to_string(),
        AppDeployConfig {
            uuid: "caterm-app-uuid".to_string(),
            app_type: "application".to_string(),
        },
    );
    m.insert(
        "fathstore".to_string(),
        AppDeployConfig {
            uuid: "fathstore-app-uuid".to_string(),
            app_type: "application".to_string(),
        },
    );
    m.insert(
        "gcc".to_string(),
        AppDeployConfig {
            uuid: "gcc-app-uuid".to_string(),
            app_type: "application".to_string(),
        },
    );
    m
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoolifyConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_token: String,
    #[serde(default = "default_coolify_timeout")]
    pub timeout_seconds: u64,
}

fn default_coolify_timeout() -> u64 {
    120
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppDeployConfig {
    #[serde(default)]
    pub uuid: String,
    #[serde(rename = "type", default = "default_app_type")]
    pub app_type: String,
}

fn default_app_type() -> String {
    "application".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default = "default_scan_dirs")]
    pub scan_dirs: Vec<String>,
    #[serde(default = "default_ignore_patterns")]
    pub ignore_patterns: Vec<String>,
    #[serde(default)]
    pub custom_projects: Vec<String>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            scan_dirs: default_scan_dirs(),
            ignore_patterns: default_ignore_patterns(),
            custom_projects: Vec::new(),
        }
    }
}

fn default_scan_dirs() -> Vec<String> {
    vec!["~/Product".to_string(), "~/Project".to_string()]
}

fn default_ignore_patterns() -> Vec<String> {
    vec![
        "target".to_string(),
        "node_modules".to_string(),
        ".git".to_string(),
        "_archive".to_string(),
        "tmp".to_string(),
    ]
}

/// Uptime Kuma endpoint monitor configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UptimeConfig {
    /// Custom Uptime Kuma endpoint monitors added via the cockpit wizard.
    #[serde(default)]
    pub custom_monitors: Vec<UptimeMonitorConfig>,
}

/// A custom Uptime Kuma style endpoint monitor definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UptimeMonitorConfig {
    pub name: String,
    pub target: String,
}

/// Docker container runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DockerConfig {
    /// Containers started via the cockpit "Add Docker Container" wizard.
    #[serde(default)]
    pub containers: Vec<DockerContainerConfig>,
}

/// A Docker container launched through the cockpit wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContainerConfig {
    pub name: String,
    pub image: String,
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub started_at: String,
}

/// AI layer config (REQ-P2-02). All fields serde-defaulted so old configs load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_ai_provider")]
    pub default_provider: String,
    #[serde(default = "default_ai_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_ai_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_fallback_chain")]
    pub fallback_chain: Vec<String>,
    #[serde(default = "default_providers")]
    pub providers: std::collections::HashMap<String, AiProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiProviderConfig {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub default_model: String,
}

fn default_ai_provider() -> String {
    "9router".to_string()
}
fn default_ai_timeout() -> u64 {
    30
}
fn default_ai_max_retries() -> u32 {
    1
}
fn default_fallback_chain() -> Vec<String> {
    vec!["9router".to_string()]
}
fn default_providers() -> std::collections::HashMap<String, AiProviderConfig> {
    let mut m = std::collections::HashMap::new();
    let mk = |base: &str, key: &str, model: &str| AiProviderConfig {
        base_url: base.to_string(),
        api_key: key.to_string(),
        default_model: model.to_string(),
    };
    m.insert(
        "9router".to_string(),
        mk("http://localhost:20128/v1", "", ""),
    );
    m.insert(
        "omniroute".to_string(),
        mk(
            "http://100.76.150.46:3007/v1",
            "env:OMNIROUTE_API_KEY",
            "auto",
        ),
    );
    m.insert(
        "openai".to_string(),
        mk("https://api.openai.com/v1", "env:OPENAI_API_KEY", "gpt-4o"),
    );
    m.insert(
        "anthropic".to_string(),
        mk(
            "https://api.anthropic.com/v1",
            "env:ANTHROPIC_API_KEY",
            "claude-3-5-sonnet-20241022",
        ),
    );
    m.insert(
        "local".to_string(),
        mk("http://127.0.0.1:11434/api", "", "qwen2.5-coder:7b"),
    );
    m
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            default_provider: default_ai_provider(),
            timeout_seconds: default_ai_timeout(),
            max_retries: default_ai_max_retries(),
            fallback_chain: default_fallback_chain(),
            providers: default_providers(),
        }
    }
}

/// Diagnostics configuration for `cashell doctor` (REQ-P2-01).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorConfig {
    #[serde(default = "default_doctor_target")]
    pub target_addr: String,
    #[serde(default = "default_doctor_target_port")]
    pub target_port: u16,
    #[serde(default = "default_doctor_ports")]
    pub ports: Vec<u16>,
    #[serde(default = "default_storage_mounts")]
    pub storage_mounts: Vec<String>,
    #[serde(default = "default_toolchain")]
    pub toolchain: Vec<String>,
}

impl Default for DoctorConfig {
    fn default() -> Self {
        Self {
            target_addr: default_doctor_target(),
            target_port: default_doctor_target_port(),
            ports: default_doctor_ports(),
            storage_mounts: default_storage_mounts(),
            toolchain: default_toolchain(),
        }
    }
}

fn default_doctor_target() -> String {
    "100.76.150.46".to_string()
}

fn default_doctor_target_port() -> u16 {
    443
}

fn default_doctor_ports() -> Vec<u16> {
    vec![8081, 8082, 8083, 8084, 8085, 8086, 8087, 8088]
}

fn default_storage_mounts() -> Vec<String> {
    if std::path::Path::new("/run/media/cecepazhar/DATA").exists() {
        vec!["/".to_string(), "/run/media/cecepazhar/DATA".to_string()]
    } else {
        vec!["/".to_string()]
    }
}

fn default_toolchain() -> Vec<String> {
    vec![
        "cargo".to_string(),
        "rustc".to_string(),
        "go".to_string(),
        "templ".to_string(),
        "pnpm".to_string(),
    ]
}

/// AI assistant persona and user identity configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaConfig {
    #[serde(default = "default_user_name")]
    pub user_name: String,
    #[serde(default = "default_assistant_role")]
    pub assistant_role: String,
}

impl Default for PersonaConfig {
    fn default() -> Self {
        Self {
            user_name: default_user_name(),
            assistant_role: default_assistant_role(),
        }
    }
}

fn default_user_name() -> String {
    "Prof. Cecep".to_string()
}

fn default_assistant_role() -> String {
    "Kamu adalah Hana AI / Chief Operations Assistant Fathforce & CAShell. Terse, cekatan, dan patuh pada instruksi Prof. Cecep.".to_string()
}

/// SQLite database persistence & retention policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_auto_prune")]
    pub auto_prune: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            retention_days: default_retention_days(),
            auto_prune: default_auto_prune(),
        }
    }
}

fn default_retention_days() -> u32 {
    7
}

fn default_auto_prune() -> bool {
    true
}

impl Config {
    /// Returns the default config file path.
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("cashell")
            .join("config.toml")
    }

    /// Load config from file (or create default).
    pub fn load(path: Option<&Path>) -> Result<Config> {
        let path: PathBuf = match path {
            Some(p) => p.to_path_buf(),
            None => Self::default_path(),
        };
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let cfg: Config = toml::from_str(&content)?;
            Ok(cfg)
        } else {
            let cfg = Config::default();
            Self::save(&cfg, &path)?;
            Ok(cfg)
        }
    }

    /// Save config to file.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// Security and master password configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecurityConfig {
    #[serde(default)]
    pub require_password: bool,
    #[serde(default)]
    pub master_password_hash: String,
    #[serde(default)]
    pub hint: String,
}

use sha2::{Digest, Sha256};

/// Hash master password using SHA-256 with sovereign domain salt.
pub fn hash_master_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"cashell_master_salt_sovereign_2026_");
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Verify an input password against stored hash.
pub fn verify_master_password(password: &str, stored_hash: &str) -> bool {
    if stored_hash.is_empty() {
        return true;
    }
    hash_master_password(password) == stored_hash
}
