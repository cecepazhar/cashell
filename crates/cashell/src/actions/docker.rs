//! Docker / Podman abstraction layer for typed actions (F-01).

use anyhow::{anyhow, bail, Result};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub type DockerFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Trait abstracting container runtime interactions for testability.
pub trait DockerApi: Send + Sync {
    /// Return list of all container names.
    fn list_containers<'a>(&'a self) -> DockerFuture<'a, Vec<String>>;
    /// Check if container with given name exists.
    fn container_exists<'a>(&'a self, name: &'a str) -> DockerFuture<'a, bool>;
    /// Perform an action ("start", "stop", "restart") on a container.
    fn container_action<'a>(&'a self, action: &'a str, target: &'a str)
        -> DockerFuture<'a, String>;
    /// Fetch container logs.
    fn container_logs<'a>(&'a self, name: &'a str, lines: usize) -> DockerFuture<'a, String>;
}

/// Production container runtime using system `docker` (with fallback to `podman`).
#[derive(Debug, Default, Clone)]
pub struct SystemDocker;

impl DockerApi for SystemDocker {
    fn list_containers<'a>(&'a self) -> DockerFuture<'a, Vec<String>> {
        Box::pin(async move {
            let mut result = tokio::process::Command::new("docker")
                .args(["ps", "-a", "--format", "{{.Names}}"])
                .output()
                .await;
            if result.is_err() || matches!(&result, Ok(o) if !o.status.success()) {
                let pod_result = tokio::process::Command::new("podman")
                    .args(["ps", "-a", "--format", "{{.Names}}"])
                    .output()
                    .await;
                if pod_result.is_ok() {
                    result = pod_result;
                }
            }

            match result {
                Ok(o) if o.status.success() => {
                    let out = String::from_utf8_lossy(&o.stdout);
                    let names = out
                        .lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    Ok(names)
                }
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    bail!("list containers failed: {err}");
                }
                Err(e) => bail!("docker command failed to execute: {e}"),
            }
        })
    }

    fn container_exists<'a>(&'a self, name: &'a str) -> DockerFuture<'a, bool> {
        Box::pin(async move {
            // Mock seam for test environments
            if let Ok(override_val) = std::env::var("CONTAINER_NAMES_OVERRIDE") {
                return Ok(override_val.split(',').map(|s| s.trim()).any(|s| s == name));
            }

            match self.list_containers().await {
                Ok(list) => Ok(list.iter().any(|c| c == name)),
                Err(_) => Ok(false),
            }
        })
    }

    fn container_action<'a>(
        &'a self,
        action: &'a str,
        target: &'a str,
    ) -> DockerFuture<'a, String> {
        Box::pin(async move {
            let mut result = tokio::process::Command::new("docker")
                .args([action, target])
                .output()
                .await;
            if result.is_err() || matches!(&result, Ok(o) if !o.status.success()) {
                let pod_result = tokio::process::Command::new("podman")
                    .args([action, target])
                    .output()
                    .await;
                if pod_result.is_ok() {
                    result = pod_result;
                }
            }

            match result {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    if o.status.success() {
                        let out = if stdout.trim().is_empty() {
                            format!("Action '{action}' executed on '{target}'.")
                        } else {
                            stdout
                        };
                        Ok(out)
                    } else {
                        let err = if stderr.trim().is_empty() {
                            stdout
                        } else {
                            stderr
                        };
                        bail!("{err}");
                    }
                }
                Err(e) => Err(anyhow!("docker execution error: {e}")),
            }
        })
    }

    fn container_logs<'a>(&'a self, name: &'a str, lines: usize) -> DockerFuture<'a, String> {
        Box::pin(async move {
            let lines_str = lines.to_string();
            let mut result = tokio::process::Command::new("docker")
                .args(["logs", "--tail", &lines_str, name])
                .output()
                .await;
            if result.is_err() || matches!(&result, Ok(o) if !o.status.success()) {
                let pod_result = tokio::process::Command::new("podman")
                    .args(["logs", "--tail", &lines_str, name])
                    .output()
                    .await;
                if pod_result.is_ok() {
                    result = pod_result;
                }
            }

            match result {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    let combined = format!("{stdout}{stderr}");
                    Ok(combined)
                }
                Err(e) => Err(anyhow!("docker logs error: {e}")),
            }
        })
    }
}

/// In-memory mock container runtime for unit and integration testing.
#[derive(Debug, Default, Clone)]
pub struct MockDocker {
    pub containers: Arc<Mutex<Vec<String>>>,
    pub action_history: Arc<Mutex<Vec<(String, String)>>>,
    pub logs_response: Arc<Mutex<String>>,
}

impl MockDocker {
    pub fn new(initial_containers: &[&str]) -> Self {
        Self {
            containers: Arc::new(Mutex::new(
                initial_containers.iter().map(|s| s.to_string()).collect(),
            )),
            action_history: Arc::new(Mutex::new(Vec::new())),
            logs_response: Arc::new(Mutex::new("mock log line 1\nmock log line 2\n".to_string())),
        }
    }
}

impl DockerApi for MockDocker {
    fn list_containers<'a>(&'a self) -> DockerFuture<'a, Vec<String>> {
        Box::pin(async move {
            let list = self.containers.lock().unwrap().clone();
            Ok(list)
        })
    }

    fn container_exists<'a>(&'a self, name: &'a str) -> DockerFuture<'a, bool> {
        Box::pin(async move {
            let list = self.containers.lock().unwrap();
            Ok(list.iter().any(|c| c == name))
        })
    }

    fn container_action<'a>(
        &'a self,
        action: &'a str,
        target: &'a str,
    ) -> DockerFuture<'a, String> {
        Box::pin(async move {
            let exists = self.containers.lock().unwrap().iter().any(|c| c == target);
            if !exists {
                bail!("container '{target}' not found");
            }
            self.action_history
                .lock()
                .unwrap()
                .push((action.to_string(), target.to_string()));
            Ok(format!("Mock action '{action}' on '{target}' completed."))
        })
    }

    fn container_logs<'a>(&'a self, name: &'a str, _lines: usize) -> DockerFuture<'a, String> {
        Box::pin(async move {
            let exists = self.containers.lock().unwrap().iter().any(|c| c == name);
            if !exists {
                bail!("container '{name}' not found");
            }
            let logs = self.logs_response.lock().unwrap().clone();
            Ok(logs)
        })
    }
}
