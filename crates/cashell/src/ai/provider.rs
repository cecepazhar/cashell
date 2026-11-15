//! Provider abstractions, connection test, and execution for OpenAI, Anthropic, Ollama, Generic (AI-01, AI-02).

use crate::ai::tools::{
    format_anthropic_tools, format_ollama_tools, format_openai_tools, parse_anthropic_response,
    parse_ollama_response, parse_openai_response, AiResponse, ToolDefinition,
};
use crate::config::AiProviderConfig;
use anyhow::{bail, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::time::Duration;

/// Normalized provider kinds supported by CAShell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Ollama,
    Generic,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
            Self::Generic => "generic",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "anthropic" | "claude" => Self::Anthropic,
            "ollama" | "local" => Self::Ollama,
            "generic" => Self::Generic,
            _ => Self::OpenAi,
        }
    }
}

/// Detect the [`ProviderKind`] from the explicit `kind` field or fall back to URL/name heuristics.
pub fn detect_provider_kind(p: &AiProviderConfig, name: &str) -> ProviderKind {
    if !p.kind.is_empty() {
        return ProviderKind::from_str_loose(&p.kind);
    }
    if p.base_url.contains("api.anthropic.com") {
        ProviderKind::Anthropic
    } else if name == "local" || p.base_url.contains("11434") {
        ProviderKind::Ollama
    } else {
        ProviderKind::OpenAi
    }
}

/// Resolve `env:NAME` indirection or return the raw value.
pub fn resolve_secret(val: &str) -> String {
    match val.strip_prefix("env:") {
        Some(var) => std::env::var(var).unwrap_or_default(),
        None => val.to_string(),
    }
}

/// Fail fast on rate limits and server errors so the caller fails over.
pub fn check_status(resp: &reqwest::blocking::Response, name: &str, key: &str) -> Result<()> {
    let status = resp.status();
    if status.as_u16() == 429 {
        bail!("{name}: HTTP 429 rate limited");
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        if key.is_empty() {
            bail!(
                "{name}: HTTP {status} — API key is empty. \
                 Set it with: cashell ai setup --name {name} \
                 or cashell config set ai.api_key <key>"
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

/// Test connection to an AI provider by issuing a lightweight probe.
pub fn test_connection(
    client: &Client,
    name: &str,
    p: &AiProviderConfig,
    timeout: Duration,
) -> Result<()> {
    let kind = detect_provider_kind(p, name);
    let key = resolve_secret(&p.api_key);
    let model = if p.default_model.is_empty() {
        match kind {
            ProviderKind::Anthropic => "claude-3-5-sonnet-20241022",
            ProviderKind::Ollama => "llama3",
            _ => "gpt-4o-mini",
        }
    } else {
        &p.default_model
    };

    match kind {
        ProviderKind::Anthropic => {
            let url = format!("{}/messages", p.base_url.trim_end_matches('/'));
            let payload = serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "ping" }]
            });
            let resp = client
                .post(&url)
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01")
                .timeout(timeout)
                .json(&payload)
                .send()?;
            check_status(&resp, name, &key)?;
            Ok(())
        }
        ProviderKind::Ollama => {
            // Check Ollama tags first
            let base = p.base_url.trim_end_matches('/');
            let tags_url = format!("{base}/api/tags");
            if let Ok(resp) = client.get(&tags_url).timeout(timeout).send() {
                if resp.status().is_success() {
                    return Ok(());
                }
            }
            // Fallback to minimal generate probe
            let gen_url = format!("{base}/api/generate");
            let payload = serde_json::json!({
                "model": model,
                "prompt": "ping",
                "stream": false
            });
            let resp = client
                .post(&gen_url)
                .timeout(timeout)
                .json(&payload)
                .send()?;
            check_status(&resp, name, "")?;
            Ok(())
        }
        ProviderKind::OpenAi | ProviderKind::Generic => {
            let url = format!("{}/chat/completions", p.base_url.trim_end_matches('/'));
            let payload = serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "ping" }]
            });
            let mut req = client.post(&url).timeout(timeout).json(&payload);
            if !key.is_empty() {
                req = req.header("Authorization", format!("Bearer {key}"));
            }
            let resp = req.send()?;
            check_status(&resp, name, &key)?;
            Ok(())
        }
    }
}

