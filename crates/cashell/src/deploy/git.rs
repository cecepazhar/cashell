//! Git checkout and clone engine (D-04).
//!
//! Uses the `git` CLI rather than libgit2 so private repositories work with
//! whatever credentials the server already has (SSH agent, deploy keys,
//! credential helpers). The previous libgit2 version never moved the local
//! branch to the fetched `origin/<branch>`, so redeploys silently kept the
//! first cloned commit, and fetch errors were discarded.

use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;
use std::process::Command;

/// Result metadata from a git deploy sync operation (D-04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployGitResult {
    pub commit_hash: String,
    pub short_hash: String,
    pub branch: String,
    pub message: String,
}

/// A commit id from a webhook or CLI: 7–40 lowercase/uppercase hex chars.
pub fn is_valid_commit_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Branch names we accept (no leading `-`, no `..`, conservative charset).
pub fn is_valid_branch(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    // Never block on an interactive credential prompt inside the daemon.
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    let out = cmd
        .output()
        .with_context(|| "failed to run `git` — is git installed?".to_string())?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Clones or fetches an application repository and checks out the requested
/// commit, or the tip of `origin/<branch>` when no commit is given (D-04).
pub fn sync_app_repository(
    repo_url: &str,
    target_dir: &Path,
    branch: &str,
    target_commit: Option<&str>,
) -> Result<DeployGitResult> {
    if repo_url.trim().is_empty() || repo_url.starts_with('-') {
        bail!("invalid repository URL '{repo_url}'");
    }
    if !is_valid_branch(branch) {
        bail!("invalid branch name '{branch}'");
    }
    if let Some(c) = target_commit {
        if !is_valid_commit_sha(c) {
            bail!("invalid commit SHA '{c}'");
        }
    }

    if target_dir.join(".git").exists() {
        git(Some(target_dir), &["fetch", "--prune", "--tags", "origin"])
            .context("fetching latest commits from origin")?;
    } else {
        if let Some(parent) = target_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let dest = target_dir
            .to_str()
            .ok_or_else(|| anyhow!("non-UTF-8 target path {target_dir:?}"))?;
        git(None, &["clone", "--", repo_url, dest])
            .with_context(|| format!("cloning '{repo_url}'"))?;
    }

    let remote_ref = format!("origin/{branch}");
    if let Some(sha) = target_commit {
        git(Some(target_dir), &["checkout", "--force", "--detach", sha])
            .with_context(|| format!("commit '{sha}' not found after fetch"))?;
    } else if git(
        Some(target_dir),
        &["rev-parse", "--verify", "--quiet", &remote_ref],
    )
    .is_ok()
    {
        // Move (or create) the local branch to the fetched remote tip.
        git(
            Some(target_dir),
            &["checkout", "--force", "-B", branch, &remote_ref],
        )?;
    } else {
        // Remote has no such branch (e.g. default branch is named differently):
        // stay on the cloned HEAD rather than failing.
        git(Some(target_dir), &["checkout", "--force", "HEAD"])?;
    }
    // Drop untracked build leftovers so every deploy builds the exact commit.
    git(Some(target_dir), &["clean", "-fdq"])?;

    let hash = git(Some(target_dir), &["rev-parse", "HEAD"])?;
    let message = git(Some(target_dir), &["log", "-1", "--format=%s"]).unwrap_or_default();
    let short_hash = hash.get(..7).unwrap_or(&hash).to_string();

    Ok(DeployGitResult {
        commit_hash: hash,
        short_hash,
        branch: branch.to_string(),
        message,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use git2::{Repository, Signature};
    use std::fs;

    #[test]
    fn test_d04_git_clone_and_checkout_commit() {
        let tmp = std::env::temp_dir().join(format!(
            "cashell_test_git_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let src_dir = tmp.join("src");
        let dest_dir = tmp.join("dest");
        fs::create_dir_all(&src_dir).unwrap();

        // 1. Initialize local source repo
        let repo = Repository::init(&src_dir).unwrap();
        let sig = Signature::now("Cecep Azhar", "hi@cecepazhar.com").unwrap();
        let file_path = src_dir.join("README.md");
        fs::write(&file_path, "# Test Project\n").unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new("README.md")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();

        let first_commit_oid = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "Initial commit for deploy test",
                &tree,
                &[],
            )
            .unwrap();

        // 2. Add second commit
        fs::write(&file_path, "# Test Project\nUpdated content\n").unwrap();
        index.add_path(Path::new("README.md")).unwrap();
        let tree2_id = index.write_tree().unwrap();
        let tree2 = repo.find_tree(tree2_id).unwrap();
        let parent = repo.find_commit(first_commit_oid).unwrap();
        let second_commit_oid = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "Second release commit",
                &tree2,
                &[&parent],
            )
            .unwrap();

        // 3. Clone and checkout HEAD
        let src_url = src_dir.to_str().unwrap();
        let res = sync_app_repository(src_url, &dest_dir, "main", None).unwrap();
        assert_eq!(res.commit_hash, second_commit_oid.to_string());
        assert_eq!(res.short_hash, second_commit_oid.to_string()[..7]);
        assert_eq!(res.message, "Second release commit");

        // 4. Checkout specific older commit SHA
        let first_sha = first_commit_oid.to_string();
        let res_old = sync_app_repository(src_url, &dest_dir, "main", Some(&first_sha)).unwrap();
        assert_eq!(res_old.commit_hash, first_sha);
        assert_eq!(res_old.message, "Initial commit for deploy test");

        // 5. Regression: a new commit pushed after the first clone must be
        //    picked up on the next sync (libgit2 version kept the old commit).
        let branch = repo
            .head()
            .unwrap()
            .shorthand()
            .unwrap_or("main")
            .to_string();
        fs::write(&file_path, "# Test Project\nThird\n").unwrap();
        index.add_path(Path::new("README.md")).unwrap();
        let tree3 = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent2 = repo.find_commit(second_commit_oid).unwrap();
        let third_oid = repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "Third commit",
                &tree3,
                &[&parent2],
            )
            .unwrap();
        let res_new = sync_app_repository(src_url, &dest_dir, &branch, None).unwrap();
        assert_eq!(res_new.commit_hash, third_oid.to_string());

        // 6. Hostile inputs are rejected before git runs.
        assert!(sync_app_repository(src_url, &dest_dir, "--upload-pack=x", None).is_err());
        assert!(sync_app_repository(src_url, &dest_dir, &branch, Some("--evil")).is_err());

        // Cleanup
        let _ = fs::remove_dir_all(&tmp);
    }
}
