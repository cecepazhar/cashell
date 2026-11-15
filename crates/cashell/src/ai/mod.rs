//! Multi-vendor AI proxy client & subcommands — `cashell ai` (REQ-P2-02, AI-01..AI-05).
//!
//! Providers: OpenAI, Anthropic, Ollama, Generic OpenAI-compatible gateways.
//! Features:
//! - Connection test & setup wizard (`cashell ai setup`, AI-01)
//! - Tool calling abstraction across all providers (AI-02)
//! - Secret scrubbing before outgoing payloads (AI-04)
//! - Token and request budget rate limits (AI-05)
//! - SSE streaming on TTY, buffered plain output in pipe mode, cumulative 45s fallback chain budget.

pub mod diagnosis;
pub mod provider;
pub mod rate_limit;
pub mod redact;
pub mod setup;
pub mod tools;

#[cfg(test)]
mod tests;

use crate::cli::{AiCommand, AiMode};
use crate::config::{AiConfig, Config};
use crate::output;
use anyhow::{bail, Result};
use reqwest::blocking::Client;
use std::process::Command;
use std::time::{Duration, Instant};

pub use diagnosis::{diagnose_logs, LogDiagnosisResult};
pub use provider::ProviderKind;
pub use rate_limit::{estimate_tokens, global_budget_guard, AiBudgetGuard};
pub use redact::redact_secrets;
pub use tools::{
    system_tool_definitions, tool_call_to_action, AiResponse, ToolCall, ToolDefinition,
};

const TOTAL_BUDGET: Duration = Duration::from_secs(45);

/// Collect known secrets from application configuration to scrub from prompts.
fn collect_configured_secrets(cfg: &Config) -> Vec<String> {
    let mut secrets = Vec::new();
    if let Some(t) = &cfg.auth.token {
        if !t.is_empty() {
            secrets.push(t.clone());
        }
    }
    if !cfg.daemon.telegram_bot_token.is_empty() {
        secrets.push(cfg.daemon.telegram_bot_token.clone());
    }
    for p in cfg.ai.providers.values() {
        let key = provider::resolve_secret(&p.api_key);
        if !key.is_empty() {
            secrets.push(key);
        }
    }
    secrets
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
    let p = cfg
        .providers
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("provider '{name}' not configured"))?;

    let timeout = Duration::from_secs(cfg.timeout_seconds.clamp(5, 300));
    provider::call_provider(client, name, p, system_prompt, user_prompt, stream, timeout)
}

/// Run `user_prompt` through the configured fallback chain.
pub fn query(cfg: &Config, system_prompt: Option<&str>, user_prompt: &str) -> Result<String> {
    query_opts(cfg, system_prompt, user_prompt, output::is_tty())
}

/// Like [`query`] but never streams to stdout — used by `audit` so raw model
/// deltas don't interleave with the rendered report on a TTY.
pub fn query_buffered(
    cfg: &Config,
    system_prompt: Option<&str>,
    user_prompt: &str,
) -> Result<String> {
    query_opts(cfg, system_prompt, user_prompt, false)
}

/// Like [`query_buffered`] but reuses a caller-supplied [`Client`].
pub fn query_buffered_with(
    client: &Client,
    cfg: &Config,
    system_prompt: Option<&str>,
    user_prompt: &str,
) -> Result<String> {
    query_with_client(client, cfg, system_prompt, user_prompt, false)
}

fn query_opts(
    cfg: &Config,
    system_prompt: Option<&str>,
    user_prompt: &str,
    stream: bool,
) -> Result<String> {
    let client = Client::builder().build()?;
    query_with_client(&client, cfg, system_prompt, user_prompt, stream)
}

