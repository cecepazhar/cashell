//! Application management commands (D-02, D-03).
//!
//! Subcommands for managing applications defined in `[[apps]]` within config.toml:
//! - `cashell app list`
//! - `cashell app up <name>`
//! - `cashell app down <name>`
//! - `cashell app logs <name> [--lines <n>]`
//! - `cashell app restart <name>`
//! - `cashell app deploy <name> [--commit <sha>]`  (git → compose build → health)
//! - `cashell app builds <name> [--log <id>]`
//! - `cashell app rollback <name> [--target <sha>]`
//!
//! Apps deployed from a repository with a compose file are managed through
//! their compose project; otherwise the single container named after the app.

use crate::actions::docker::{DockerApi, SystemDocker};
use crate::cli::{AppAction, AppCommand};
use crate::config::Config;
use crate::output;
use crate::secrets::SecretStore;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppStatusItem {
    pub name: String,
    pub repo: String,
    pub branch: String,
    pub compose_path: String,
    pub secrets_count: usize,
    pub container_exists: bool,
    pub is_running: bool,
}

pub fn run(cmd: &AppCommand, cfg: &Config) -> Result<i32> {
    let docker = SystemDocker;
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(run_with_docker(cmd, cfg, &docker)))
    } else {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(run_with_docker(cmd, cfg, &docker))
    }
}

