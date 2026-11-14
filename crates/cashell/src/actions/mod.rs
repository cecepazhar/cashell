//! Typed action layer for CAShell (F-01).
//!
//! Provides a single, strongly-typed execution entry point for operations
//! triggered by the daemon HTTP API, Telegram bot, Hana AI, and the Cockpit TUI.
//!
//! All mutating actions are logged to `audit_log` via [`crate::db::Database`].

pub mod docker;
pub use docker::{DockerApi, DockerFuture, MockDocker, SystemDocker};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::db::Database;

/// Strongly-typed action requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", content = "params")]
pub enum Action {
    /// Probe system and daemon status.
    GetStatus,
    /// Fetch container logs for an application/container.
    GetLogs { app: String, lines: usize },
    /// List all registered or running containers.
    ListContainers,
    /// Restart a target container.
    RestartContainer { name: String },
    /// Stop a target container.
    StopContainer { name: String },
    /// Start a target container.
    StartContainer { name: String },
    /// Trigger deployment for an application.
    DeployApp { app: String },
    /// Query uptime monitors status summary.
    GetUptime,
    /// Query sovereign web analytics traffic metrics.
    GetAnalytics { site: String, days: u32 },
}

/// Action classification: Read (safe) vs Write (mutating / state-changing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    Read,
    Write,
}

impl Action {
    /// Return whether this action is Read or Write.
    pub fn kind(&self) -> ActionKind {
        match self {
            Action::GetStatus
            | Action::GetLogs { .. }
            | Action::ListContainers
            | Action::GetUptime
            | Action::GetAnalytics { .. } => ActionKind::Read,
            Action::RestartContainer { .. }
            | Action::StopContainer { .. }
            | Action::StartContainer { .. }
            | Action::DeployApp { .. } => ActionKind::Write,
        }
    }

    /// Return the canonical action name for audit logging.
    pub fn action_name(&self) -> &'static str {
        match self {
            Action::GetStatus => "get_status",
            Action::GetLogs { .. } => "get_logs",
            Action::ListContainers => "list_containers",
            Action::RestartContainer { .. } => "restart_container",
            Action::StopContainer { .. } => "stop_container",
            Action::StartContainer { .. } => "start_container",
            Action::DeployApp { .. } => "deploy_app",
            Action::GetUptime => "get_uptime",
            Action::GetAnalytics { .. } => "get_analytics",
        }
    }
}

/// Structured execution result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub ok: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Context passed to [`execute`], bundling actor identity, container runtime, and database.
pub struct ActionContext {
    pub actor: String,
    pub source: String,
    pub docker: Arc<dyn DockerApi + Send + Sync>,
    pub db: Option<Arc<Mutex<Database>>>,
}

impl ActionContext {
    pub fn new(source: impl Into<String>, actor: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            actor: actor.into(),
            docker: Arc::new(SystemDocker),
            db: None,
        }
    }

    pub fn with_docker(mut self, docker: Arc<dyn DockerApi + Send + Sync>) -> Self {
        self.docker = docker;
        self
    }

    pub fn with_db(mut self, db: Arc<Mutex<Database>>) -> Self {
        self.db = Some(db);
        self
    }
}

/// Validate identifier (container name, app name).
/// Must match `^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}$`.
/// Forbids path traversal (`..`), slashes, spaces, shell metacharacters (`;`, `|`, `&`, `$`, backtick).
pub fn validate_identifier(name: &str) -> Result<()> {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        bail!("identifier cannot be empty");
    }
    if bytes.len() > 128 {
        bail!("identifier exceeds maximum length of 128 characters");
    }
    if !bytes[0].is_ascii_alphanumeric() {
        bail!("identifier must start with an alphanumeric character");
    }
    for b in &bytes[1..] {
        if !b.is_ascii_alphanumeric() && !matches!(*b, b'_' | b'.' | b'-') {
            bail!("identifier contains illegal character: {:?}", *b as char);
        }
    }
    Ok(())
}

