//! Sequential deployment build queue and database build logger (D-05).

use crate::db::{Database, DeployBuildRecord};
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
#[cfg(test)]
use tokio::time::{sleep, Duration};

/// A job payload placed in the deployment build queue (D-05).
#[derive(Debug, Clone)]
pub struct BuildJob {
    pub build_id: i64,
    pub app_name: String,
    pub commit_hash: String,
    pub branch: String,
}

/// Execution hook for performing the actual build & deploy step.
pub type BuildExecutor = Arc<
    dyn Fn(BuildJob, Arc<Mutex<Database>>) -> tokio::task::JoinHandle<Result<String>> + Send + Sync,
>;

/// What to check out for a queued job: the exact commit when the webhook
/// carried a valid SHA, otherwise `None` = the tip of the configured branch
/// (payloads without a commit are queued as the placeholder `"HEAD"`).
pub fn commit_to_deploy(commit_hash: &str) -> Option<String> {
    crate::deploy::git::is_valid_commit_sha(commit_hash).then(|| commit_hash.to_string())
}

/// The real executor for configured `[[apps]]`: git sync → compose build →
/// health gate, with the full command output stored in the build log.
pub fn app_build_executor(apps: Vec<crate::config::AppConfig>) -> BuildExecutor {
    let apps = Arc::new(apps);
    Arc::new(move |job: BuildJob, db: Arc<Mutex<Database>>| {
        let apps = apps.clone();
        tokio::spawn(async move {
            let app = apps
                .iter()
                .find(|a| a.name == job.app_name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("app '{}' is not configured", job.app_name))?;
            // Webhooks without a commit id deploy the branch tip.
            let commit = commit_to_deploy(&job.commit_hash);
            let dir = crate::paths::app_dir(&app.name);
            let store = crate::secrets::SecretStore::load_default().ok();
            let env = app.resolve_env(store.as_ref());
            let outcome = {
                let (app, dir, env) = (app.clone(), dir.clone(), env.clone());
                tokio::task::spawn_blocking(move || {
                    crate::deploy::deploy_app(&app, &dir, commit.as_deref(), &env)
                })
                .await
                .map_err(|e| anyhow::anyhow!("deploy task failed: {e}"))?
            };
            let history = {
                let db = db.lock().await;
                let _ = db.append_deploy_build_log(job.build_id, &outcome.log);
                if let Some(c) = &outcome.commit {
                    let _ = db.update_deploy_build_commit(job.build_id, c);
                }
                if outcome.result.is_ok() {
                    None
                } else {
                    crate::deploy::rollback::history(&db, &app.name).ok()
                }
            };
            let (Err(err), Some(history)) = (&outcome.result, history) else {
                return outcome.result.map_err(|e| anyhow::anyhow!(e));
            };
            // The deploy failed: bring the last good release back if the app is down.
            let failed = outcome.commit.clone();
            let rb = {
                let app = app.clone();
                tokio::task::spawn_blocking(move || {
                    crate::deploy::rollback::run_auto_rollback(
                        &app,
                        &dir,
                        &env,
                        &history,
                        failed.as_deref(),
                    )
                })
                .await
                .map_err(|e| anyhow::anyhow!("rollback task failed: {e}"))?
            };
            let summary = {
                let db = db.lock().await;
                crate::deploy::rollback::record_auto_rollback(&db, &app.name, job.build_id, &rb)
                    .unwrap_or_else(|e| format!("auto-rollback: could not record result: {e:#}"))
            };
            Err(anyhow::anyhow!("{err}; {summary}"))
        })
    })
}

/// Sequential build queue processing builds one by one (D-05).
pub struct BuildQueue {
    db: Arc<Mutex<Database>>,
    tx: mpsc::Sender<BuildJob>,
    active_job_count: Arc<Mutex<usize>>,
    pub is_running: Arc<AtomicBool>,
}

impl std::fmt::Debug for BuildQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildQueue")
            .field("is_running", &self.is_running.load(Ordering::Relaxed))
            .finish()
    }
}

