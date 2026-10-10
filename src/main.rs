//! cashell — CLI entry point
//!
//! Subcommands:
//!   auth      Login & manage tokens
//!   product   Query/list CA products metadata
//!   version   Print version info
//!   config    View/edit config file

use anyhow::Result;
use clap::Parser;

use cashell::cli::{Cashell, Commands};
use cashell::commands;

fn main() -> Result<()> {
    let args = Cashell::parse();
    match args.command {
        Commands::Auth(cmd) => commands::auth::run(&cmd)?,
        Commands::Product(cmd) => commands::product::run(&cmd)?,
        Commands::Version(cmd) => commands::version::run(&cmd)?,
        Commands::Config(cmd) => commands::config::run(&cmd)?,
    }
    Ok(())
}
