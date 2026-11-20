//! Rollback targets and automatic rollback after a failed deploy.
//!
//! A failed deploy can leave the app down (e.g. the new container crashed).
//! In that case the newest successful commit is redeployed through the same
//! executor and recorded as its own build, so `app builds` stays honest.
//! The rollback runs at most once: a failing rollback is not rolled back.

use std::collections::HashMap;
use std::path::Path;

use crate::config::AppConfig;
use crate::db::Database;
use crate::deploy::executor::{
    deploy_app, detect_compose, project_containers, project_name, ContainerHealth, DeployOutcome,
};
use crate::deploy::git::is_valid_commit_sha;

/// `(commit, status)` pairs, newest first, as read from `deploy_builds`.
pub type BuildHistory = [(String, String)];

/// Load the build history of one app (newest first).
pub fn history(db: &Database, app_name: &str) -> anyhow::Result<Vec<(String, String)>> {
    Ok(db
        .list_deploy_builds(Some(app_name), 50)?
        .into_iter()
        .map(|b| (b.commit_hash, b.status))
        .collect())
}

fn good_commits(history: &BuildHistory) -> impl Iterator<Item = &String> {
    history
        .iter()
        .filter(|(c, st)| st == "success" && is_valid_commit_sha(c))
        .map(|(c, _)| c)
}

/// Commit `app rollback` restores when no target is given: the newest
/// successful commit that differs from the last finished build. After a
/// failed deploy this is the release that was live, not the one before it.
pub fn default_rollback_target(history: &BuildHistory) -> Option<String> {
    let current = history
        .iter()
        .find(|(_, st)| st == "success" || st == "failed")
        .map(|(c, _)| c.as_str());
    good_commits(history)
        .find(|c| Some(c.as_str()) != current)
        .cloned()
}

/// Newest successful commit other than the one that just failed.
pub fn last_good_commit(history: &BuildHistory, failed: Option<&str>) -> Option<String> {
    good_commits(history)
        .find(|c| failed.map_or(true, |f| !c.starts_with(f) && !f.starts_with(c.as_str())))
        .cloned()
}

/// True when every container of the app's compose project is running and
/// not unhealthy right now (no waiting).
pub fn app_is_serving(app: &AppConfig) -> bool {
    let Some(tool) = detect_compose() else {
        return false;
    };
    match project_containers(&tool, &project_name(&app.name)) {
        Ok(c) => !c.is_empty() && c.iter().all(|(_, h)| *h == ContainerHealth::Healthy),
        Err(_) => false,
    }
}

/// What happened after a failed deploy.
#[derive(Debug)]
pub enum AutoRollback {
    /// The previous containers are still running and healthy.
    NotNeeded,
    /// The app is down but there is no earlier successful commit.
    NoGoodCommit,
    /// The last good commit was redeployed.
    Attempted {
        target: String,
        outcome: DeployOutcome,
    },
}

/// Decide and, when needed, run the rollback. Blocking (git + compose).
pub fn run_auto_rollback(
    app: &AppConfig,
    app_dir: &Path,
    env: &HashMap<String, String>,
    history: &BuildHistory,
    failed_commit: Option<&str>,
) -> AutoRollback {
    if app_is_serving(app) {
        return AutoRollback::NotNeeded;
    }
    match last_good_commit(history, failed_commit) {
        None => AutoRollback::NoGoodCommit,
        Some(target) => {
            let outcome = deploy_app(app, app_dir, Some(&target), env);
            AutoRollback::Attempted { target, outcome }
        }
    }
}