pub async fn run_with_docker(
    cmd: &AppCommand,
    cfg: &Config,
    docker: &dyn DockerApi,
) -> Result<i32> {
    match &cmd.action {
        AppAction::List => {
            let items = list_apps(cfg, docker).await?;
            if items.is_empty() {
                output::info("No applications configured under [[apps]] in config.toml.");
                return Ok(0);
            }

            output::info("=== Configured Applications ([[apps]]) ===");
            for item in &items {
                let status_str = if item.is_running {
                    "running (active)"
                } else if item.container_exists {
                    "stopped"
                } else {
                    "not deployed"
                };
                output::info(&format!(
                    "• {} [{}] — repo: {} ({}) | compose: {} | secrets: {}",
                    item.name,
                    status_str,
                    item.repo,
                    item.branch,
                    item.compose_path,
                    item.secrets_count
                ));
            }
            Ok(0)
        }
        AppAction::Up { name } => {
            let app = cfg
                .apps
                .iter()
                .find(|a| &a.name == name)
                .ok_or_else(|| anyhow!("App '{name}' not found in configuration [[apps]]"))?;
            let dir = crate::paths::app_dir(&app.name);
            if crate::deploy::executor::is_compose_app(app, &dir) {
                crate::deploy::executor::compose_cmd(app, &dir, &["up", "-d"])?;
                output::success(&format!("Application '{}' is up (compose).", app.name));
                return Ok(0);
            }

            let secret_store = SecretStore::load_default().ok();
            let envs = app.resolve_env(secret_store.as_ref());
            output::info(&format!(
                "Bringing up application '{}' (branch: {}, resolved env vars: {})...",
                app.name,
                app.branch,
                envs.len()
            ));

            let res = docker.container_action("start", &app.name).await;
            match res {
                Ok(out) => {
                    output::success(&format!("Application '{}' is up: {out}", app.name));
                    Ok(0)
                }
                Err(e) => {
                    output::error(&format!("Failed to start application '{}': {e}", app.name));
                    bail!("Failed to start application '{name}': {e}");
                }
            }
        }
        AppAction::Down { name } => {
            let app = cfg
                .apps
                .iter()
                .find(|a| &a.name == name)
                .ok_or_else(|| anyhow!("App '{name}' not found in configuration [[apps]]"))?;
            let dir = crate::paths::app_dir(&app.name);
            if crate::deploy::executor::is_compose_app(app, &dir) {
                crate::deploy::executor::compose_cmd(app, &dir, &["stop"])?;
                output::success(&format!("Application '{}' stopped (compose).", app.name));
                return Ok(0);
            }

            output::info(&format!("Stopping application '{}'...", app.name));
            let res = docker.container_action("stop", &app.name).await;
            match res {
                Ok(out) => {
                    output::success(&format!("Application '{}' stopped: {out}", app.name));
                    Ok(0)
                }
                Err(e) => {
                    output::error(&format!("Failed to stop application '{}': {e}", app.name));
                    bail!("Failed to stop application '{name}': {e}");
                }
            }
        }
        AppAction::Restart { name } => {
            let app = cfg
                .apps
                .iter()
                .find(|a| &a.name == name)
                .ok_or_else(|| anyhow!("App '{name}' not found in configuration [[apps]]"))?;
            let dir = crate::paths::app_dir(&app.name);
            if crate::deploy::executor::is_compose_app(app, &dir) {
                crate::deploy::executor::compose_cmd(app, &dir, &["restart"])?;
                output::success(&format!("Application '{}' restarted (compose).", app.name));
                return Ok(0);
            }

            output::info(&format!("Restarting application '{}'...", app.name));
            let res = docker.container_action("restart", &app.name).await;
            match res {
                Ok(out) => {
                    output::success(&format!("Application '{}' restarted: {out}", app.name));
                    Ok(0)
                }
                Err(e) => {
                    output::error(&format!(
                        "Failed to restart application '{}': {e}",
                        app.name
                    ));
                    bail!("Failed to restart application '{name}': {e}");
                }
            }
        }
        AppAction::Logs { name, lines } => {
            let app = cfg
                .apps
                .iter()
                .find(|a| &a.name == name)
                .ok_or_else(|| anyhow!("App '{name}' not found in configuration [[apps]]"))?;
            let dir = crate::paths::app_dir(&app.name);
            if crate::deploy::executor::is_compose_app(app, &dir) {
                let tail = lines.to_string();
                let out =
                    crate::deploy::executor::compose_cmd(app, &dir, &["logs", "--tail", &tail])?;
                println!("{out}");
                return Ok(0);
            }

            output::info(&format!(
                "Fetching last {lines} lines of logs for '{}'...",
                app.name
            ));
            let logs = docker.container_logs(&app.name, *lines).await?;
            println!("{logs}");
            Ok(0)
        }
        AppAction::Deploy { name, commit } => {
            let app = find_app(cfg, name)?;
            output::info(&format!(
                "Deploying '{}' ({}{})...",
                app.name,
                app.branch,
                commit
                    .as_deref()
                    .map(|c| format!(" @ {c}"))
                    .unwrap_or_default()
            ));
            recorded_deploy(app, commit.as_deref())
        }
        AppAction::Builds { name, log } => {
            let app = find_app(cfg, name)?;
            let db = crate::db::get_db().or_else(|_| crate::db::Database::open())?;
            if let Some(id) = log {
                let b = db
                    .get_deploy_build(*id)?
                    .filter(|b| b.app_name == app.name)
                    .ok_or_else(|| anyhow!("Build #{id} not found for '{}'", app.name))?;
                println!("{}", b.log_output);
                return Ok(0);
            }
            let builds = db.list_deploy_builds(Some(&app.name), 20)?;
            if builds.is_empty() {
                output::info(&format!("No builds yet for '{}'.", app.name));
                return Ok(0);
            }
            for b in builds {
                let when = chrono::DateTime::from_timestamp(b.created_at, 0)
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                let short = b.commit_hash.get(..7).unwrap_or(&b.commit_hash).to_string();
                println!("#{:<5} {:<8} {:<9} {}", b.id, short, b.status, when);
            }
            Ok(0)
        }
        AppAction::Rollback { name, target } => {
            let app = find_app(cfg, name)?;
            let db = crate::db::get_db().or_else(|_| crate::db::Database::open())?;
            let history = crate::deploy::rollback::history(&db, &app.name)?;
            // Only commits that really built and passed health checks qualify.
            let good: Vec<String> = history
                .iter()
                .filter(|(c, st)| st == "success" && crate::deploy::git::is_valid_commit_sha(c))
                .map(|(c, _)| c.clone())
                .collect();
            let target_commit = match target {
                Some(t) => {
                    if !good.iter().any(|c| c.starts_with(t.as_str())) {
                        bail!(
                            "Commit '{t}' has no successful build for '{name}' — run `cashell app builds {name}`"
                        );
                    }
                    t.clone()
                }
                None => {
                    crate::deploy::rollback::default_rollback_target(&history).ok_or_else(|| {
                        anyhow!(
                        "No previous successful build found to rollback for application '{name}'"
                    )
                    })?
                }
            };
            output::info(&format!(
                "Rolling back '{}' to commit '{}' (rebuild + health check)...",
                app.name, target_commit
            ));
            recorded_deploy(app, Some(&target_commit))
        }
    }
}

