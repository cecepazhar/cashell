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
    let cfg = commands::load_config(&args.config_file)?;
    match args.command {
        None => {
            if cashell::output::is_tty() {
                commands::cockpit::run(&commands::cockpit::CockpitCommand, &cfg)?;
            } else {
                banner::print(args.quiet);
                println!("cashell v{} — CA ecosystem CLI", env!("CARGO_PKG_VERSION"));
                println!("Run 'cashell --help' for usage or run in an interactive terminal for CAS Cockpit.");
            }
        }
        Some(Commands::Cockpit(cmd)) => {
            commands::cockpit::run(&cmd, &cfg)?;
        }
        Some(Commands::Auth(cmd)) => {
            banner::print(args.quiet);
            commands::auth::run(&cmd)?;
        }
        Some(Commands::Product(cmd)) => {
            banner::print(args.quiet);
            commands::product::run(&cmd)?;
        }
        Some(Commands::Version(cmd)) => {
            banner::print(args.quiet);
            commands::version::run(&cmd)?;
        }
        Some(Commands::Config(cmd)) => {
            banner::print(args.quiet);
            commands::config::run(&cmd)?;
        }
        Some(Commands::Git(cmd)) => {
            banner::print(args.quiet);
            let code = commands::git::run(&cmd, &cfg)?;
            std::process::exit(code);
        }
        Some(Commands::GitStatus(cmd)) => {
            banner::print(args.quiet);
            let code = commands::git_status::run(&cmd, &cfg)?;
            std::process::exit(code);
        }
        Some(Commands::Deploy(cmd)) => {
            banner::print(args.quiet);
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
        Some(Commands::Doctor(cmd)) => {
            banner::print(args.quiet);
            let code = commands::doctor::run(&cmd, &cfg)?;
            std::process::exit(code);
        }
        Some(Commands::Ai(cmd)) => {
            banner::print(args.quiet);
            cashell::ai::run(&cmd, &cfg)?;
        }
        Some(Commands::Audit(cmd)) => {
            banner::print(args.quiet);
            let code = commands::audit::run(&cmd, &cfg, args.quiet)?;
            std::process::exit(code);
        }
        Some(Commands::Fix(cmd)) => {
            banner::print(args.quiet);
            let code = commands::fix::run(&cmd, &cfg)?;
            std::process::exit(code);
        }
        Some(Commands::Review(cmd)) => {
            banner::print(args.quiet);
            commands::review::run(&cmd, &cfg)?;
        }
        Some(Commands::R2(cmd)) => {
            banner::print(args.quiet);
            let code = commands::r2::run(&cmd)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Some(Commands::Vault(cmd)) => {
            banner::print(args.quiet);
            let code = commands::vault::run(&cmd)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Some(Commands::Server(cmd)) => {
            banner::print(args.quiet);
            commands::server::run(&cmd, &cfg)?;
        }
        Some(Commands::Chat(cmd)) => {
            commands::chat::run(&cmd, &cfg)?;
        }
    }
    Ok(())
}