/// Record the rollback: a new build row for an attempted rollback, and a
/// note on the failed build. Returns a one-line summary for the user.
pub fn record_auto_rollback(
    db: &Database,
    app_name: &str,
    failed_build_id: i64,
    rollback: &AutoRollback,
) -> anyhow::Result<String> {
    let summary = match rollback {
        AutoRollback::NotNeeded => {
            "auto-rollback: not needed — previous containers are still running and healthy"
                .to_string()
        }
        AutoRollback::NoGoodCommit => {
            "auto-rollback: no earlier successful build — app stays down".to_string()
        }
        AutoRollback::Attempted { target, outcome } => {
            let id = db.create_deploy_build(app_name, target)?;
            let now = chrono::Utc::now().timestamp();
            db.update_deploy_build_status(id, "building", Some(now), None, None)?;
            let status = if outcome.result.is_ok() {
                "success"
            } else {
                "failed"
            };
            let log = format!(
                "auto-rollback after failed build #{failed_build_id}\n{}",
                outcome.log
            );
            let done = chrono::Utc::now().timestamp();
            db.update_deploy_build_status(id, status, None, Some(done), Some(&log))?;
            let short = target.get(..7).unwrap_or(target);
            match &outcome.result {
                Ok(_) => format!("auto-rollback: restored {short} (build #{id})"),
                Err(e) => format!("auto-rollback to {short} failed too (build #{id}): {e}"),
            }
        }
    };
    db.append_deploy_build_log(failed_build_id, &format!("{summary}\n"))?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter()
            .map(|(c, s)| (c.to_string(), s.to_string()))
            .collect()
    }

    #[test]
    fn rollback_after_failed_deploy_restores_last_good() {
        // v3 failed while v2 was live: restore v2, not v1.
        let hist = h(&[
            ("6dfdedc", "failed"),
            ("c59a5eb", "success"),
            ("793541d", "success"),
        ]);
        assert_eq!(default_rollback_target(&hist).as_deref(), Some("c59a5eb"));
    }

    #[test]
    fn rollback_from_healthy_goes_to_previous_distinct_commit() {
        let hist = h(&[
            ("c59a5eb", "success"),
            ("c59a5eb", "success"),
            ("793541d", "success"),
        ]);
        assert_eq!(default_rollback_target(&hist).as_deref(), Some("793541d"));
    }

    #[test]
    fn rollback_ignores_in_flight_and_invalid_builds() {
        let hist = h(&[
            ("HEAD", "building"),
            ("c59a5eb", "success"),
            ("HEAD", "success"),
            ("793541d", "success"),
        ]);
        assert_eq!(default_rollback_target(&hist).as_deref(), Some("793541d"));
        assert_eq!(default_rollback_target(&h(&[("c59a5eb", "success")])), None);
    }

    #[test]
    fn last_good_skips_the_failed_commit() {
        let full = "6dfdedc0000000000000000000000000000000aa";
        let hist = h(&[
            ("HEAD", "building"),
            (full, "success"),
            ("c59a5eb", "success"),
        ]);
        // Same commit given short or full is still excluded.
        assert_eq!(
            last_good_commit(&hist, Some("6dfdedc")).as_deref(),
            Some("c59a5eb")
        );
        assert_eq!(
            last_good_commit(&hist, Some(full)).as_deref(),
            Some("c59a5eb")
        );
        assert_eq!(last_good_commit(&hist, None).as_deref(), Some(full));
        assert_eq!(last_good_commit(&h(&[("6dfdedc", "failed")]), None), None);
    }

    #[test]
    fn record_attempted_rollback_adds_build_and_note() {
        let db = Database::open_in_memory().expect("in-memory db");
        let failed = db.create_deploy_build("web", "6dfdedc").expect("create");
        db.update_deploy_build_status(failed, "failed", None, None, Some("boom\n"))
            .expect("status");
        let rb = AutoRollback::Attempted {
            target: "c59a5eb".to_string(),
            outcome: DeployOutcome {
                result: Ok("Deployed".to_string()),
                commit: Some("c59a5eb".to_string()),
                log: "health ok\n".to_string(),
            },
        };
        let summary = record_auto_rollback(&db, "web", failed, &rb).expect("record");
        assert!(summary.contains("restored c59a5eb"));
        let builds = db.list_deploy_builds(Some("web"), 10).expect("list");
        let newest = builds.first().expect("rollback build");
        assert_eq!(
            (newest.commit_hash.as_str(), newest.status.as_str()),
            ("c59a5eb", "success")
        );
        assert!(newest
            .log_output
            .contains(&format!("after failed build #{failed}")));
        let orig = builds
            .iter()
            .find(|b| b.id == failed)
            .expect("failed build");
        assert_eq!(orig.status, "failed");
        assert!(orig.log_output.contains("auto-rollback: restored"));
    }
}
