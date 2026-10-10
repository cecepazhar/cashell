//! Config command — view, edit, reset config.

use crate::{config, output};
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Show current config
    Show,
    /// Set a config value (key = value)
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

fn cmd_show() -> Result<()> {
    let cfg = config::Config::load(None)?;
    let path = config::Config::default_path();
    let path_str = path.display().to_string();
    output::kv("config_path", &path_str);
    if let Some(tok) = &cfg.auth.token {
        let masked = format!("{}...{}", &tok[..4], &tok[tok.len() - 4..]);
        output::kv("auth_token", &masked);
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
    Ok(())
}

fn cmd_set(key: &str, value: &str) -> Result<()> {
    let mut cfg = config::Config::load(None)?;
    match key {
        "api_base" => cfg.auth.api_base = Some(value.to_string()),
        "theme" => cfg.output.theme = value.to_string(),
        "catalog" => cfg.products.default_catalog = Some(value.to_string()),
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
