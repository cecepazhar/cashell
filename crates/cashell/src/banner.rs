//! Startup banner — CA dual-wing ASCII brandmark.
//!
//! Rendered in pure ASCII so it survives any terminal/locale.
//! Gated on stdout being a TTY so piped output stays clean.
//! Hidden when `--quiet` is set.

use colored::Colorize;
use std::io::{self, IsTerminal, Write};

const LOGO: &str = r"
       /\              /\
      /  \            /  \
     / /\ \          / /\ \
    / /  \ \   CA   / /  \ \
   / /    \ \      / /    \ \
  /_/      \_\    /_/      \_\
";

/// Print the brandmark + description + version. No-op when stdout is not a TTY
/// or when `quiet` is true (set via `--quiet` global flag).
pub fn print(quiet: bool) {
    if quiet || !io::stdout().is_terminal() {
        return;
    }
    let mut out = io::stdout().lock();
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", LOGO.bright_cyan().bold());
    let _ = writeln!(
        out,
        "{}",
        "  Cecep Azhar · CA Ecosystem Universal Shell".bright_white()
    );
    let _ = writeln!(
        out,
        "{}",
        format!(
            "  cashell v{} · Zero-knowledge, local-first sovereign CLI",
            env!("CARGO_PKG_VERSION")
        )
        .dimmed()
    );
    let _ = writeln!(out);
    let _ = out.flush();
}
