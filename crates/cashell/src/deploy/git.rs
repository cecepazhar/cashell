//! Git checkout and clone engine via libgit2 (D-04).

use anyhow::{anyhow, Context, Result};
use git2::build::CheckoutBuilder;
use git2::{Oid, Repository};
use std::path::Path;

/// Result metadata from a git deploy sync operation (D-04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployGitResult {
    pub commit_hash: String,
    pub short_hash: String,
    pub branch: String,
    pub message: String,
}

/// Clones or fetches an application repository and checks out the specified branch or commit (D-04).
pub fn sync_app_repository(
    repo_url: &str,
    target_dir: &Path,
    branch: &str,
    target_commit: Option<&str>,
) -> Result<DeployGitResult> {
    let repo = if target_dir.join(".git").exists() {
        let repo = Repository::open(target_dir)
            .with_context(|| format!("Failed to open existing repo at {:?}", target_dir))?;
        if let Ok(mut remote) = repo.find_remote("origin") {
            let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
            let _ = remote.fetch(&[&refspec], None, None);
        }
        repo
    } else {
        if let Some(parent) = target_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Repository::clone(repo_url, target_dir)
            .with_context(|| format!("Failed to clone repo '{repo_url}' into {:?}", target_dir))?
    };

    if let Some(commit_sha) = target_commit {
        let oid = Oid::from_str(commit_sha)
            .map_err(|e| anyhow!("Invalid commit SHA '{commit_sha}': {e}"))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| anyhow!("Commit '{commit_sha}' not found: {e}"))?;
        let obj = commit.as_object();
        repo.checkout_tree(obj, Some(CheckoutBuilder::new().force()))?;
        repo.set_head_detached(oid)?;
    } else if let Ok(branch_ref) = repo.find_branch(branch, git2::BranchType::Local) {
        let refname = branch_ref.get().name().unwrap_or("HEAD");
        repo.set_head(refname)?;
        repo.checkout_head(Some(CheckoutBuilder::new().force()))?;
    } else if let Ok(remote_ref) = repo.find_reference(&format!("refs/remotes/origin/{branch}")) {
        let commit = remote_ref.peel_to_commit()?;
        let _ = repo.branch(branch, &commit, true);
        repo.set_head(&format!("refs/heads/{branch}"))?;
        repo.checkout_head(Some(CheckoutBuilder::new().force()))?;
    } else {
        repo.checkout_head(Some(CheckoutBuilder::new().force()))?;
    }

    let head = repo.head().context("Failed to get HEAD reference")?;
    let commit = head
        .peel_to_commit()
        .context("Failed to peel HEAD to commit")?;
    let hash = commit.id().to_string();
    let short_hash = if hash.len() >= 7 {
        hash[..7].to_string()
    } else {
        hash.clone()
    };
    let message = commit.summary().unwrap_or("").to_string();

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
    use git2::Signature;
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

        // Cleanup
        let _ = fs::remove_dir_all(&tmp);
    }
}
