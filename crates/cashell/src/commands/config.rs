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
