//! Startup banner — the CAShell `>>_` brandmark beside version info.
//!
//! Gated on stdout being a TTY so piped output stays clean.
//! Hidden when `--quiet` is set.

use colored::Colorize;
use std::io::{self, IsTerminal, Write};

/// Print the brandmark + description + version. No-op when stdout is not a TTY
/// or when `quiet` is true (set via `--quiet` global flag).
pub fn print(quiet: bool) {
    if quiet || !io::stdout().is_terminal() {
        return;
    }
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let version = env!("CARGO_PKG_VERSION");
    let home = std::env::var("HOME").unwrap_or_default();
    let cwd = std::env::current_dir()
        .map(|p| {
            let s = p.display().to_string();
            match s.strip_prefix(&home) {
                Some(rest) if !home.is_empty() => format!("~{rest}"),
                _ => s,
            }
        })
        .unwrap_or_default();
    let info = [
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        format!("{} {}", "CAShell".bright_green().bold(), version.bright_white().bold()),
        "Zero-Knowledge, Local-First Sovereign CLI".bright_white().to_string(),
        "cashell --help · cockpit: run `cashell`".dimmed().to_string(),
        cwd.dimmed().to_string(),
    ];
    let mut out = io::stdout().lock();
    let _ = writeln!(out);
    if no_color {
        let _ = writeln!(out, "  >> CAShell {version} · Zero-Knowledge, Local-First Sovereign CLI");
        let _ = writeln!(out);
        let _ = out.flush();
        return;
    }
    for (i, line) in crate::brand::ansi_lines().iter().enumerate() {
        let side = info.get(i).map(String::as_str).unwrap_or("");
        let _ = writeln!(out, "  {line}   {side}");
    }
    let _ = writeln!(out);
    let _ = out.flush();
}