fn query_with_client(
    client: &Client,
    cfg: &Config,
    system_prompt: Option<&str>,
    user_prompt: &str,
    stream: bool,
) -> Result<String> {
    let ai = &cfg.ai;

    // AI-04: Redact secrets before preparing payload
    let known_secrets = collect_configured_secrets(cfg);
    let extra_refs: Vec<&str> = known_secrets.iter().map(|s| s.as_str()).collect();

    let redacted_user = redact_secrets(user_prompt, &extra_refs);
    let redacted_sys = system_prompt.map(|s| redact_secrets(s, &extra_refs));

    // AI-05: Rate limits and daily token budget enforcement BEFORE making any HTTP request
    let est_tokens = estimate_tokens(&redacted_user)
        + estimate_tokens(redacted_sys.as_deref().unwrap_or_default());
    global_budget_guard().check_and_reserve(
        ai.daily_token_limit,
        ai.requests_per_minute_limit,
        est_tokens,
    )?;

    let start = Instant::now();
    let max_attempts = ai.max_retries.clamp(0, 1) + 1;

    for provider in &ai.fallback_chain {
        for attempt in 1..=max_attempts {
            if start.elapsed() >= TOTAL_BUDGET {
                bail!("AI fallback chain exceeded the 45s wall-clock budget");
            }
            match attempt_provider(
                client,
                provider,
                ai,
                redacted_sys.as_deref(),
                &redacted_user,
                stream,
            ) {
                Ok(answer) => {
                    // Record completion tokens used
                    global_budget_guard().record_tokens(estimate_tokens(&answer));
                    return Ok(answer);
                }
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

/// Execute a query with tools enabled using the configured primary provider.
pub fn query_with_tools(
    client: &Client,
    cfg: &Config,
    system_prompt: Option<&str>,
    user_prompt: &str,
    tools: &[ToolDefinition],
) -> Result<AiResponse> {
    let ai = &cfg.ai;
    let provider_name = if !ai.default_provider.is_empty() {
        &ai.default_provider
    } else if let Some(first) = ai.fallback_chain.first() {
        first
    } else {
        bail!("no AI provider configured (use 'cashell ai setup' to configure one)");
    };

    let p = ai
        .providers
        .get(provider_name)
        .ok_or_else(|| anyhow::anyhow!("provider '{provider_name}' not configured"))?;

    // AI-04: Redact secrets
    let known_secrets = collect_configured_secrets(cfg);
    let extra_refs: Vec<&str> = known_secrets.iter().map(|s| s.as_str()).collect();
    let redacted_user = redact_secrets(user_prompt, &extra_refs);
    let redacted_sys = system_prompt.map(|s| redact_secrets(s, &extra_refs));

    // AI-05: Rate limits and daily token budget enforcement
    let est_tokens = estimate_tokens(&redacted_user)
        + estimate_tokens(redacted_sys.as_deref().unwrap_or_default());
    global_budget_guard().check_and_reserve(
        ai.daily_token_limit,
        ai.requests_per_minute_limit,
        est_tokens,
    )?;

    let timeout = Duration::from_secs(ai.timeout_seconds.clamp(5, 300));
    let resp = provider::call_provider_with_tools(
        client,
        provider_name,
        p,
        redacted_sys.as_deref(),
        &redacted_user,
        tools,
        timeout,
    )?;

    // Record usage
    if let AiResponse::Text(ref t) = resp {
        global_budget_guard().record_tokens(estimate_tokens(t));
    }

    Ok(resp)
}

/// Print only when the response was buffered (pipe mode) — streaming already echoed it.
fn emit(answer: &str) {
    if !output::is_tty() {
        println!("{answer}");
    }
}

pub fn run(cmd: &AiCommand, cfg: &Config) -> Result<()> {
    match &cmd.mode {
        AiMode::Setup {
            name,
            kind,
            base_url,
            model,
            api_key,
            skip_test,
            set_default,
            non_interactive,
        } => {
            let opts = setup::SetupOptions {
                name: name.clone(),
                kind: kind.clone(),
                base_url: base_url.clone(),
                model: model.clone(),
                api_key: api_key.clone(),
                skip_test: *skip_test,
                set_default: *set_default,
                non_interactive: *non_interactive,
            };
            setup::run_setup(&opts, None)?;
        }
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
