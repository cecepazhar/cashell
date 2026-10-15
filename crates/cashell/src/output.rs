//! Output formatting: plain for pipe, styled for TTY.

use colored::Colorize;
use std::io::{self, IsTerminal};

/// Returns true if stdout is a TTY (interactive terminal).
pub fn is_tty() -> bool {
    io::stdout().is_terminal()
}

/// Colored info line in TTY, plain in pipe.
pub fn info(msg: &str) {
    if is_tty() {
        eprintln!("{} {}", "ℹ".green(), msg);
    } else {
        eprintln!("{msg}");
    }
}

/// Colored error line to stderr.
pub fn error(msg: &str) {
    if is_tty() {
        eprintln!("{} {}", "✗".red(), msg.red());
    } else {
        eprintln!("{msg}");
    }
}

/// Styled success message.
pub fn success(msg: &str) {
    if is_tty() {
        eprintln!("{} {}", "✓".green(), msg.green());
    } else {
        eprintln!("{msg}");
    }
}

/// Format a key=value pair.
pub fn kv(key: &str, value: &str) {
    if is_tty() {
        println!("  {}: {}", key.dimmed(), value);
    } else {
        println!("{key}={value}");
    }
}
