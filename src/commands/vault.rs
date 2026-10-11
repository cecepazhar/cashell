//! vault command — synchronize or query the markdown knowledge base.
//!
//! REQ-P4-02.
//! sync:  atomic pull/push on the vault path via git, guarded by lockfile.
//! query: fast standard-library markdown full-text search with TTY table
//!        and pipe JSON formats.

use crate::cli::{VaultCommand, VaultMode};
use crate::output;
use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DEFAULT_VAULT_PATH: &str = "/run/media/cecepazhar/DATA/11_NOTES/Notes";

/// Query search hit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QueryHit {
    pub file: String,
    pub line: usize,
    pub text: String,
}

/// Sync result representation for pipe JSON mode.
#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub vault: String,
    pub branch: String,
    pub head_before: String,
    pub head_after: String,
    pub commit_range: String,
    pub pulled: bool,
    pub pushed: bool,
}

/// A simple process-file lock using exclusive open.
struct FileLock {
    path: PathBuf,
}

impl FileLock {
    fn acquire(path: PathBuf) -> Result<Self> {
        let _ = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "vault sync already running (lockfile {} present)",
                    path.display()
                )
            })?;
        Ok(Self { path })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn run(cmd: &VaultCommand) -> Result<i32> {
    let vault_path = PathBuf::from(DEFAULT_VAULT_PATH);

    match &cmd.mode {
        VaultMode::Sync => sync(&vault_path),
        VaultMode::Query { term } => query(&vault_path, term),
    }
}

fn run_git(vault: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .current_dir(vault)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("git {} failed: {}", args.join(" "), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn rev_parse(vault: &Path, rev: &str) -> Result<String> {
    run_git(vault, &["rev-parse", rev])
}

fn sync(vault: &Path) -> Result<i32> {
    if !vault.is_dir() {
        anyhow::bail!("vault directory does not exist: {}", vault.display());
    }
    let git_dir = vault.join(".git");
    if !git_dir.exists() {
        anyhow::bail!("vault is not a git repository: {}", vault.display());
    }

    let lock = FileLock::acquire(vault.join(".cashell-sync.lock"))?;

    let branch = run_git(vault, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == "HEAD" {
        anyhow::bail!("vault is in detached HEAD state; sync aborted");
    }

    let head_before = rev_parse(vault, "HEAD")?;

    // Atomic pull with no-rebase
    run_git(vault, &["pull", "--no-rebase", "--ff-only"]).or_else(|_| {
        // Fallback: regular no-rebase pull if remote had clean merge
        run_git(vault, &["pull", "--no-rebase"])
    })?;

    // Push current branch
    run_git(vault, &["push", "origin", &branch])?;

    let head_after = rev_parse(vault, "HEAD")?;
    let short_before = head_before.get(..7).unwrap_or(&head_before);
    let short_after = head_after.get(..7).unwrap_or(&head_after);
    let range = format!("{short_before}..{short_after}");

    let result = SyncResult {
        vault: vault.display().to_string(),
        branch,
        head_before,
        head_after,
        commit_range: range.clone(),
        pulled: true,
        pushed: true,
    };

    if output::is_tty() {
        output::info(&format!("Vault synced: {}", vault.display()));
        output::kv("branch", &result.branch);
        output::kv("commit_range", &range);
        output::success("sync complete");
    } else {
        println!("{}", serde_json::to_string(&result)?);
    }

    drop(lock);
    Ok(0)
}

/// Recursively collect markdown files in `dir`.
pub fn collect_md_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_md_inner(dir, &mut files);
    files
}

fn collect_md_inner(dir: &Path, acc: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.starts_with('.') && name != "node_modules" && name != "target" {
                collect_md_inner(&p, acc);
            }
        } else if p.is_file() {
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if ext.eq_ignore_ascii_case("md") {
                    acc.push(p);
                }
            }
        }
    }
}

/// Scan a single markdown file for lines containing `term` (case-insensitive).
pub fn scan_file(path: &Path, base: &Path, term_lower: &str) -> Vec<QueryHit> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let rel = path
        .strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let reader = BufReader::new(file);
    let mut hits = Vec::new();
    for (idx, line_res) in reader.lines().enumerate() {
        let line = match line_res {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.to_ascii_lowercase().contains(term_lower) {
            hits.push(QueryHit {
                file: rel.clone(),
                line: idx + 1,
                text: line.trim().to_string(),
            });
        }
    }
    hits
}

fn query(vault: &Path, term: &str) -> Result<i32> {
    if !vault.is_dir() {
        anyhow::bail!("vault directory does not exist: {}", vault.display());
    }
    if term.trim().is_empty() {
        anyhow::bail!("query term cannot be empty");
    }

    let files = collect_md_files(vault);
    let term_lower = term.to_ascii_lowercase();

    // Rayon parallel scan across files
    let hits: Vec<QueryHit> = files
        .par_iter()
        .flat_map(|p| scan_file(p, vault, &term_lower))
        .collect();

    if output::is_tty() {
        if hits.is_empty() {
            println!("No matches found for '{term}'");
            return Ok(0);
        }
        println!("\n  {:<60} {:<6} TEXT", "FILE", "LINE");
        println!("  {}", "─".repeat(110));
        for h in &hits {
            let f = if h.file.len() > 60 {
                &h.file[..60]
            } else {
                &h.file
            };
            let text_preview = if h.text.len() > 70 {
                &h.text[..70]
            } else {
                &h.text
            };
            println!("  {:<60} {:<6} {}", f, h.line, text_preview);
        }
        output::info(&format!("{} hit(s) for '{term}'", hits.len()));
    } else {
        println!("{}", serde_json::to_string(&hits)?);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_finds_matching_line_in_temp_vault() {
        let dir = std::env::temp_dir().join("cashell_test_vault");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("note.md");
        std::fs::write(&doc, "# Welcome\nLine 2 contains keyword\nLine 3").unwrap();

        let hits = scan_file(&doc, &dir, "keyword");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 2);
        assert_eq!(hits[0].text, "Line 2 contains keyword");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lockfile_prevents_concurrent_acquire() {
        let dir = std::env::temp_dir().join("cashell_lock_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lock_path = dir.join(".sync.lock");

        let l1 = FileLock::acquire(lock_path.clone());
        assert!(l1.is_ok());

        let l2 = FileLock::acquire(lock_path.clone());
        assert!(l2.is_err());

        drop(l1);
        let l3 = FileLock::acquire(lock_path);
        assert!(l3.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