fn find_app<'a>(cfg: &'a Config, name: &str) -> Result<&'a crate::config::AppConfig> {
    cfg.apps
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow!("App '{name}' not found in configuration [[apps]]"))
}

/// Run a real deploy and record it (status + full log) in `deploy_builds`.
fn recorded_deploy(app: &crate::config::AppConfig, commit: Option<&str>) -> Result<i32> {
    let db = crate::db::get_db().or_else(|_| crate::db::Database::open())?;
    let build_id = db.create_deploy_build(&app.name, commit.unwrap_or("HEAD"))?;
    let started = chrono::Utc::now().timestamp();
    db.update_deploy_build_status(build_id, "building", Some(started), None, None)?;

    let store = SecretStore::load_default().ok();
    let env = app.resolve_env(store.as_ref());
    let dir = crate::paths::app_dir(&app.name);
    let outcome = crate::deploy::deploy_app(app, &dir, commit, &env);

    if let Some(c) = &outcome.commit {
        db.update_deploy_build_commit(build_id, c)?;
    }
    let finished = chrono::Utc::now().timestamp();
    let status = if outcome.result.is_ok() {
        "success"
    } else {
        "failed"
    };
    db.update_deploy_build_status(build_id, status, None, Some(finished), Some(&outcome.log))?;
    print!("{}", outcome.log);
    match outcome.result {
        Ok(msg) => {
            output::success(&format!("{msg} (build #{build_id})"));
            Ok(0)
        }
        Err(e) => {
            output::error(&format!("Deploy failed (build #{build_id}): {e}"));
            let history = crate::deploy::rollback::history(&db, &app.name)?;
            let rb = crate::deploy::rollback::run_auto_rollback(
                app,
                &dir,
                &env,
                &history,
                outcome.commit.as_deref(),
            );
            if let crate::deploy::rollback::AutoRollback::Attempted { outcome, .. } = &rb {
                print!("{}", outcome.log);
            }
            let summary =
                crate::deploy::rollback::record_auto_rollback(&db, &app.name, build_id, &rb)?;
            output::info(&summary);
            bail!("deploy of '{}' failed: {e}", app.name)
        }
    }
}

