//! Startup banner — CA dual-wing ASCII brandmark.
//!
//! Mirrors the CA monoline wireframe logo: two nested chevrons
//! flanking a centre spine. Gated on stdout being a TTY so piped
//! output (JSON/CSV) stays pure data.

use colored::Colorize;
use std::io::{self, IsTerminal, Write};

const LOGO: &str = "\
   <>        <>
  <  >  CA  <  >
   <>        <>";

/// Print the brandmark + version line. No-op when stdout is not a TTY.
pub fn print() {
    if !io::stdout().is_terminal() {
        return;
    }
    let mut out = io::stdout().lock();
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", LOGO.bright_cyan());
    let _ = writeln!(
        out,
        "{}",
        format!(
            "  cashell v{} · Cecep Saeful Azhar Hidayat, ST",
            env!("CARGO_PKG_VERSION")
        )
        .dimmed()
    );
    let _ = writeln!(out);
    let _ = out.flush();
}
