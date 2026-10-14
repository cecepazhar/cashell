//! cashell — CLI entry point
//!
//! Subcommands:
//!   auth      Login & manage tokens
//!   product   Query/list CA products metadata
//!   version   Print version info
//!   config    View/edit config file
//!   git-status   Show git status for all tracked repositories
//!   deploy       Deploy an application via Coolify
//!   doctor       System health diagnostics (REQ-P2-01)
//!   ai           Multi-vendor AI assistant (REQ-P2-02)

use anyhow::Result;
use clap::Parser;

use cashell::banner;
use cashell::cli::{Cashell, Commands};
use cashell::commands;

fn main() -> Result<()> {
    let args = Cashell::parse();
    banner::print(args.quiet);
    let cfg = commands::load_config(&args.config_file)?;
    match args.command {
        Commands::Auth(cmd) => commands::auth::run(&cmd)?,
        Commands::Product(cmd) => commands::product::run(&cmd)?,
        Commands::Version(cmd) => commands::version::run(&cmd)?,
        Commands::Config(cmd) => commands::config::run(&cmd)?,
        Commands::Git(cmd) => {
            let code = commands::git::run(&cmd, &cfg)?;
            std::process::exit(code);
        }
        Commands::GitStatus(cmd) => {
            let code = commands::git_status::run(&cmd, &cfg)?;
            std::process::exit(code);
        }
        Commands::Deploy(cmd) => {
            let result = commands::deploy::run(&cmd, &cfg)?;
            if !result.success {
                eprintln!("✗ Deployment failed: {}", result.error.unwrap_or_default());
                std::process::exit(2);
            }
            cashell::output::success(&format!(
                "Deployment successful in {:.1}s",
                result.duration_seconds
            ));
        }
        Commands::Doctor(cmd) => {
            let code = commands::doctor::run(&cmd)?;
            std::process::exit(code);
        }
        Commands::Ai(cmd) => cashell::ai::run(&cmd, &cfg)?,
        Commands::Audit(cmd) => {
            let code = commands::audit::run(&cmd, &cfg, args.quiet)?;
            std::process::exit(code);
        }
        Commands::Review(cmd) => commands::review::run(&cmd, &cfg)?,
        Commands::R2(cmd) => {
            let code = commands::r2::run(&cmd)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Commands::Vault(cmd) => {
            let code = commands::vault::run(&cmd)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Commands::Server(cmd) => commands::server::run(&cmd, &cfg)?,
    }
    Ok(())
}