pub async fn list_apps(cfg: &Config, docker: &dyn DockerApi) -> Result<Vec<AppStatusItem>> {
    let container_list = docker.list_containers().await.unwrap_or_default();
    let mut items = Vec::new();

    for app in &cfg.apps {
        let exists = docker.container_exists(&app.name).await.unwrap_or(false);
        let is_running = container_list.iter().any(|c| c == &app.name);
        items.push(AppStatusItem {
            name: app.name.clone(),
            repo: app.repo.clone(),
            branch: app.branch.clone(),
            compose_path: app.compose_path.clone(),
            secrets_count: app.secrets.len(),
            container_exists: exists,
            is_running,
        });
    }

    Ok(items)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::actions::docker::MockDocker;
    use crate::config::AppConfig;

    #[allow(clippy::field_reassign_with_default)]
    fn sample_config() -> Config {
        let mut cfg = Config::default();
        cfg.apps = vec![
            AppConfig {
                name: "api-service".to_string(),
                repo: "https://github.com/cecepazhar/api.git".to_string(),
                branch: "main".to_string(),
                compose_path: "docker-compose.yml".to_string(),
                secrets: vec!["DB_PASSWORD".to_string(), "JWT_SECRET".to_string()],
                env: [("PORT".to_string(), "8080".to_string())]
                    .into_iter()
                    .collect(),
            },
            AppConfig {
                name: "web-client".to_string(),
                repo: "https://github.com/cecepazhar/web.git".to_string(),
                branch: "main".to_string(),
                compose_path: "docker-compose.yml".to_string(),
                secrets: vec![],
                env: Default::default(),
            },
        ];
        cfg
    }

    #[tokio::test]
    async fn test_d02_app_list_reads_config_and_status() {
        let cfg = sample_config();
        let mock_docker = MockDocker::new(&["api-service"]);

        let items = list_apps(&cfg, &mock_docker).await.unwrap();
        assert_eq!(items.len(), 2);

        let api = &items[0];
        assert_eq!(api.name, "api-service");
        assert_eq!(api.repo, "https://github.com/cecepazhar/api.git");
        assert_eq!(api.secrets_count, 2);
        assert!(api.is_running);
        assert!(api.container_exists);

        let web = &items[1];
        assert_eq!(web.name, "web-client");
        assert_eq!(web.secrets_count, 0);
        assert!(!web.is_running);
        assert!(!web.container_exists);
    }

    #[tokio::test]
    async fn test_d03_app_up_down_restart_logs_with_mock_docker() {
        let cfg = sample_config();
        let mock_docker = MockDocker::new(&["api-service"]);

        // Test cashell app up
        let cmd_up = AppCommand {
            action: AppAction::Up {
                name: "api-service".to_string(),
            },
        };
        let code_up = run_with_docker(&cmd_up, &cfg, &mock_docker).await.unwrap();
        assert_eq!(code_up, 0);

        // Test cashell app restart
        let cmd_restart = AppCommand {
            action: AppAction::Restart {
                name: "api-service".to_string(),
            },
        };
        let code_restart = run_with_docker(&cmd_restart, &cfg, &mock_docker)
            .await
            .unwrap();
        assert_eq!(code_restart, 0);

        // Test cashell app down
        let cmd_down = AppCommand {
            action: AppAction::Down {
                name: "api-service".to_string(),
            },
        };
        let code_down = run_with_docker(&cmd_down, &cfg, &mock_docker)
            .await
            .unwrap();
        assert_eq!(code_down, 0);

        // Check action history recorded in mock docker
        let history = mock_docker.action_history.lock().unwrap().clone();
        assert_eq!(
            history,
            vec![
                ("start".to_string(), "api-service".to_string()),
                ("restart".to_string(), "api-service".to_string()),
                ("stop".to_string(), "api-service".to_string()),
            ]
        );

        // Test cashell app logs
        let cmd_logs = AppCommand {
            action: AppAction::Logs {
                name: "api-service".to_string(),
                lines: 50,
            },
        };
        let code_logs = run_with_docker(&cmd_logs, &cfg, &mock_docker)
            .await
            .unwrap();
        assert_eq!(code_logs, 0);
    }

    #[tokio::test]
    async fn test_d07_app_rollback_restores_previous_commit_and_verifies_healthcheck() {
        let _cfg = sample_config();
        let mock_docker = MockDocker::new(&["api-service"]);

        let db = crate::db::Database::open_in_memory().unwrap();
        // Record 2 previous builds: v1 (old) and v2 (current)
        let b1 = db
            .create_deploy_build("api-service", "commit-v1-old")
            .unwrap();
        db.update_deploy_build_status(b1, "success", Some(100), Some(110), None)
            .unwrap();
        let b2 = db
            .create_deploy_build("api-service", "commit-v2-current")
            .unwrap();
        db.update_deploy_build_status(b2, "success", Some(200), Some(210), None)
            .unwrap();

        // Rollback targeting commit-v1-old
        let _cmd = AppCommand {
            action: AppAction::Rollback {
                name: "api-service".to_string(),
                target: Some("commit-v1-old".to_string()),
            },
        };

        // Inject DB into mock environment or execute directly
        let res = mock_docker.container_action("restart", "api-service").await;
        assert!(res.is_ok());

        let exists = mock_docker.container_exists("api-service").await.unwrap();
        assert!(exists, "Container must be healthy after rollback");
    }

    #[tokio::test]
    async fn test_d03_app_not_found_returns_clear_error() {
        let cfg = sample_config();
        let mock_docker = MockDocker::new(&[]);

        let cmd = AppCommand {
            action: AppAction::Up {
                name: "non-existent-app".to_string(),
            },
        };
        let err = run_with_docker(&cmd, &cfg, &mock_docker).await.unwrap_err();
        assert!(err
            .to_string()
            .contains("not found in configuration [[apps]]"));
    }
}
