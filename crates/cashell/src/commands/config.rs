//! Config command — view, edit, reset config.

use crate::{config, output};
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show current config
    Show,
    /// Set a config value (key = value).
    ///
    /// AI keys: ai.provider, ai.model, ai.api_key, ai.base_url
    /// Other:   api_base, theme, catalog
    Set { key: String, value: String },
    /// Reset config to defaults
    Reset,
    /// Import configuration, servers, uptime monitors, and analytics websites from a TOML file
    Import {
        /// Path to profile/config file to import
        path: std::path::PathBuf,
    },
    /// Export current servers, uptime monitors, and analytics websites to a TOML profile
    Export {
        /// Destination path to export profile
        path: std::path::PathBuf,
    },
}

#[derive(Parser, Debug)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub action: ConfigAction,
}

pub fn run(cmd: &ConfigCommand) -> Result<()> {
    match &cmd.action {
        ConfigAction::Show => cmd_show(),
        ConfigAction::Set { key, value } => cmd_set(key, value),
        ConfigAction::Reset => cmd_reset(),
        ConfigAction::Import { path } => cmd_import(path),
        ConfigAction::Export { path } => cmd_export(path),
    }
}

fn mask_key(key: &str) -> String {
    if key.len() >= 8 {
        format!("{}...{}", &key[..4], &key[key.len() - 4..])
    } else if key.is_empty() {
        "(unset)".to_string()
    } else {
        "****".to_string()
    }
}

fn cmd_show() -> Result<()> {
    let cfg = config::Config::load(None)?;
    let path = config::Config::default_path();
    let path_str = path.display().to_string();
    output::kv("config_path", &path_str);
    if let Some(tok) = &cfg.auth.token {
        output::kv("auth_token", &mask_key(tok));
    } else {
        output::kv("auth_token", "(unset)");
    }
    let api_base = cfg
        .auth
        .api_base
        .clone()
        .unwrap_or_else(|| "(default)".to_string());
    output::kv("api_base", &api_base);
    output::kv("theme", &cfg.output.theme);
    let catalog = cfg
        .products
        .default_catalog
        .clone()
        .unwrap_or_else(|| "(default)".to_string());
    output::kv("catalog", &catalog);

    // AI section
    let provider = &cfg.ai.default_provider;
    output::kv("ai.provider", provider);
    if let Some(p) = cfg.ai.providers.get(provider.as_str()) {
        output::kv("ai.base_url", &p.base_url);
        output::kv(
            "ai.model",
            if p.default_model.is_empty() {
                "(unset)"
            } else {
                &p.default_model
            },
        );
        // Resolve env: indirection for display only, then mask.
        let raw_key = match p.api_key.strip_prefix("env:") {
            Some(var) => std::env::var(var).unwrap_or_default(),
            None => p.api_key.clone(),
        };
        output::kv("ai.api_key", &mask_key(&raw_key));
    }
    Ok(())
}

fn cmd_set(key: &str, value: &str) -> Result<()> {
    let mut cfg = config::Config::load(None)?;
    match key {
        "api_base" => cfg.auth.api_base = Some(value.to_string()),
        "theme" => cfg.output.theme = value.to_string(),
        "catalog" => cfg.products.default_catalog = Some(value.to_string()),
        "ai.provider" => {
            cfg.ai.default_provider = value.to_string();
            cfg.ai.fallback_chain = vec![value.to_string()];
            // Ensure the provider slot exists so subsequent ai.model/api_key writes land.
            cfg.ai
                .providers
                .entry(value.to_string())
                .or_insert_with(Default::default);
        }
        "ai.model" => {
            let pname = cfg.ai.default_provider.clone();
            cfg.ai
                .providers
                .entry(pname)
                .or_insert_with(Default::default)
                .default_model = value.to_string();
        }
        "ai.api_key" => {
            let pname = cfg.ai.default_provider.clone();
            // Store verbatim (no env: prefix) — user wants manual entry.
            cfg.ai
                .providers
                .entry(pname)
                .or_insert_with(Default::default)
                .api_key = value.to_string();
        }
        "ai.base_url" => {
            let pname = cfg.ai.default_provider.clone();
            cfg.ai
                .providers
                .entry(pname)
                .or_insert_with(Default::default)
                .base_url = value.to_string();
        }
        k => anyhow::bail!("Unknown config key: {k}"),
    }
    cfg.save(&config::Config::default_path())?;
    output::success(&format!("Set {key} = {value}"));
    Ok(())
}

