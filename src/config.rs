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
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub auth: AuthConfig,
    pub output: OutputConfig,
    pub products: ProductsConfig,
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
        let path: std::path::PathBuf = match path {
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
