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

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use cashell::banner;
use cashell::cli::{Cashell, Commands};
use cashell::commands;

fn main() -> ExitCode {
    match run_app() {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("Error: {err:?}");
            ExitCode::from(1)
        }
    }
}

fn run_app() -> Result<i32> {
    let args = Cashell::parse();
    let cfg = commands::load_config(&args.config_file)?;

    // Suppress banner for full-screen TUI or background daemon operations
    let suppress_banner = matches!(
        args.command,
        None | Some(Commands::Cockpit(_)) | Some(Commands::Chat(_)) | Some(Commands::Daemon(_))
    );
    if !suppress_banner {
        banner::print(args.quiet);
    }

    match args.command {
        None => {
            if cashell::output::is_tty() {
                // L-08: periodic license verification when cockpit opens.
                commands::daemon::periodic_license_verify(&cfg);
                commands::cockpit::run(&commands::cockpit::CockpitCommand, &cfg)?;
            } else {
                banner::print(args.quiet);
                println!("cashell v{} — CA ecosystem CLI", env!("CARGO_PKG_VERSION"));
                println!("Run 'cashell --help' for usage or run in an interactive terminal for CAS Cockpit.");
            }
            Ok(0)
        }
        Some(Commands::Cockpit(cmd)) => {
            // L-08: periodic license verification when cockpit opens.
            commands::daemon::periodic_license_verify(&cfg);
            commands::cockpit::run(&cmd, &cfg)?;
            Ok(0)
        }
        Some(Commands::Auth(cmd)) => {
            commands::auth::run(&cmd)?;
            Ok(0)
        }
        Some(Commands::Product(cmd)) => {
            commands::product::run(&cmd)?;
            Ok(0)
        }
        Some(Commands::Project(cmd)) => {
            let code = commands::project::run(&cmd, &cfg)?;
            Ok(code)
        }
        Some(Commands::Version(cmd)) => {
            commands::version::run(&cmd)?;
            Ok(0)
        }
        Some(Commands::Config(cmd)) => {
            commands::config::run(&cmd)?;
            Ok(0)
        }
        Some(Commands::Git(cmd)) => {
            let code = commands::git::run(&cmd, &cfg)?;
            Ok(code)
        }
        Some(Commands::GitStatus(cmd)) => {
            let code = commands::git_status::run(&cmd, &cfg)?;
            Ok(code)
        }
        Some(Commands::Deploy(cmd)) => {
            let result = commands::deploy::run(&cmd, &cfg)?;
            if !result.success {
                eprintln!("✗ Deployment failed: {}", result.error.unwrap_or_default());
                Ok(2)
            } else {
                cashell::output::success(&format!(
                    "Deployment successful in {:.1}s",
                    result.duration_seconds
                ));
                Ok(0)
            }
        }
        Some(Commands::Doctor(cmd)) => {
            let code = commands::doctor::run(&cmd, &cfg)?;
            Ok(code)
        }
        Some(Commands::Ai(cmd)) => {
            cashell::ai::run(&cmd, &cfg)?;
            Ok(0)
        }
        Some(Commands::Audit(cmd)) => {
            let code = commands::audit::run(&cmd, &cfg, args.quiet)?;
            Ok(code)
        }
        Some(Commands::Fix(cmd)) => {
            let code = commands::fix::run(&cmd, &cfg)?;
            Ok(code)
        }
        Some(Commands::Review(cmd)) => {
            commands::review::run(&cmd, &cfg)?;
            Ok(0)
        }
        Some(Commands::R2(cmd)) => {
            let code = commands::r2::run(&cmd)?;
            Ok(code)
        }
        Some(Commands::Vault(cmd)) => {
            let code = commands::vault::run(&cmd)?;
            Ok(code)
        }
        Some(Commands::Server(cmd)) => {
            commands::server::run(&cmd, &cfg)?;
            Ok(0)
        }
        Some(Commands::Chat(cmd)) => {
            commands::chat::run(&cmd, &cfg)?;
            Ok(0)
        }
        Some(Commands::Daemon(cmd)) => {
            commands::daemon::run(&cmd, &cfg)?;
            Ok(0)
        }
        Some(Commands::License(cmd)) => {
            let code = commands::license::run(&cmd, &cfg)?;
            Ok(code)
        }
        Some(Commands::Pro(cmd)) => {
            let code = commands::pro::run(&cmd, &cfg)?;
            Ok(code)
        }
        Some(Commands::Uptime(cmd)) => {
            commands::uptime::run(&cmd)?;
            Ok(0)
        }
        Some(Commands::Analytics(cmd)) => {
            commands::analytics::run(&cmd)?;
            Ok(0)
        }
        Some(Commands::Reset(cmd)) => {
            commands::reset::run(&cmd)?;
            Ok(0)
        }
    }
}
