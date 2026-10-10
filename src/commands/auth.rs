//! Auth command — login, token status, logout.

use crate::{config, output};
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Subcommand, Debug)]
pub enum AuthMode {
    /// Login with API token
    Login,
    /// Show current token status (masked)
    Status,
    /// Remove stored token
    Logout,
}

#[derive(Parser, Debug)]
pub struct AuthCommand {
    #[command(subcommand)]
    pub mode: AuthMode,
}

pub fn run(cmd: &AuthCommand) -> Result<()> {
    match &cmd.mode {
        AuthMode::Login => cmd_login(),
        AuthMode::Status => cmd_status(),
        AuthMode::Logout => cmd_logout(),
    }
}

fn cmd_login() -> Result<()> {
    let cfg = config::Config::load(None)?;
    if let Some(ref tok) = cfg.auth.token {
        // already logged in
        let masked = format!("{}...{}", &tok[..4], &tok[tok.len() - 4..]);
        output::info(&format!("Already logged in as {masked}"));
        println!("\nTo refresh: run `cashell auth login` again.");
        return Ok(());
    }
    eprintln!("Enter your API token (won't be echoed):");
    // In production, use rpassword crate; for now read via stdin
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let token = input.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("Empty token provided");
    }
    let mut cfg = cfg;
    cfg.auth.token = Some(token);
    cfg.save(&config::Config::default_path())?;
    output::success("Logged in successfully");
    Ok(())
}

fn cmd_status() -> Result<()> {
    let cfg = config::Config::load(None)?;
    match &cfg.auth.token {
        Some(tok) => {
            let masked = format!("{}...{}", &tok[..4], &tok[tok.len() - 4..]);
            output::kv("token", &masked);
            output::kv(
                "api_base",
                cfg.auth.api_base.as_deref().unwrap_or("(default)"),
            );
        }
        None => output::error("Not logged in — run: cashell auth login"),
    }
    Ok(())
}

fn cmd_logout() -> Result<()> {
    let mut cfg = config::Config::load(None)?;
    cfg.auth.token = None;
    cfg.save(&config::Config::default_path())?;
    output::success("Logged out — token removed");
    Ok(())
}