impl BuildQueue {
    /// Create a new build queue backed by SQLite database.
    pub fn new(
        db: Database,
        custom_executor: Option<BuildExecutor>,
    ) -> (Arc<Self>, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<BuildJob>(100);
        let db_arc = Arc::new(Mutex::new(db));
        let is_running = Arc::new(AtomicBool::new(true));
        let is_running_worker = is_running.clone();
        let active_count = Arc::new(Mutex::new(0));
        let active_count_worker = active_count.clone();

        let queue = Arc::new(Self {
            db: db_arc.clone(),
            tx,
            active_job_count: active_count,
            is_running,
        });

        let worker_handle = tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                if !is_running_worker.load(Ordering::Relaxed) {
                    break;
                }

                {
                    let mut count = active_count_worker.lock().await;
                    *count += 1;
                }

                let now = chrono::Utc::now().timestamp();
                // 1. Mark status = 'building', started_at = now
                {
                    let db = db_arc.lock().await;
                    let _ = db.update_deploy_build_status(
                        job.build_id,
                        "building",
                        Some(now),
                        None,
                        None,
                    );
                    let _ = db.append_deploy_build_log(
                        job.build_id,
                        &format!(
                            "[{}] Starting sequential build for '{}' (commit: {}, branch: {})...\n",
                            chrono::Utc::now().to_rfc3339(),
                            job.app_name,
                            job.commit_hash,
                            job.branch
                        ),
                    );
                }

                // 2. Execute build step
                let build_result = if let Some(ref exec) = custom_executor {
                    let handle = exec(job.clone(), db_arc.clone());
                    handle.await.unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")))
                } else {
                    // Never report a build that did not run.
                    Err(anyhow::anyhow!(
                        "no build executor configured — nothing was built or deployed"
                    ))
                };

                let finished_now = chrono::Utc::now().timestamp();
                // 3. Mark finished status and write logs to DB
                {
                    let db = db_arc.lock().await;
                    match build_result {
                        Ok(log_msg) => {
                            let _ = db.append_deploy_build_log(
                                job.build_id,
                                &format!(
                                    "[{}] Build succeeded: {}\n",
                                    chrono::Utc::now().to_rfc3339(),
                                    log_msg
                                ),
                            );
                            let _ = db.update_deploy_build_status(
                                job.build_id,
                                "success",
                                None,
                                Some(finished_now),
                                None,
                            );
                        }
                        Err(err) => {
                            let _ = db.append_deploy_build_log(
                                job.build_id,
                                &format!(
                                    "[{}] Build failed: {}\n",
                                    chrono::Utc::now().to_rfc3339(),
                                    err
                                ),
                            );
                            let _ = db.update_deploy_build_status(
                                job.build_id,
                                "failed",
                                None,
                                Some(finished_now),
                                None,
                            );
                        }
                    }
                }

                {
                    let mut count = active_count_worker.lock().await;
                    *count -= 1;
                }
            }
        });

        (queue, worker_handle)
    }

    /// Enqueue a deployment job and persist it to SQLite as 'queued' (D-05).
    pub async fn enqueue(&self, app_name: &str, commit_hash: &str, branch: &str) -> Result<i64> {
        let build_id = {
            let db = self.db.lock().await;
            db.create_deploy_build(app_name, commit_hash)?
        };

        self.tx
            .send(BuildJob {
                build_id,
                app_name: app_name.to_string(),
                commit_hash: commit_hash.to_string(),
                branch: branch.to_string(),
            })
            .await
            .map_err(|e| anyhow::anyhow!("Failed to enqueue build: {e}"))?;

        Ok(build_id)
    }

    /// Retrieve build status and logs from SQLite.
    /// Return the count of currently active build jobs.
    pub async fn active_job_count(&self) -> usize {
        *self.active_job_count.lock().await
    }

    pub async fn get_build(&self, build_id: i64) -> Result<Option<DeployBuildRecord>> {
        let db = self.db.lock().await;
        db.get_deploy_build(build_id)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn test_d05_sequential_build_queue_and_db_logs() {
        let db = Database::open_in_memory().unwrap();
        static CONCURRENT_RUNNING: AtomicUsize = AtomicUsize::new(0);

        // Custom executor that tracks concurrency to prove sequential execution
        let executor: BuildExecutor = Arc::new(|job, db_arc| {
            tokio::spawn(async move {
                let current = CONCURRENT_RUNNING.fetch_add(1, Ordering::SeqCst) + 1;
                assert_eq!(
                    current, 1,
                    "Build jobs must execute strictly one at a time!"
                );

                // Simulate workload
                sleep(Duration::from_millis(80)).await;

                {
                    let db = db_arc.lock().await;
                    let _ = db.append_deploy_build_log(
                        job.build_id,
                        &format!("Compiling {}...\nDone.\n", job.app_name),
                    );
                }

                CONCURRENT_RUNNING.fetch_sub(1, Ordering::SeqCst);
                Ok(format!("Successfully built {}", job.app_name))
            })
        });

        let (queue, worker) = BuildQueue::new(db, Some(executor));

        // Enqueue two webhook builds simultaneously
        let id1 = queue
            .enqueue("api-service", "commit-aaa-111", "main")
            .await
            .unwrap();
        let id2 = queue
            .enqueue("web-service", "commit-bbb-222", "main")
            .await
            .unwrap();

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);

        // Wait for both builds to finish sequentially
        for _ in 0..50 {
            let b1 = queue.get_build(id1).await.unwrap().unwrap();
            let b2 = queue.get_build(id2).await.unwrap().unwrap();
            if b1.status == "success" && b2.status == "success" {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }

        let build1 = queue.get_build(id1).await.unwrap().unwrap();
        let build2 = queue.get_build(id2).await.unwrap().unwrap();

        assert_eq!(build1.status, "success");
        assert_eq!(build2.status, "success");

        // Verify sequential timing: Job 2 started at or after Job 1 started
        assert!(build2.started_at.unwrap() >= build1.started_at.unwrap());

        // Verify logs saved in SQLite DB
        assert!(build1.log_output.contains("Compiling api-service"));
        assert!(build2.log_output.contains("Compiling web-service"));

        queue.is_running.store(false, Ordering::Relaxed);
        worker.abort();
    }

    async fn wait_finished(queue: &BuildQueue, id: i64) -> DeployBuildRecord {
        for _ in 0..150 {
            let b = queue.get_build(id).await.unwrap().unwrap();
            if b.status == "success" || b.status == "failed" {
                return b;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("build {id} did not finish");
    }

    // D-EXE-4: a queue without an executor must never report a build it did not run.
    #[tokio::test]
    async fn test_d_exe4_queue_without_executor_never_reports_success() {
        let db = Database::open_in_memory().unwrap();
        let (queue, worker) = BuildQueue::new(db, None);
        let id = queue.enqueue("api", "abc1234", "main").await.unwrap();

        let build = wait_finished(&queue, id).await;
        assert_eq!(build.status, "failed");
        assert!(
            build.log_output.contains("no build executor configured"),
            "log: {}",
            build.log_output
        );
        assert!(!build.log_output.contains("Build succeeded"));

        queue.is_running.store(false, Ordering::Relaxed);
        worker.abort();
    }

    #[tokio::test]
    async fn test_d_exe4_executor_error_marks_build_failed() {
        let db = Database::open_in_memory().unwrap();
        let executor: BuildExecutor =
            Arc::new(|_job, _db| tokio::spawn(async { Err(anyhow::anyhow!("boom")) }));
        let (queue, worker) = BuildQueue::new(db, Some(executor));
        let id = queue.enqueue("api", "abc1234", "main").await.unwrap();

        let build = wait_finished(&queue, id).await;
        assert_eq!(build.status, "failed");
        assert!(build.log_output.contains("boom"));

        queue.is_running.store(false, Ordering::Relaxed);
        worker.abort();
    }

    // The real executor refuses unknown apps before touching git or containers.
    #[tokio::test]
    async fn test_d_exe4_real_executor_rejects_unconfigured_app() {
        let db = Database::open_in_memory().unwrap();
        let (queue, worker) = BuildQueue::new(db, Some(app_build_executor(vec![])));
        let id = queue.enqueue("ghost", "abc1234", "main").await.unwrap();

        let build = wait_finished(&queue, id).await;
        assert_eq!(build.status, "failed");
        assert!(build.log_output.contains("app 'ghost' is not configured"));

        queue.is_running.store(false, Ordering::Relaxed);
        worker.abort();
    }

    // D-WH-2: no usable commit in the payload -> deploy the branch tip, never the literal "HEAD".
    #[test]
    fn test_d_wh2_commit_to_deploy_maps_placeholder_to_branch_tip() {
        assert_eq!(commit_to_deploy("HEAD"), None);
        assert_eq!(commit_to_deploy(""), None);
        assert_eq!(commit_to_deploy("--upload-pack=evil"), None);
        assert_eq!(commit_to_deploy("main; rm -rf /"), None);
        assert_eq!(commit_to_deploy("674ac1f").as_deref(), Some("674ac1f"));
        let full = "674ac1f20b5722962e2a28a43a87bf2c29008a2d";
        assert_eq!(commit_to_deploy(full).as_deref(), Some(full));
    }
}
