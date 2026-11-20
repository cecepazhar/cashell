//! Real build & deploy executor for `[[apps]]` (replaces the fake 50 ms
//! "Build completed successfully" step).
//!
//! Pipeline: git sync → `compose up -d --build` → health gate. Every command's
//! real output goes into the build log, and a build is only `success` when
//! the containers are actually running and not unhealthy.
//!
//! Works with Docker (`docker compose` / `docker-compose`) and Podman
//! (`podman-compose`). Only Compose projects are supported for now; a repo
//! without the configured compose file fails with a clear message.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::config::AppConfig;

/// Max bytes of one command's output kept in the build log (tail).
const LOG_TAIL_BYTES: usize = 32 * 1024;
/// How long containers get to become running/healthy after `up`.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(90);
/// Containers must stay running/healthy this long before a build counts as
/// successful (catches apps that crash a second after starting).
pub const STABLE_FOR: Duration = Duration::from_secs(5);

/// A compose implementation found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeTool {
    /// Executable, e.g. `docker`, `docker-compose`, `podman-compose`.
    pub program: String,
    /// Leading args, e.g. `["compose"]` for `docker compose`.
    pub prefix: Vec<String>,
    /// Container engine CLI used for `ps` (`docker` or `podman`).
    pub engine: String,
}

impl ComposeTool {
    pub fn label(&self) -> String {
        if self.prefix.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.prefix.join(" "))
        }
    }
}

fn runs_ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Detect a compose implementation: docker compose → docker-compose →
/// podman-compose.
pub fn detect_compose() -> Option<ComposeTool> {
    if runs_ok("docker", &["compose", "version"]) {
        return Some(ComposeTool {
            program: "docker".into(),
            prefix: vec!["compose".into()],
            engine: "docker".into(),
        });
    }
    if runs_ok("docker-compose", &["version"]) {
        return Some(ComposeTool {
            program: "docker-compose".into(),
            prefix: vec![],
            engine: "docker".into(),
        });
    }
    if runs_ok("podman-compose", &["version"]) {
        return Some(ComposeTool {
            program: "podman-compose".into(),
            prefix: vec![],
            engine: "podman".into(),
        });
    }
    None
}

/// Compose project name for an app: lowercase `[a-z0-9_-]`.
pub fn project_name(app_name: &str) -> String {
    let s: String = app_name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "app".to_string()
    } else {
        s
    }
}

fn tail(s: &str) -> &str {
    if s.len() <= LOG_TAIL_BYTES {
        return s;
    }
    let mut start = s.len() - LOG_TAIL_BYTES;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s.get(start..).unwrap_or(s)
}

/// Run a command, append `$ cmd` + its combined output to `log`.
fn run_logged(cmd: &mut Command, display: &str, log: &mut String) -> Result<String> {
    log.push_str(&format!("$ {display}\n"));
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to start `{display}`"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let t = tail(&text);
    log.push_str(t);
    if !t.ends_with('\n') && !t.is_empty() {
        log.push('\n');
    }
    if !out.status.success() {
        bail!(
            "`{display}` exited with {}",
            out.status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string())
        );
    }
    Ok(text)
}

/// State of one container of the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerHealth {
    Healthy,
    Starting,
    Failed(String),
}

/// Parse one `ps --format '{{.Names}}\t{{.State}}\t{{.Status}}'` line.
pub fn parse_ps_line(line: &str) -> Option<(String, ContainerHealth)> {
    let mut parts = line.split('\t');
    let name = parts.next()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let state = parts.next().unwrap_or("").trim().to_lowercase();
    let status = parts.next().unwrap_or("").trim().to_lowercase();
    let health = if state != "running" {
        if state == "created" || state == "restarting" || state == "configured" {
            ContainerHealth::Starting
        } else {
            ContainerHealth::Failed(format!("state {state} ({status})"))
        }
    } else if status.contains("unhealthy") {
        ContainerHealth::Failed("unhealthy".to_string())
    } else if status.contains("starting") {
        ContainerHealth::Starting
    } else {
        ContainerHealth::Healthy
    };
    Some((name, health))
}

