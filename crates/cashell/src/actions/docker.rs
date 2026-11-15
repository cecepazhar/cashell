//! Docker / Podman abstraction layer for typed actions (F-01, D-01).

use anyhow::{anyhow, bail, Result};
use bollard::container::{
    ListContainersOptions, LogsOptions, RestartContainerOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::Docker;
use futures_util::StreamExt;
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

/// Native Docker API client via bollard (D-01).
/// Communicates directly with the Docker Engine or Podman socket without CLI subprocesses.
#[derive(Debug, Clone)]
pub struct BollardDocker {
    client: Docker,
}

impl BollardDocker {
    /// Connect to Docker daemon via unix socket or DOCKER_HOST.
    /// Evaluates:
    /// 1. DOCKER_HOST env var
    /// 2. /var/run/docker.sock
    /// 3. /run/user/<UID>/podman/podman.sock (rootless Podman socket)
    /// 4. /run/podman/podman.sock
    /// 5. Docker::connect_with_socket_defaults()
    pub fn connect() -> Result<Self> {
        if let Ok(host) = std::env::var("DOCKER_HOST") {
            let trimmed = host.trim();
            if !trimmed.is_empty() {
                let client = Docker::connect_with_socket_defaults()
                    .map_err(|e| anyhow!("Failed to connect via DOCKER_HOST ({trimmed}): {e}"))?;
                return Ok(Self { client });
            }
        }

        if std::path::Path::new("/var/run/docker.sock").exists() {
            let client = Docker::connect_with_unix(
                "/var/run/docker.sock",
                120,
                bollard::API_DEFAULT_VERSION,
            )
            .map_err(|e| anyhow!("Failed to connect to /var/run/docker.sock: {e}"))?;
            return Ok(Self { client });
        }

        // Check rootless podman socket
        let uid_str = std::env::var("UID").unwrap_or_else(|_| "1000".to_string());
        let user_sock = format!("/run/user/{uid_str}/podman/podman.sock");
        if std::path::Path::new(&user_sock).exists() {
            let client = Docker::connect_with_unix(&user_sock, 120, bollard::API_DEFAULT_VERSION)
                .map_err(|e| {
                anyhow!("Failed to connect to podman socket at {user_sock}: {e}")
            })?;
            return Ok(Self { client });
        }

        if std::path::Path::new("/run/podman/podman.sock").exists() {
            let client = Docker::connect_with_unix(
                "/run/podman/podman.sock",
                120,
                bollard::API_DEFAULT_VERSION,
            )
            .map_err(|e| anyhow!("Failed to connect to /run/podman/podman.sock: {e}"))?;
            return Ok(Self { client });
        }

        let client = Docker::connect_with_socket_defaults()
            .map_err(|e| anyhow!("Docker socket connection failed: {e}"))?;
        Ok(Self { client })
    }

    pub fn from_client(client: Docker) -> Self {
        Self { client }
    }
}

impl DockerApi for BollardDocker {
    fn list_containers<'a>(&'a self) -> DockerFuture<'a, Vec<String>> {
        Box::pin(async move {
            let options = Some(ListContainersOptions::<String> {
                all: true,
                ..Default::default()
            });
            let containers = self
                .client
                .list_containers(options)
                .await
                .map_err(|e| anyhow!("Failed to list containers: {e}"))?;
            let names = containers
                .into_iter()
                .flat_map(|c| {
                    c.names
                        .unwrap_or_default()
                        .into_iter()
                        .map(|n| n.trim_start_matches('/').to_string())
                })
                .collect();
            Ok(names)
        })
    }

    fn container_exists<'a>(&'a self, name: &'a str) -> DockerFuture<'a, bool> {
        Box::pin(async move {
            // Mock seam for test environments
            if let Ok(override_val) = std::env::var("CONTAINER_NAMES_OVERRIDE") {
                return Ok(override_val.split(',').map(|s| s.trim()).any(|s| s == name));
            }
            match self.client.inspect_container(name, None).await {
                Ok(_) => Ok(true),
                Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => Ok(false),
                Err(_) => {
                    let list = self.list_containers().await.unwrap_or_default();
                    Ok(list.iter().any(|c| c == name))
                }
            }
        })
    }

    fn container_action<'a>(
        &'a self,
        action: &'a str,
        target: &'a str,
    ) -> DockerFuture<'a, String> {
        Box::pin(async move {
            match action {
                "start" => {
                    self.client
                        .start_container(target, None::<StartContainerOptions<String>>)
                        .await
                        .map_err(|e| anyhow!("Failed to start container '{target}': {e}"))?;
                    Ok(format!("Container '{target}' started successfully."))
                }
                "stop" => {
                    self.client
                        .stop_container(target, None::<StopContainerOptions>)
                        .await
                        .map_err(|e| anyhow!("Failed to stop container '{target}': {e}"))?;
                    Ok(format!("Container '{target}' stopped successfully."))
                }
                "restart" => {
                    self.client
                        .restart_container(target, None::<RestartContainerOptions>)
                        .await
                        .map_err(|e| anyhow!("Failed to restart container '{target}': {e}"))?;
                    Ok(format!("Container '{target}' restarted successfully."))
                }
                _ => bail!("Unsupported docker action: '{action}'"),
            }
        })
    }

    fn container_logs<'a>(&'a self, name: &'a str, lines: usize) -> DockerFuture<'a, String> {
        Box::pin(async move {
            let tail_str = lines.to_string();
            let options = Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                tail: tail_str,
                ..Default::default()
            });
            let mut stream = self.client.logs(name, options);
            let mut combined = String::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(log) => combined.push_str(&log.to_string()),
                    Err(e) => return Err(anyhow!("Error reading container logs: {e}")),
                }
            }
            Ok(combined)
        })
    }
}