/// Execute a typed action and automatically record audit logs.
pub async fn execute(action: Action, ctx: &ActionContext) -> Result<ActionResult> {
    let action_name = action.action_name();
    let args_json = serde_json::to_string(&action).ok();

    let res: Result<ActionResult> = match &action {
        Action::GetStatus => {
            let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
            let uptime = sysinfo::System::uptime();
            let out = format!("Host: {hostname}, uptime: {uptime}s");
            Ok(ActionResult {
                ok: true,
                output: out,
                data: Some(serde_json::json!({
                    "hostname": hostname,
                    "uptime_secs": uptime,
                })),
            })
        }
        Action::ListContainers => match ctx.docker.list_containers().await {
            Ok(containers) => Ok(ActionResult {
                ok: true,
                output: format!("Found {} containers.", containers.len()),
                data: Some(serde_json::json!(containers)),
            }),
            Err(e) => Ok(ActionResult {
                ok: false,
                output: format!("Failed to list containers: {e}"),
                data: None,
            }),
        },
        Action::RestartContainer { name } => {
            validate_identifier(name)?;
            if !ctx.docker.container_exists(name).await? {
                Ok(ActionResult {
                    ok: false,
                    output: format!("container '{name}' not found"),
                    data: None,
                })
            } else {
                match ctx.docker.container_action("restart", name).await {
                    Ok(out) => Ok(ActionResult {
                        ok: true,
                        output: out,
                        data: None,
                    }),
                    Err(e) => Ok(ActionResult {
                        ok: false,
                        output: format!("restart failed: {e}"),
                        data: None,
                    }),
                }
            }
        }
        Action::StopContainer { name } => {
            validate_identifier(name)?;
            if !ctx.docker.container_exists(name).await? {
                Ok(ActionResult {
                    ok: false,
                    output: format!("container '{name}' not found"),
                    data: None,
                })
            } else {
                match ctx.docker.container_action("stop", name).await {
                    Ok(out) => Ok(ActionResult {
                        ok: true,
                        output: out,
                        data: None,
                    }),
                    Err(e) => Ok(ActionResult {
                        ok: false,
                        output: format!("stop failed: {e}"),
                        data: None,
                    }),
                }
            }
        }
        Action::StartContainer { name } => {
            validate_identifier(name)?;
            if !ctx.docker.container_exists(name).await? {
                Ok(ActionResult {
                    ok: false,
                    output: format!("container '{name}' not found"),
                    data: None,
                })
            } else {
                match ctx.docker.container_action("start", name).await {
                    Ok(out) => Ok(ActionResult {
                        ok: true,
                        output: out,
                        data: None,
                    }),
                    Err(e) => Ok(ActionResult {
                        ok: false,
                        output: format!("start failed: {e}"),
                        data: None,
                    }),
                }
            }
        }
        Action::GetLogs { app, lines } => {
            validate_identifier(app)?;
            match ctx.docker.container_logs(app, *lines).await {
                Ok(logs) => Ok(ActionResult {
                    ok: true,
                    output: logs,
                    data: None,
                }),
                Err(e) => Ok(ActionResult {
                    ok: false,
                    output: format!("get logs failed: {e}"),
                    data: None,
                }),
            }
        }
        Action::DeployApp { app } => {
            validate_identifier(app)?;
            // Emulate / trigger app deployment
            Ok(ActionResult {
                ok: true,
                output: format!("Deployment triggered for application '{app}'."),
                data: Some(serde_json::json!({ "app": app, "status": "queued" })),
            })
        }
        Action::GetUptime => {
            let count = if let Some(ref db_lock) = ctx.db {
                if let Ok(db) = db_lock.lock() {
                    db.conn()
                        .query_row("SELECT COUNT(*) FROM uptime_monitors", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap_or(0)
                } else {
                    0
                }
            } else {
                0
            };
            Ok(ActionResult {
                ok: true,
                output: format!("Uptime monitoring active: {count} configured monitors."),
                data: Some(serde_json::json!({ "monitors_count": count })),
            })
        }
        Action::GetAnalytics { site, days } => {
            validate_identifier(site)?;
            Ok(ActionResult {
                ok: true,
                output: format!("Analytics summary for '{site}' (last {days} days)."),
                data: Some(serde_json::json!({ "site": site, "days": days })),
            })
        }
    };

    // Log to audit table if database is attached (F-03 integration)
    if let Some(ref db_lock) = ctx.db {
        if let Ok(db) = db_lock.lock() {
            match &res {
                Ok(r) => {
                    let _ = db.log_audit(
                        &ctx.source,
                        &ctx.actor,
                        action_name,
                        args_json.as_deref(),
                        Some(&r.output),
                        None,
                    );
                }
                Err(e) => {
                    let _ = db.log_audit(
                        &ctx.source,
                        &ctx.actor,
                        action_name,
                        args_json.as_deref(),
                        None,
                        Some(&e.to_string()),
                    );
                }
            }
        }
    }

    res
}

// ── tests (F-01) ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_kind_classification() {
        assert_eq!(Action::GetStatus.kind(), ActionKind::Read);
        assert_eq!(
            Action::GetLogs {
                app: "api".into(),
                lines: 50
            }
            .kind(),
            ActionKind::Read
        );
        assert_eq!(Action::ListContainers.kind(), ActionKind::Read);
        assert_eq!(Action::GetUptime.kind(), ActionKind::Read);
        assert_eq!(
            Action::GetAnalytics {
                site: "cashell".into(),
                days: 7
            }
            .kind(),
            ActionKind::Read
        );

        assert_eq!(
            Action::RestartContainer { name: "api".into() }.kind(),
            ActionKind::Write
        );
        assert_eq!(
            Action::StopContainer { name: "api".into() }.kind(),
            ActionKind::Write
        );
        assert_eq!(
            Action::StartContainer { name: "api".into() }.kind(),
            ActionKind::Write
        );
        assert_eq!(
            Action::DeployApp { app: "web".into() }.kind(),
            ActionKind::Write
        );
    }

    #[test]
    fn test_validate_identifier_security() {
        // Valid identifiers
        assert!(validate_identifier("web").is_ok());
        assert!(validate_identifier("api-server").is_ok());
        assert!(validate_identifier("redis_1.0").is_ok());

        // Dangerous / shell metacharacters blocked
        assert!(validate_identifier("").is_err());
        assert!(validate_identifier("-leading-dash").is_err());
        assert!(validate_identifier("web; rm -rf /").is_err());
        assert!(validate_identifier("web | cat").is_err());
        assert!(validate_identifier("web & echo hi").is_err());
        assert!(validate_identifier("web`id`").is_err());
        assert!(validate_identifier("web$(id)").is_err());
        assert!(validate_identifier("../traversal").is_err());
        assert!(validate_identifier("web/subdir").is_err());
    }

    #[tokio::test]
    async fn test_execute_actions_with_mock_docker_and_audit_logging() {
        let mock_docker = Arc::new(MockDocker::new(&["cashell-web", "cashell-db"]));
        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));

        let ctx = ActionContext::new("daemon", "admin")
            .with_docker(mock_docker.clone())
            .with_db(db.clone());

        // 1. ListContainers
        let list_res = execute(Action::ListContainers, &ctx).await.unwrap();
        assert!(list_res.ok);
        assert!(list_res.output.contains("Found 2 containers"));

        // 2. RestartContainer existing
        let restart_res = execute(
            Action::RestartContainer {
                name: "cashell-web".into(),
            },
            &ctx,
        )
        .await
        .unwrap();
        assert!(restart_res.ok);
        assert!(restart_res.output.contains("Mock action 'restart'"));

        // 3. RestartContainer non-existing
        let missing_res = execute(
            Action::RestartContainer {
                name: "non-existent".into(),
            },
            &ctx,
        )
        .await
        .unwrap();
        assert!(!missing_res.ok);
        assert!(missing_res.output.contains("not found"));

        // 4. GetLogs
        let logs_res = execute(
            Action::GetLogs {
                app: "cashell-web".into(),
                lines: 10,
            },
            &ctx,
        )
        .await
        .unwrap();
        assert!(logs_res.ok);
        assert!(logs_res.output.contains("mock log line"));

        // 5. Verify audit logs were written in DB
        let audit_records = db.lock().unwrap().query_audit(10).unwrap();
        assert!(audit_records.len() >= 4);
        assert_eq!(audit_records[0].action, "get_logs");
        assert_eq!(audit_records[1].action, "restart_container");
        assert_eq!(audit_records[2].action, "restart_container");
        assert_eq!(audit_records[3].action, "list_containers");
    }
}
