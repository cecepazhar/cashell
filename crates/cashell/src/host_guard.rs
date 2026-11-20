//! Guard for commands that change the host: container start/stop/restart/
//! rm/run, compose up, `systemctl` enable/stop/restart, sending signals.
//!
//! Unit tests (`cfg(test)`) must never touch the developer's real
//! containers or services, so every such call goes through this module and
//! is refused there. Read-only probes (`ps`, `logs`, `is-active`) are not
//! guarded. Production builds run the command unchanged.

use std::io;
use std::process::{Command, ExitStatus, Output};

/// Message of the error returned when a host-changing call is refused.
pub const REFUSED: &str = "refused in unit tests: this command would change the host";

/// True when host-changing commands may run (always, outside unit tests).
pub const fn allowed() -> bool {
    !cfg!(test)
}

/// `anyhow` flavour for functions that return `anyhow::Result`.
pub fn check() -> anyhow::Result<()> {
    if allowed() {
        Ok(())
    } else {
        anyhow::bail!(REFUSED)
    }
}

fn refused() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, REFUSED)
}

/// `Command::output()` for a host-changing command.
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    if allowed() {
        cmd.output()
    } else {
        Err(refused())
    }
}

/// `Command::status()` for a host-changing command.
pub fn status(cmd: &mut Command) -> io::Result<ExitStatus> {
    if allowed() {
        cmd.status()
    } else {
        Err(refused())
    }
}

/// `tokio::process::Command::output()` for a host-changing command.
pub async fn output_async(cmd: &mut tokio::process::Command) -> io::Result<Output> {
    if allowed() {
        cmd.output().await
    } else {
        Err(refused())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_changes_are_refused_in_unit_tests() {
        assert!(!allowed());
        assert!(check().is_err());
        // `true` would succeed if it ran; the guard must stop it first.
        let err = output(&mut Command::new("true")).expect_err("must be refused");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(status(&mut Command::new("true")).is_err());
    }
}
