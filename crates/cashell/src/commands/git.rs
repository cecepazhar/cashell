//! git command group — dispatches `cashell git <sub>`.

use crate::cli::{GitCommand, GitMode};
use crate::config::Config;
use anyhow::Result;

pub fn run(cmd: &GitCommand, cfg: &Config) -> Result<i32> {
    match &cmd.mode {
        GitMode::Status(status_cmd) => super::git_status::run(status_cmd, cfg),
    }
}