/// Call provider for plain text completion (streaming on TTY if enabled).
pub fn call_provider(
    client: &Client,
    name: &str,
    p: &AiProviderConfig,
    system_prompt: Option<&str>,
    user_prompt: &str,
    stream: bool,
    timeout: Duration,
) -> Result<String> {
    let kind = detect_provider_kind(p, name);
    let key = resolve_secret(&p.api_key);
    let model = if p.default_model.is_empty() {
        "auto"
    } else {
        &p.default_model
    };

    match kind {
        ProviderKind::Anthropic => call_anthropic(
            client,
            &p.base_url,
            &key,
            model,
            system_prompt,
            user_prompt,
            timeout,
        ),
        ProviderKind::Ollama => call_ollama(
            client,
            &p.base_url,
            model,
            system_prompt,
            user_prompt,
            timeout,
        ),
        ProviderKind::OpenAi | ProviderKind::Generic => call_openai(
            client,
            name,
            &p.base_url,
            &key,
            model,
            system_prompt,
            user_prompt,
            stream,
            timeout,
        ),
    }
}

/// Call provider with tools enabled and return [`AiResponse`].
pub fn call_provider_with_tools(
    client: &Client,
    name: &str,
    p: &AiProviderConfig,
    system_prompt: Option<&str>,
    user_prompt: &str,
    tools: &[ToolDefinition],
    timeout: Duration,
) -> Result<AiResponse> {
    let kind = detect_provider_kind(p, name);
    let key = resolve_secret(&p.api_key);
    let model = if p.default_model.is_empty() {
        "auto"
    } else {
        &p.default_model
    };

    match kind {
        ProviderKind::Anthropic => {
            let url = format!("{}/messages", p.base_url.trim_end_matches('/'));
            let mut payload = serde_json::json!({
                "model": model,
                "max_tokens": 4096,
                "messages": [{ "role": "user", "content": user_prompt }],
                "tools": format_anthropic_tools(tools),
            });
            if let Some(sys) = system_prompt {
                payload["system"] = serde_json::json!(sys);
            }
            let resp = client
                .post(&url)
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01")
                .timeout(timeout)
                .json(&payload)
                .send()?;
            check_status(&resp, name, &key)?;
            let body: serde_json::Value = resp.json()?;
            parse_anthropic_response(&body)
        }
        ProviderKind::Ollama => {
            let url = format!("{}/api/chat", p.base_url.trim_end_matches('/'));
            let mut messages = Vec::new();
            if let Some(sys) = system_prompt {
                messages.push(serde_json::json!({ "role": "system", "content": sys }));
            }
            messages.push(serde_json::json!({ "role": "user", "content": user_prompt }));

            let payload = serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": false,
                "tools": format_ollama_tools(tools),
            });
            let resp = client.post(&url).timeout(timeout).json(&payload).send()?;
            check_status(&resp, name, "")?;
            let body: serde_json::Value = resp.json()?;
            parse_ollama_response(&body)
        }
        ProviderKind::OpenAi | ProviderKind::Generic => {
            let url = format!("{}/chat/completions", p.base_url.trim_end_matches('/'));
            let mut messages = Vec::new();
            if let Some(sys) = system_prompt {
                messages.push(serde_json::json!({ "role": "system", "content": sys }));
            }
            messages.push(serde_json::json!({ "role": "user", "content": user_prompt }));

            let payload = serde_json::json!({
                "model": model,
                "messages": messages,
                "tools": format_openai_tools(tools),
            });
            let mut req = client.post(&url).timeout(timeout).json(&payload);
            if !key.is_empty() {
                req = req.header("Authorization", format!("Bearer {key}"));
            }
            let resp = req.send()?;
            check_status(&resp, name, &key)?;
            let body: serde_json::Value = resp.json()?;
            parse_openai_response(&body)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn call_openai(
    client: &Client,
    name: &str,
    base_url: &str,
    key: &str,
    model: &str,
    system_prompt: Option<&str>,
    user_prompt: &str,
    stream: bool,
    timeout: Duration,
) -> Result<String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut messages = Vec::new();
    if let Some(sys) = system_prompt {
        messages.push(serde_json::json!({ "role": "system", "content": sys }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": user_prompt }));

    let payload = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    });

    let mut req = client.post(&url).timeout(timeout).json(&payload);
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let resp = req.send()?;
    check_status(&resp, name, key)?;

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