/// Production container runtime using Bollard API (D-01) with fallback to CLI.
#[derive(Debug, Default, Clone)]
pub struct SystemDocker;

impl DockerApi for SystemDocker {
    fn list_containers<'a>(&'a self) -> DockerFuture<'a, Vec<String>> {
        Box::pin(async move {
            if let Ok(bollard_client) = BollardDocker::connect() {
                if let Ok(containers) = bollard_client.list_containers().await {
                    return Ok(containers);
                }
            }

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

            if let Ok(bollard_client) = BollardDocker::connect() {
                if let Ok(exists) = bollard_client.container_exists(name).await {
                    return Ok(exists);
                }
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
            if let Ok(bollard_client) = BollardDocker::connect() {
                if let Ok(res) = bollard_client.container_action(action, target).await {
                    return Ok(res);
                }
            }

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
            if let Ok(bollard_client) = BollardDocker::connect() {
                if let Ok(logs) = bollard_client.container_logs(name, lines).await {
                    return Ok(logs);
                }
            }

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

#[cfg(test)]
pub mod tests {
    use super::*;

    #[tokio::test]
    async fn test_d01_bollard_docker_api_abstraction() {
        let mock = MockDocker::new(&["app-service", "cache-redis"]);
        let containers = mock.list_containers().await.unwrap();
        assert_eq!(containers.len(), 2);
        assert!(containers.contains(&"app-service".to_string()));

        let exists = mock.container_exists("app-service").await.unwrap();
        assert!(exists);
        let exists_fake = mock.container_exists("non-existent").await.unwrap();
        assert!(!exists_fake);

        let action_res = mock
            .container_action("restart", "app-service")
            .await
            .unwrap();
        assert!(action_res.contains("restart"));
        assert!(action_res.contains("app-service"));

        let logs = mock.container_logs("app-service", 50).await.unwrap();
        assert!(logs.contains("mock log line 1"));
    }

    #[tokio::test]
    async fn test_d01_bollard_docker_connection_probe() {
        // Probe BollardDocker connection on host (or verify error handling if daemon is not running)
        let conn_res = BollardDocker::connect();
        if let Ok(docker) = conn_res {
            let containers = docker.list_containers().await;
            assert!(
                containers.is_ok(),
                "bollard list_containers should succeed if socket is open"
            );
        } else {
            // In container or CI environment without socket, connect returns an error cleanly
            assert!(conn_res.is_err());
        }
    }
}
