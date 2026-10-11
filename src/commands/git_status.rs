//! git-status command — scan multiple repositories for git status.

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
    pub repo: String,
    pub path: String,
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub stash: usize,
    pub clean: bool,
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

/// Scan a directory for .git folders up to the given depth,
/// skipping subdirectories whose names match any ignore pattern.
fn scan_repos(base: &Path, depth: u32, ignore_patterns: &[String]) -> Vec<PathBuf> {
    let mut results = Vec::new();
    scan_dir(base, depth, 0, ignore_patterns, &mut results);
    results
}

fn scan_dir(
    dir: &Path,
    max_depth: u32,
    current_depth: u32,
    ignore_patterns: &[String],
    results: &mut Vec<PathBuf>,
) {
    // depth=0 means unlimited; otherwise cap at max_depth
    if max_depth > 0 && current_depth > max_depth {
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
    // Recurse into subdirectories, respecting ignore patterns
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if ignore_patterns.iter().any(|p| p == &name) {
                continue;
            }
            scan_dir(
                &path,
                max_depth,
                current_depth + 1,
                ignore_patterns,
                results,
            );
        }
    }
}

/// Get the status of a single repository.
fn get_repo_status(repo_path: &Path) -> Option<RepoStatus> {
    let repo = Repository::open(repo_path).ok()?;
    let repo_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            repo_path
                .to_string_lossy()
                .rsplit('/')
                .next()
                .unwrap_or("?")
                .to_string()
        });

    // Branch
    let branch = if let Ok(head) = repo.head() {
        head.shorthand().unwrap_or("HEAD").to_string()
    } else {
        "HEAD".to_string()
    };

    // Ahead/behind vs upstream
    let (ahead, behind) = if let Ok(head) = repo.head() {
        let local_oid = head.target();
        let upstream_oid = head
            .name()
            .and_then(|n| repo.branch_upstream_name(n).ok())
            .and_then(|bu| bu.as_str().map(String::from))
            .and_then(|name| repo.find_reference(&name).ok())
            .and_then(|r| r.target());
        match (local_oid, upstream_oid) {
            (Some(l), Some(u)) => repo.graph_ahead_behind(l, u).unwrap_or((0, 0)),
            _ => (0, 0),
        }
    } else {
        (0, 0)
    };

    // Status
    let mut opts = StatusOptions::new();
    opts.include_untracked(true);
    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(_) => return None,
    };

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

    let clean = staged == 0 && unstaged == 0 && untracked == 0 && ahead == 0;

    Some(RepoStatus {
        repo: repo_name,
        path: repo_path.to_string_lossy().to_string(),
        branch,
        ahead,
        behind,
        staged,
        unstaged,
        untracked,
        stash,
        clean,
    })
}

/// Display status in table format (TTY).
fn print_table(repos: &[RepoStatus], changed_only: bool) {
    let filtered: Vec<_> = if changed_only {
        repos.iter().filter(|r| !r.clean).collect()
    } else {
        repos.iter().collect()
    };

    if filtered.is_empty() {
        println!("  No repositories found.");
        return;
    }

    println!(
        "\n  {:<35} {:<20} {:<10} {:<8} {:<8} UNPUSHED",
        "REPO", "BRANCH", "STATUS", "AHEAD", "BEHIND"
    );
    println!("  {}", "─".repeat(100));

    for r in &filtered {
        let status_label = if !r.clean {
            if r.staged > 0 || r.unstaged > 0 || r.untracked > 0 {
                "dirty".red().to_string()
            } else if r.ahead > 0 {
                "unpushed".yellow().to_string()
            } else {
                "clean".green().to_string()
            }
        } else {
            "clean".green().to_string()
        };

        println!(
            "  {:<35} {:<20} {:<10} {:<8} {:<8} {}",
            truncate(&r.repo, 35),
            truncate(&r.branch, 20),
            status_label,
            r.ahead,
            r.behind,
            if r.ahead > 0 {
                "⚠".yellow().to_string()
            } else {
                "".to_string()
            }
        );
    }

    let clean = repos.iter().filter(|r| r.clean).count();
    let dirty = repos
        .iter()
        .filter(|r| r.staged > 0 || r.unstaged > 0 || r.untracked > 0)
        .count();
    let unpushed = repos.iter().filter(|r| r.ahead > 0).count();

    println!(
        "\n  Summary: {} total, {} clean, {} dirty, {} unpushed\n",
        repos.len(),
        clean,
        dirty,
        unpushed
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        s.char_indices()
            .take_while(|&(i, c)| i < max && c as u32 <= 127)
            .map(|(_, c)| c)
            .collect::<String>()
    }
}

/// Display status in JSON format (strict array).
fn print_json(repos: &[RepoStatus], changed_only: bool) {
    let filtered: Vec<_> = if changed_only {
        repos.iter().filter(|r| !r.clean).collect()
    } else {
        repos.iter().collect()
    };
    let json =
        serde_json::to_string_pretty(filtered.as_slice()).unwrap_or_else(|_| "[]".to_string());
    println!("{json}");
}

pub fn run(cmd: &GitStatusCommand, cfg: &Config) -> Result<i32> {
    let scan_dirs = &cfg.workspace.scan_dirs;
    let depth = cmd.depth;
    let ignore = &cfg.workspace.ignore_patterns;

    let repo_paths: Vec<PathBuf> = scan_dirs
        .par_iter()
        .map(|d| expand_tilde(d))
        .flat_map(|p| scan_repos(&p, depth, ignore))
        .collect();

    // De-duplicate (a repo reachable via two scan_dirs should appear once)
    let mut repo_paths: Vec<PathBuf> = repo_paths
        .into_iter()
        .map(|p| p.canonicalize().unwrap_or(p))
        .collect();
    repo_paths.sort();
    repo_paths.dedup();

    let statuses: Vec<RepoStatus> = repo_paths
        .par_iter()
        .filter_map(|p| get_repo_status(p))
        .collect();

    let is_json = cmd.json || !output::is_tty();

    if is_json {
        print_json(&statuses, cmd.changed);
    } else {
        print_table(&statuses, cmd.changed);
    }

    // Exit code: 0 if all clean, 1 if any repo dirty or unpushed
    let any_dirty = statuses
        .iter()
        .any(|r| r.staged > 0 || r.unstaged > 0 || r.untracked > 0);
    let any_unpushed = statuses.iter().any(|r| r.ahead > 0);

    if any_dirty || any_unpushed {
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
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_repos_ignores_patterns() {
        use std::fs;
        let tmp = std::env::temp_dir().join("cashell_test_scan");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("good_repo")).unwrap();
        fs::create_dir_all(tmp.join("good_repo/.git")).unwrap();
        fs::create_dir_all(tmp.join("node_modules")).unwrap();
        fs::create_dir_all(tmp.join("node_modules/fake_repo/.git")).unwrap();
        fs::create_dir_all(tmp.join("target")).unwrap();
        fs::create_dir_all(tmp.join("target/fake_repo/.git")).unwrap();

        let ignore = vec!["node_modules".to_string(), "target".to_string()];
        let found = scan_repos(&tmp, 2, &ignore);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n == "good_repo"),
            "good_repo not found: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "fake_repo"),
            "ignored dirs not skipped: {names:?}"
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}
