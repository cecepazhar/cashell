//! Multi-vendor AI proxy client & subcommands — `cashell ai` (REQ-P2-02).
//!
//! Providers: OmniRoute, OpenAI, Anthropic, Local (Ollama). Fallback chain,
//! SSE streaming on TTY, buffered plain output in pipe mode, cumulative 45s budget.

use crate::cli::{AiCommand, AiMode};
use crate::config::{AiConfig, AiProviderConfig, Config};
use crate::output;
use anyhow::{bail, Result};
use reqwest::blocking::Client;
use serde::Serialize;
use std::io::{BufRead, Write};
use std::process::Command;
use std::time::{Duration, Instant};

const TOTAL_BUDGET: Duration = Duration::from_secs(45);

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
}

/// Resolve `env:NAME` indirection or return the raw value.
fn resolve_secret(val: &str) -> String {
    match val.strip_prefix("env:") {
        Some(var) => std::env::var(var).unwrap_or_default(),
        None => val.to_string(),
    }
}

/// One provider attempt. Streams to stdout when `stream` is true, otherwise buffers.
fn attempt_provider(
    client: &Client,
    name: &str,
    cfg: &AiConfig,
    system_prompt: Option<&str>,
    user_prompt: &str,
    stream: bool,
) -> Result<String> {
    let p: &AiProviderConfig = cfg
        .providers
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("provider '{name}' not configured"))?;

    let timeout = Duration::from_secs(cfg.timeout_seconds.clamp(5, 300));
    let key = resolve_secret(&p.api_key);

    // 9router ships with an empty model + key on purpose: the user must set both
    // by hand so no credential is ever baked into the defaults.
    if name == "9router" && p.default_model.is_empty() {
        bail!(
            "9router model not set — run: cashell config set ai.model <model-id> \
             (e.g. cashell config set ai.model ZA126_PRO)"
        );
    }
    let model = if p.default_model.is_empty() {
        "auto"
    } else {
        &p.default_model
    };

    if p.base_url.contains("api.anthropic.com") {
        return call_anthropic(
            client,
            &p.base_url,
            &key,
            model,
            system_prompt,
            user_prompt,
            timeout,
        );
    }
    if name == "local" || p.base_url.contains("11434") {
        return call_ollama(
            client,
            &p.base_url,
            model,
            system_prompt,
            user_prompt,
            timeout,
        );
    }

    // OpenAI-compatible chat completions.
    let url = format!("{}/chat/completions", p.base_url.trim_end_matches('/'));
    let mut messages = Vec::new();
    if let Some(sys) = system_prompt {
        messages.push(Message {
            role: "system",
            content: sys,
        });
    }
    messages.push(Message {
        role: "user",
        content: user_prompt,
    });

    let payload = ChatRequest {
        model,
        messages,
        stream,
    };

    let mut req = client.post(&url).timeout(timeout).json(&payload);
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let resp = req.send()?;
    check_status(&resp, name, &key)?;

    if stream {
        read_sse(resp)
    } else {
        let body: serde_json::Value = resp.json()?;
        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if text.is_empty() {
            bail!("empty response from {name}");
        }
        Ok(text)
    }
}

/// Fail fast on rate limits and server errors so the caller fails over.
fn check_status(resp: &reqwest::blocking::Response, name: &str, key: &str) -> Result<()> {
    let status = resp.status();
    if status.as_u16() == 429 {
        bail!("{name}: HTTP 429 rate limited");
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        if key.is_empty() {
            bail!(
                "{name}: HTTP {status} — API key is empty. \
                 Set it with: cashell config set ai.api_key <key> \
                 (use `--provider {name}` to target a non-default provider)"
            );
        }
        bail!("{name}: HTTP {status} — authentication failed (401/403), check your API key");
    }
    if status.is_server_error() {
        bail!("{name}: HTTP {status}");
    }
    if !status.is_success() {
        bail!("{name}: HTTP {status}");
    }
    Ok(())
}

/// Parse an SSE stream: emit `data:` deltas live, return the concatenation.
fn read_sse(resp: reqwest::blocking::Response) -> Result<String> {
    let mut reader = std::io::BufReader::new(resp);
    let mut accumulated = String::new();
    let mut out = std::io::stdout().lock();
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        let Some(data) = trimmed.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if let Some(delta) = chunk["choices"][0]["delta"]["content"].as_str() {
            accumulated.push_str(delta);
            let _ = out.write_all(delta.as_bytes());
            let _ = out.flush();
        }
    }
    let _ = writeln!(out);
    if accumulated.is_empty() {
        bail!("empty streamed response");
    }
    Ok(accumulated)
}

fn call_anthropic(
    client: &Client,
    base_url: &str,
    key: &str,
    model: &str,
    system_prompt: Option<&str>,
    user_prompt: &str,
    timeout: Duration,
) -> Result<String> {
    let url = format!("{}/messages", base_url.trim_end_matches('/'));
    let mut payload = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "messages": [{ "role": "user", "content": user_prompt }],
    });
    if let Some(sys) = system_prompt {
        payload["system"] = serde_json::json!(sys);
    }
    let resp = client
        .post(&url)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .timeout(timeout)
        .json(&payload)
        .send()?;
    check_status(&resp, "anthropic", key)?;
    let body: serde_json::Value = resp.json()?;
    let text = body["content"][0]["text"].as_str().unwrap_or_default();
    if text.is_empty() {
        bail!("empty anthropic response");
    }
    Ok(text.to_string())
}