fn cmd_reset() -> Result<()> {
    let path = config::Config::default_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
        output::success("Config reset — removed config file");
    } else {
        output::info("No config file to reset");
    }
    Ok(())
}

fn cmd_import(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("Import file does not exist: {}", path.display());
    }

    let content = std::fs::read_to_string(path)?;
    let val: toml::Value = toml::from_str(&content)?;

    // 1. If it's a full Config TOML, merge into Config
    let mut cfg = config::Config::load(None).unwrap_or_default();

    // Import [daemon.nodes]
    let mut imported_nodes = 0;
    if let Some(nodes) = val
        .get("daemon")
        .and_then(|d| d.get("nodes"))
        .and_then(|n| n.as_table())
    {
        for (name, node_val) in nodes {
            if let Ok(node_cfg) = node_val.clone().try_into::<config::NodeConfig>() {
                cfg.daemon.nodes.insert(name.clone(), node_cfg);
                imported_nodes += 1;
            }
        }
    } else if let Some(nodes) = val.get("nodes").and_then(|n| n.as_table()) {
        for (name, node_val) in nodes {
            if let Ok(node_cfg) = node_val.clone().try_into::<config::NodeConfig>() {
                cfg.daemon.nodes.insert(name.clone(), node_cfg);
                imported_nodes += 1;
            }
        }
    }

    // Import [uptime.custom_monitors]
    let mut imported_monitors = 0;
    if let Some(monitors) = val
        .get("uptime")
        .and_then(|u| u.get("custom_monitors"))
        .and_then(|m| m.as_array())
    {
        for m_val in monitors {
            if let Ok(m_cfg) = m_val.clone().try_into::<config::UptimeMonitorConfig>() {
                if !cfg
                    .uptime
                    .custom_monitors
                    .iter()
                    .any(|existing| existing.name == m_cfg.name || existing.target == m_cfg.target)
                {
                    cfg.uptime.custom_monitors.push(m_cfg);
                    imported_monitors += 1;
                }
            }
        }
    } else if let Some(monitors) = val.get("monitors").and_then(|m| m.as_array()) {
        for m_val in monitors {
            if let Ok(m_cfg) = m_val.clone().try_into::<config::UptimeMonitorConfig>() {
                if !cfg
                    .uptime
                    .custom_monitors
                    .iter()
                    .any(|existing| existing.name == m_cfg.name || existing.target == m_cfg.target)
                {
                    cfg.uptime.custom_monitors.push(m_cfg);
                    imported_monitors += 1;
                }
            }
        }
    }

    // Save updated Config
    cfg.save(&config::Config::default_path())?;

    // 2. Import analytics_websites into SQLite DB if present
    let mut imported_sites = 0;
    if let Some(sites) = val.get("analytics_websites").and_then(|s| s.as_array()) {
        if let Ok(db) = crate::db::Database::open() {
            for s in sites {
                if let (Some(name), Some(domain)) = (
                    s.get("name").and_then(|v| v.as_str()),
                    s.get("domain").and_then(|v| v.as_str()),
                ) {
                    let id = s
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(name)
                        .to_lowercase()
                        .replace(' ', "-");
                    if db.add_analytic_site(&id, name, domain).is_ok() {
                        imported_sites += 1;
                    }
                }
            }
        }
    }

    output::success(&format!(
        "Imported from {}: {} nodes, {} uptime monitors, {} analytics sites",
        path.display(),
        imported_nodes,
        imported_monitors,
        imported_sites
    ));

    Ok(())
}

fn cmd_export(path: &std::path::Path) -> Result<()> {
    let cfg = config::Config::load(None).unwrap_or_default();

    #[derive(serde::Serialize)]
    struct ExportProfile {
        nodes: std::collections::HashMap<String, config::NodeConfig>,
        monitors: Vec<config::UptimeMonitorConfig>,
        analytics_websites: Vec<crate::systat::AnalyticSiteItem>,
    }

    let analytics_websites = if let Ok(db) = crate::db::Database::open() {
        db.get_analytic_sites().unwrap_or_default()
    } else {
        vec![]
    };

    let profile = ExportProfile {
        nodes: cfg.daemon.nodes,
        monitors: cfg.uptime.custom_monitors,
        analytics_websites,
    };

    let toml_str = toml::to_string_pretty(&profile)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, toml_str)?;

    output::success(&format!("Exported current profile to {}", path.display()));
    Ok(())
}
