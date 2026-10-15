//! deploy command — deploy applications via Coolify API.

use crate::cli::DeployCommand;
use crate::config::Config;
use crate::output;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Deployment result.
pub struct DeployResult {
    pub success: bool,
    pub deployment_uuid: Option<String>,
    pub duration_seconds: f64,
    pub error: Option<String>,
}

/// Spawn an ASCII spinner on stderr; returns a handle to stop it.
fn start_spinner(message: &'static str) -> Arc<AtomicBool> {
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    std::thread::spawn(move || {
        let frames = ['|', '/', '-', '\\'];
        let mut i = 0;
        while !flag.load(Ordering::Relaxed) {
            eprint!("\r{} {}", frames[i % frames.len()], message);
            let _ = std::io::Write::flush(&mut std::io::stderr());
            std::thread::sleep(Duration::from_millis(150));
            i += 1;
        }
        eprint!("\r\x1b[2K");
        let _ = std::io::Write::flush(&mut std::io::stderr());
    });
    done
}

/// Poll deployment status from Coolify API.
fn poll_deployment(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    deployment_uuid: &str,
) -> Result<(String, Option<String>, Option<String>)> {
    let url = format!("{base_url}/deployments/{deployment_uuid}");
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()?;

    if resp.status().is_success() {
        let body: serde_json::Value = resp.json()?;
        let status = body
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        let url = body.get("url").and_then(|u| u.as_str()).map(String::from);
        let commit = body
            .get("commit")
            .and_then(|c| c.as_str())
            .map(String::from);
        Ok((status.to_string(), url, commit))
    } else {
        anyhow::bail!("Failed to get deployment status: {}", resp.status());
    }
}

pub fn run(cmd: &DeployCommand, cfg: &Config) -> Result<DeployResult> {
    let apps = &cfg.deploy.apps;

    // Validate slug
    let app = apps
        .get(&cmd.slug)
        .ok_or_else(|| anyhow::anyhow!("App '{}' not found in deploy.apps", cmd.slug))?;

    // Dry run
    if cmd.dry_run {
        output::info(&format!(
            "Dry run: would deploy {} (uuid: {}) to {}",
            cmd.slug, app.uuid, cmd.env
        ));
        return Ok(DeployResult {
            success: true,
            deployment_uuid: None,
            duration_seconds: 0.0,
            error: None,
        });
    }

    let coolify = cfg
        .deploy
        .coolify
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Coolify config not set ([deploy.coolify])"))?;
    let base_url = &coolify.base_url;
    if base_url.is_empty() {
        anyhow::bail!("Coolify base_url not configured");
    }
    let token = resolve_token(&coolify.api_token)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(coolify.timeout_seconds))
        .build()?;

    let start = std::time::Instant::now();

    // Trigger deployment
    let deploy_url = format!("{base_url}/deploy?uuid={}", app.uuid);
    let resp = client
        .post(&deploy_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("Deploy trigger failed: {} - {}", status, body);
    }

    let deploy_body: serde_json::Value = resp.json()?;
    let deployment_uuid = deploy_body
        .get("id")
        .or(deploy_body.get("deployment_uuid"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let deployment_uuid = match deployment_uuid {
        Some(u) => u,
        None => anyhow::bail!("No deployment UUID returned"),
    };

    output::info(&format!("Deployment started: {deployment_uuid}"));

    // Spinner + poll loop (max 120 attempts @ 3s)
    let spinner = start_spinner("Deploying…");
    let mut last_status = String::new();
    let mut app_url: Option<String> = None;
    let mut attempts = 0u32;
    let outcome = loop {
        if attempts >= 120 {
            break Ok(None);
        }
        attempts += 1;
        match poll_deployment(&client, base_url, &token, &deployment_uuid) {
            Ok((status, url, _commit)) => {
                if status != last_status {
                    last_status = status.clone();
                    output::info(&format!("Status: {status}"));
                }
                if url.is_some() {
                    app_url = url;
                }
                match status.as_str() {
                    "finished" | "success" => break Ok(Some(true)),
                    "failed" | "error" | "cancelled" => break Ok(Some(false)),
                    _ => std::thread::sleep(Duration::from_secs(3)),
                }
            }
            Err(e) => break Err(e),
        }
    };
    spinner.store(true, Ordering::Relaxed);

    let duration_seconds = start.elapsed().as_secs_f64();
    match outcome {
        Ok(Some(true)) => {
            if let Some(url) = &app_url {
                output::info(&format!("App URL: {url}"));
            }
            Ok(DeployResult {
                success: true,
                deployment_uuid: Some(deployment_uuid),
                duration_seconds,
                error: None,
            })
        }
        Ok(Some(false)) => Ok(DeployResult {
            success: false,
            deployment_uuid: Some(deployment_uuid),
            duration_seconds,
            error: Some(format!("Deployment failed with status: {last_status}")),
        }),
        Ok(None) => Ok(DeployResult {
            success: false,
            deployment_uuid: Some(deployment_uuid),
            duration_seconds,
            error: Some("Deployment timed out after 120 polls (360s)".to_string()),
        }),
        Err(e) => Err(e),
    }
}

/// Resolve token: support env: prefix.
fn resolve_token(token: &str) -> Result<String> {
    if let Some(env_var) = token.strip_prefix("env:") {
        std::env::var(env_var)
            .map_err(|_| anyhow::anyhow!("Environment variable {env_var} not set"))
    } else {
        Ok(token.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_token_with_env() {
        std::env::set_var("TEST_TOKEN_RESOLVE", "test_value");
        assert_eq!(
            resolve_token("env:TEST_TOKEN_RESOLVE").unwrap(),
            "test_value"
        );
        std::env::remove_var("TEST_TOKEN_RESOLVE");
    }

    #[test]
    fn test_resolve_token_plain() {
        assert_eq!(resolve_token("plain_token").unwrap(), "plain_token");
    }

    #[test]
    fn test_resolve_token_missing_env() {
        assert!(resolve_token("env:NONEXISTENT_VAR_12345").is_err());
    }
}