fn call_ollama(
    client: &Client,
    base_url: &str,
    model: &str,
    system_prompt: Option<&str>,
    user_prompt: &str,
    timeout: Duration,
) -> Result<String> {
    let url = format!("{}/generate", base_url.trim_end_matches('/'));
    let prompt = match system_prompt {
        Some(sys) => format!("{sys}\n\n{user_prompt}"),
        None => user_prompt.to_string(),
    };
    let payload = serde_json::json!({ "model": model, "prompt": prompt, "stream": false });
    let resp = client.post(&url).timeout(timeout).json(&payload).send()?;
    check_status(&resp, "local", "")?;
    let body: serde_json::Value = resp.json()?;
    let text = body["response"].as_str().unwrap_or_default();
    if text.is_empty() {
        bail!("empty ollama response");
    }
    Ok(text.to_string())
}

/// Run `user_prompt` through the configured fallback chain.
pub fn query(cfg: &Config, system_prompt: Option<&str>, user_prompt: &str) -> Result<String> {
    query_opts(cfg, system_prompt, user_prompt, output::is_tty())
}

/// Like [`query`] but never streams to stdout — used by `audit` so raw model
/// deltas don't interleave with the rendered report on a TTY.
pub fn query_buffered(cfg: &Config, system_prompt: Option<&str>, user_prompt: &str) -> Result<String> {
    query_opts(cfg, system_prompt, user_prompt, false)
}

fn query_opts(
    cfg: &Config,
    system_prompt: Option<&str>,
    user_prompt: &str,
    stream: bool,
) -> Result<String> {
    let ai = &cfg.ai;
    let client = Client::builder().build()?;
    let start = Instant::now();
    let max_attempts = ai.max_retries.clamp(0, 1) + 1;

    for provider in &ai.fallback_chain {
        for attempt in 1..=max_attempts {
            if start.elapsed() >= TOTAL_BUDGET {
                bail!("AI fallback chain exceeded the 45s wall-clock budget");
            }
            match attempt_provider(&client, provider, ai, system_prompt, user_prompt, stream) {
                Ok(answer) => return Ok(answer),
                Err(err) => {
                    eprintln!(
                        "{}",
                        colored::Colorize::yellow(
                            format!(
                                "▲ [ai] {provider} attempt {attempt}/{max_attempts} failed: {err}"
                            )
                            .as_str()
                        )
                    );
                }
            }
        }
    }
    bail!("all AI providers in the fallback chain failed")
}

/// Print only when the response was buffered (pipe mode) — streaming already echoed it.
fn emit(answer: &str) {
    if !output::is_tty() {
        println!("{answer}");
    }
}

pub fn run(cmd: &AiCommand, cfg: &Config) -> Result<()> {
    match &cmd.mode {
        AiMode::Chat { prompt } => {
            emit(&query(cfg, None, &prompt.join(" "))?);
        }
        AiMode::Analyze { path } => {
            let content = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("cannot read '{path}': {e}"))?;
            let sys = "You are a code and systems analysis assistant. Analyze the file for \
                       security, performance, bugs, and maintainability. Be direct and terse.";
            let prompt = format!("File: {path}\n```\n{content}\n```");
            emit(&query(cfg, Some(sys), &prompt)?);
        }
        AiMode::Review { staged, branch } => {
            let mut git = Command::new("git");
            git.arg("diff");
            if *staged {
                git.arg("--staged");
            } else {
                git.arg(branch);
            }
            let out = git.output()?;
            let diff = String::from_utf8_lossy(&out.stdout);
            if diff.trim().is_empty() {
                output::info("no diff to review");
                return Ok(());
            }
            let sys = "Review this diff for security, zero-allocation efficiency, and \
                       architectural compliance. Lead with critical findings.";
            emit(&query(cfg, Some(sys), &diff)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_secret_env_indirection() {
        std::env::set_var("CASHELL_TEST_AI_KEY", "s3cret");
        assert_eq!(resolve_secret("env:CASHELL_TEST_AI_KEY"), "s3cret");
        std::env::remove_var("CASHELL_TEST_AI_KEY");
    }

    #[test]
    fn resolve_secret_missing_env_is_empty() {
        assert_eq!(resolve_secret("env:CASHELL_DEFINITELY_UNSET_XYZ"), "");
    }

    #[test]
    fn resolve_secret_plain_passthrough() {
        assert_eq!(resolve_secret("raw_key"), "raw_key");
    }

    #[test]
    fn empty_9router_model_raises_actionable_error() {
        // No network: the empty-model guard fires before any HTTP request.
        let cfg = AiConfig::default();
        let client = Client::builder().build().unwrap();
        let err = attempt_provider(&client, "9router", &cfg, None, "hi", false)
            .expect_err("empty 9router model must error");
        let msg = err.to_string();
        assert!(
            msg.contains("cashell config set ai.model"),
            "error must name the fix command, got: {msg}"
        );
    }
}
