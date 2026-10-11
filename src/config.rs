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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    pub token: Option<String>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutputConfig {
    pub theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProductsConfig {
    pub default_catalog: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployConfig {
    pub coolify: Option<CoolifyConfig>,
    #[serde(default)]
    pub apps: std::collections::HashMap<String, AppDeployConfig>,
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
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            scan_dirs: default_scan_dirs(),
            ignore_patterns: default_ignore_patterns(),
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
    "omniroute".to_string()
}
fn default_ai_timeout() -> u64 {
    30
}
fn default_ai_max_retries() -> u32 {
    1
}
fn default_fallback_chain() -> Vec<String> {
    vec![
        "omniroute".to_string(),
        "openai".to_string(),
        "anthropic".to_string(),
        "local".to_string(),
    ]
}
fn default_providers() -> std::collections::HashMap<String, AiProviderConfig> {
    let mut m = std::collections::HashMap::new();
    let mk = |base: &str, key: &str, model: &str| AiProviderConfig {
        base_url: base.to_string(),
        api_key: key.to_string(),
        default_model: model.to_string(),
    };
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