fn project_containers(tool: &ComposeTool, project: &str) -> Result<Vec<(String, ContainerHealth)>> {
    let out = Command::new(&tool.engine)
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--format",
            "{{.Names}}\t{{.State}}\t{{.Status}}",
        ])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run `{} ps`", tool.engine))?;
    if !out.status.success() {
        bail!(
            "`{} ps` failed: {}",
            tool.engine,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(parse_ps_line)
        .collect())
}

fn engine_out(tool: &ComposeTool, args: &[&str]) -> Result<String> {
    let out = Command::new(&tool.engine)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run `{}`", tool.engine))?;
    if !out.status.success() {
        bail!(
            "`{} {}` failed",
            tool.engine,
            args.first().copied().unwrap_or("")
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Fail when a container still runs an older image than its tag now points
/// to — otherwise a build could be "success" while serving the old code.
fn ensure_fresh_images(tool: &ComposeTool, project: &str, log: &mut String) -> Result<()> {
    for (name, _) in project_containers(tool, project)? {
        let info = engine_out(
            tool,
            &["inspect", &name, "--format", "{{.Image}}|{{.Config.Image}}"],
        )?;
        let (running, tag) = info.split_once('|').unwrap_or((info.as_str(), ""));
        if tag.is_empty() {
            continue;
        }
        let latest = engine_out(tool, &["image", "inspect", tag, "--format", "{{.Id}}"])?;
        let norm = |s: &str| s.trim_start_matches("sha256:").to_string();
        if norm(running) != norm(&latest) {
            bail!("container {name} still runs an old image of {tag} (not recreated)");
        }
    }
    log.push_str("images: all containers run the freshly built images\n");
    Ok(())
}

/// Wait until every container of the project is running and not unhealthy.
fn wait_healthy(tool: &ComposeTool, project: &str, log: &mut String) -> Result<()> {
    let start = Instant::now();
    let mut healthy_since: Option<Instant> = None;
    loop {
        let containers = project_containers(tool, project)?;
        if containers.is_empty() {
            bail!("no containers found for compose project '{project}'");
        }
        if let Some((name, ContainerHealth::Failed(why))) = containers
            .iter()
            .find(|(_, h)| matches!(h, ContainerHealth::Failed(_)))
        {
            bail!("container {name} is not healthy: {why}");
        }
        if containers
            .iter()
            .all(|(_, h)| *h == ContainerHealth::Healthy)
        {
            let since = *healthy_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= STABLE_FOR {
                log.push_str(&format!(
                    "health: {} container(s) running and healthy, stable {}s (total {}s)\n",
                    containers.len(),
                    STABLE_FOR.as_secs(),
                    start.elapsed().as_secs()
                ));
                return Ok(());
            }
        } else {
            healthy_since = None;
        }
        if start.elapsed() > HEALTH_TIMEOUT {
            bail!(
                "containers still starting after {}s",
                HEALTH_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Outcome of a deploy run: always carries the full log, success or not.
#[derive(Debug, Clone)]
pub struct DeployOutcome {
    pub result: Result<String, String>,
    /// Commit actually deployed (when git sync got that far).
    pub commit: Option<String>,
    pub log: String,
}

/// Deploy `app` at `commit` (or the tip of its branch) into `app_dir`.
/// Blocking — call from `spawn_blocking`.
pub fn deploy_app(
    app: &AppConfig,
    app_dir: &Path,
    commit: Option<&str>,
    env: &HashMap<String, String>,
) -> DeployOutcome {
    let mut log = String::new();
    let mut deployed_commit = None;
    let result = (|| -> Result<String> {
        let tool = detect_compose().context(
            "no compose tool found — install `docker compose`, `docker-compose` or `podman-compose`",
        )?;
        log.push_str(&format!("compose: {}\n", tool.label()));

        log.push_str(&format!(
            "git: syncing {} ({}){}\n",
            app.repo,
            app.branch,
            commit.map(|c| format!(" @ {c}")).unwrap_or_default()
        ));
        let git = crate::deploy::sync_app_repository(&app.repo, app_dir, &app.branch, commit)?;
        log.push_str(&format!("git: at {} — {}\n", git.short_hash, git.message));
        deployed_commit = Some(git.commit_hash.clone());

        let compose_file = app_dir.join(&app.compose_path);
        if !compose_file.is_file() {
            bail!(
                "compose file '{}' not found in the repository (set compose_path in [[apps]])",
                app.compose_path
            );
        }
        let project = project_name(&app.name);
        let compose_str = compose_file.to_string_lossy().to_string();

        let mut args: Vec<String> = tool.prefix.clone();
        args.extend(
            [
                "-p",
                &project,
                "-f",
                &compose_str,
                "up",
                "-d",
                "--build",
                "--remove-orphans",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        if tool.program == "podman-compose" {
            // podman-compose keeps old containers even when the image changed.
            args.push("--force-recreate".to_string());
        }
        let display = format!(
            "{} -p {project} -f {} up -d --build --remove-orphans",
            tool.label(),
            app.compose_path
        );
        let mut cmd = Command::new(&tool.program);
        cmd.args(&args).current_dir(app_dir).envs(env);
        if tool.engine == "podman" {
            // Podman builds OCI images by default, which drop HEALTHCHECK;
            // the Docker format keeps it so the health gate can use it.
            cmd.env("BUILDAH_FORMAT", "docker");
        }
        run_logged(&mut cmd, &display, &mut log)?;

        ensure_fresh_images(&tool, &project, &mut log)?;
        wait_healthy(&tool, &project, &mut log)?;
        Ok(format!(
            "Deployed {} @ {} ({})",
            app.name, git.short_hash, git.message
        ))
    })();

    DeployOutcome {
        result: result.map_err(|e| format!("{e:#}")),
        commit: deployed_commit,
        log,
    }
}

/// Run a compose subcommand (`stop`, `restart`, `logs`, `ps`) for a deployed app.
pub fn compose_cmd(app: &AppConfig, app_dir: &Path, sub: &[&str]) -> Result<String> {
    let tool = detect_compose().context("no compose tool found")?;
    let project = project_name(&app.name);
    let compose_file = app_dir.join(&app.compose_path);
    let mut args: Vec<String> = tool.prefix.clone();
    args.extend(["-p".to_string(), project]);
    if compose_file.is_file() {
        args.extend(["-f".to_string(), compose_file.to_string_lossy().to_string()]);
    }
    args.extend(sub.iter().map(|s| s.to_string()));
    let mut log = String::new();
    let display = format!("{} {}", tool.label(), sub.join(" "));
    let mut cmd = Command::new(&tool.program);
    cmd.args(&args).current_dir(app_dir);
    run_logged(&mut cmd, &display, &mut log)
}

/// True when the app was deployed through compose (repo checked out with
/// its compose file present).
pub fn is_compose_app(app: &AppConfig, app_dir: &Path) -> bool {
    app_dir.join(&app.compose_path).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_is_sanitized() {
        assert_eq!(project_name("My App!"), "my-app");
        assert_eq!(project_name("api_v2"), "api_v2");
        assert_eq!(project_name("..."), "app");
    }

    #[test]
    fn ps_line_states() {
        assert_eq!(
            parse_ps_line("web\trunning\tUp 3 seconds (healthy)"),
            Some(("web".into(), ContainerHealth::Healthy))
        );
        assert_eq!(
            parse_ps_line("web\trunning\tUp 1 second (health: starting)"),
            Some(("web".into(), ContainerHealth::Starting))
        );
        assert!(matches!(
            parse_ps_line("web\trunning\tUp 9 seconds (unhealthy)"),
            Some((_, ContainerHealth::Failed(_)))
        ));
        assert!(matches!(
            parse_ps_line("web\texited\tExited (1) 2 seconds ago"),
            Some((_, ContainerHealth::Failed(_)))
        ));
        assert_eq!(
            parse_ps_line("web\trunning\tUp 5 seconds"),
            Some(("web".into(), ContainerHealth::Healthy))
        );
        assert_eq!(parse_ps_line(""), None);
    }

    #[test]
    fn log_tail_keeps_end_on_char_boundary() {
        let s = "é".repeat(LOG_TAIL_BYTES);
        let t = tail(&s);
        assert!(t.len() <= LOG_TAIL_BYTES);
        assert!(t.chars().all(|c| c == 'é'));
    }
}
