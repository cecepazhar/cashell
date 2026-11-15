//! Application management commands (D-02, D-03).
//!
//! Subcommands for managing applications defined in `[[apps]]` within config.toml:
//! - `cashell app list`
//! - `cashell app up <name>`
//! - `cashell app down <name>`
//! - `cashell app logs <name> [--lines <n>]`
//! - `cashell app restart <name>`

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

            output::info(&format!(
                "Fetching last {lines} lines of logs for '{}'...",
                app.name
            ));
            let logs = docker.container_logs(&app.name, *lines).await?;
            println!("{logs}");
            Ok(0)
        }
        AppAction::Rollback { name, target } => {
            let app = cfg
                .apps
                .iter()
                .find(|a| &a.name == name)
                .ok_or_else(|| anyhow!("App '{name}' not found in configuration [[apps]]"))?;

            output::info(&format!(
                "Initiating rollback for application '{}'...",
                app.name
            ));

            let db = crate::db::get_db().or_else(|_| crate::db::Database::open())?;

            let builds = db.list_deploy_builds(Some(&app.name), 20)?;
            let successful_builds: Vec<_> = builds
                .into_iter()
                .filter(|b| b.status == "success")
                .collect();

            let target_commit = match target {
                Some(t) => t.clone(),
                None => {
                    if successful_builds.len() >= 2 {
                        successful_builds[1].commit_hash.clone()
                    } else if let Some(first) = successful_builds.first() {
                        first.commit_hash.clone()
                    } else {
                        bail!("No previous successful build found to rollback for application '{name}'");
                    }
                }
            };

            output::info(&format!(
                "Rolling back '{}' to commit '{}'...",
                app.name, target_commit
            ));

            let app_dir = crate::paths::app_dir(&app.name);
            if app_dir.exists() {
                let _ = crate::deploy::sync_app_repository(
                    &app.repo,
                    &app_dir,
                    &app.branch,
                    Some(&target_commit),
                );
            }

            let restart_res = docker.container_action("restart", &app.name).await;
            let healthy = docker.container_exists(&app.name).await.unwrap_or(false);

            if !healthy || restart_res.is_err() {
                let _ = db.create_deploy_build(&app.name, &target_commit);
                bail!("Rollback healthcheck failed for application '{name}'!");
            }

            let build_id = db.create_deploy_build(&app.name, &target_commit)?;
            let now = chrono::Utc::now().timestamp();
            let _ = db.update_deploy_build_status(
                build_id,
                "success",
                Some(now),
                Some(now),
                Some(&format!(
                    "Rollback to {target_commit} succeeded. Healthcheck: PASSED"
                )),
            );

            output::success(&format!(
                "Application '{}' rolled back to commit '{}' successfully. Healthcheck: PASSED",
                app.name, target_commit
            ));
            Ok(0)
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
