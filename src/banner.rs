//! Startup banner — CA dual-wing ASCII brandmark.
//!
//! Mirrors the CA monoline wireframe logo: two nested chevrons
//! flanking a centre spine. Written to stderr so stdout stays
//! pure data (JSON/CSV). Skipped entirely when stderr is not a TTY.

use colored::Colorize;
use std::io::{self, IsTerminal};

const LOGO: &str = "\
   <>        <>
  <  >  CA  <  >
   <>        <>";

/// Print the brandmark + version line. No-op in pipes.
pub fn print() {
    if !io::stderr().is_terminal() {
        return;
    }
    eprintln!();
    eprintln!("{}", LOGO.bright_cyan());
    eprintln!(
        "{}",
        format!(
            "  cashell v{} · Cecep Saeful Azhar Hidayat, ST\n",
            env!("CARGO_PKG_VERSION")
        )
        .dimmed()
    );
}
