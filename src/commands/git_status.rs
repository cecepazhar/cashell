//! git status command — scan multiple repositories for status.

use crate::cli::GitStatusCommand;
use crate::config::Config;
use crate::output;
use anyhow::Result;
use colored::Colorize;
use git2::{Repository, StatusOptions};
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A single repository's git status.
#[derive(Debug, Clone, Serialize)]
pub struct RepoStatus {
    pub path: String,
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub stash: usize,
    pub is_dirty: bool,
    pub is_unpushed: bool,
}

/// Summary of all scanned repositories.
#[derive(Debug, Clone, Serialize)]
pub struct GitStatusSummary {
    pub total: usize,
    pub clean: usize,
    pub dirty: usize,
    pub unpushed: usize,
    pub repos: Vec<RepoStatus>,
}

/// Expand tilde in paths.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}

/// Scan a directory for .git folders up to the given depth.
fn scan_repos(base: &Path, depth: u32) -> Vec<PathBuf> {
    let mut results = Vec::new();
    scan_dir(base, depth, 0, &mut results);
    results
}

fn scan_dir(dir: &Path, max_depth: u32, current_depth: u32, results: &mut Vec<PathBuf>) {
    if max_depth != 0 && current_depth > max_depth {
        return;
    }
    if !dir.is_dir() {
        return;
    }
    // Check for .git folder
    let git_path = dir.join(".git");
    if git_path.exists() {
        results.push(dir.to_path_buf());
        return;
    }
    // Recurse into subdirectories
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_dir(&path, max_depth, current_depth + 1, results);
            }
        }
    }
}

/// Get the status of a single repository.
fn get_repo_status(repo_path: &Path) -> Option<RepoStatus> {
    let repo = Repository::open(repo_path).ok()?;

    // Branch & remote tracking ahead/behind
    let (branch, ahead, behind) = if let Ok(head) = repo.head() {
        let b = head.shorthand().unwrap_or("HEAD").to_string();
        let (a, bh) = if let Ok(local_oid) = head
            .resolve()
            .and_then(|r| r.target().ok_or(git2::Error::from_str("no target")))
        {
            if let Ok(upstream) = repo.branch_upstream_name(head.name().unwrap_or("")) {
                if let Ok(upstream_buf) = upstream
                    .as_str()
                    .ok_or(git2::Error::from_str("invalid str"))
                {
                    if let Ok(upstream_ref) = repo.find_reference(upstream_buf) {
                        if let Some(upstream_oid) = upstream_ref.target() {
                            repo.graph_ahead_behind(local_oid, upstream_oid)
                                .unwrap_or((0, 0))
                        } else {
                            (0, 0)
                        }
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };
        (b, a, bh)
    } else {
        ("HEAD".to_string(), 0, 0)
    };

    // Status
    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts)).ok()?;

    let mut staged = 0;
    let mut unstaged = 0;
    let mut untracked = 0;

    for entry in statuses.iter() {
        let st = entry.status();
        if st.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            staged += 1;
        }
        if st.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        ) {
            unstaged += 1;
        }
        if st.contains(git2::Status::WT_NEW) {
            untracked += 1;
        }
    }

    // Stash count
    let stash = repo.reflog("refs/stash").map(|r| r.len()).unwrap_or(0);

    let is_dirty = staged > 0 || unstaged > 0 || untracked > 0;
    let is_unpushed = ahead > 0;

    Some(RepoStatus {
        path: repo_path.to_string_lossy().to_string(),
        branch,
        ahead,
        behind,
        staged,
        unstaged,
        untracked,
        stash,
        is_dirty,
        is_unpushed,
    })
}

/// Display status in table format (TTY).
fn print_table(repos: &[RepoStatus], changed_only: bool) {
    let filtered: Vec<_> = if changed_only {
        repos
            .iter()
            .filter(|r| r.is_dirty || r.is_unpushed)
            .collect()
    } else {
        repos.iter().collect()
    };

    if filtered.is_empty() {
        println!("  No repositories found.");
        return;
    }

    println!(
        "\n  {:<40} {:<15} {:<8} {:<8} {:<8} {:<8} UNPUSHED",
        "PATH", "BRANCH", "AHEAD", "BEHIND", "STAGED", "DIRTY"
    );
    println!("  {}", "─".repeat(110));

    for r in &filtered {
        let status_color = if r.is_dirty {
            "dirty".red().to_string()
        } else if r.is_unpushed {
            "unpushed".yellow().to_string()
        } else {
            "clean".green().to_string()
        };

        let path_display = if r.path.len() > 40 {
            &r.path[..40]
        } else {
            &r.path
        };

        println!(
            "  {:<40} {:<15} {:<8} {:<8} {:<8} {:<8} {}",
            path_display,
            r.branch,
            r.ahead,
            r.behind,
            r.staged + r.unstaged,
            r.untracked,
            status_color,
        );
    }

    let clean = repos
        .iter()
        .filter(|r| !r.is_dirty && !r.is_unpushed)
        .count();
    let dirty = repos.iter().filter(|r| r.is_dirty).count();
    let unpushed = repos.iter().filter(|r| r.is_unpushed).count();

    println!(
        "\n  Summary: {} total, {} clean, {} dirty, {} unpushed\n",
        repos.len(),
        clean,
        dirty,
        unpushed
    );
}

/// Display status in JSON format.
fn print_json(summary: &GitStatusSummary) {
    println!("{}", serde_json::to_string_pretty(summary).unwrap());
}

pub fn run(cmd: &GitStatusCommand, cfg: &Config) -> Result<i32> {
    let scan_dirs = &cfg.workspace.scan_dirs;
    let depth = cmd.depth;

    let repo_paths: Vec<PathBuf> = scan_dirs
        .par_iter()
        .map(|d| expand_tilde(d))
        .flat_map(|p| scan_repos(&p, depth))
        .collect();

    let statuses: Vec<RepoStatus> = repo_paths
        .par_iter()
        .filter_map(|p| get_repo_status(p))
        .collect();

    let clean = statuses
        .iter()
        .filter(|r| !r.is_dirty && !r.is_unpushed)
        .count();
    let dirty = statuses.iter().filter(|r| r.is_dirty).count();
    let unpushed = statuses.iter().filter(|r| r.is_unpushed).count();

    let summary = GitStatusSummary {
        total: statuses.len(),
        clean,
        dirty,
        unpushed,
        repos: statuses.clone(),
    };

    let is_json = cmd.json || !output::is_tty();

    if is_json {
        print_json(&summary);
    } else {
        print_table(&statuses, cmd.changed);
    }

    // Exit code: 0 if all clean, 1 if any dirty/unpushed
    if dirty > 0 || unpushed > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde("~/test"), home.join("test"));
        assert_eq!(
            expand_tilde("/absolute/path"),
            PathBuf::from("/absolute/path")
        );
    }

    #[test]
    fn test_repo_status_empty() {
        let tmp = std::env::temp_dir().join("cashell_test_empty");
        std::fs::create_dir_all(&tmp).unwrap();
        let status = get_repo_status(&tmp);
        assert!(status.is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }
}
