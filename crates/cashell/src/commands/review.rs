//! Review command — `cashell review` (first-class review).

use crate::cli::ReviewCommand;
use crate::config::Config;
use crate::output;
use anyhow::Result;
use std::process::Command;

pub fn run(cmd: &ReviewCommand, cfg: &Config) -> Result<()> {
    let mut git = Command::new("git");
    git.arg("diff");
    if cmd.staged {
        git.arg("--staged");
    } else {
        git.arg(&cmd.branch);
    }
    if let Some(p) = &cmd.path {
        git.arg("--").arg(p);
    }
    let out = git.output()?;
    let diff = String::from_utf8_lossy(&out.stdout);
    if diff.trim().is_empty() {
        output::info("no diff to review");
        return Ok(());
    }
    let sys = "Review this diff for security, zero-allocation efficiency, and \
               architectural compliance. Lead with critical findings.";
    let answer = crate::ai::query(cfg, Some(sys), &diff)?;
    if !output::is_tty() {
        println!("{answer}");
    }
    Ok(())
}
