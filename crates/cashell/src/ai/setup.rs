//! Interactive and scripted setup wizard for AI providers (`cashell ai setup`) (AI-01).

use crate::ai::provider::{test_connection, ProviderKind};
use crate::config::{AiProviderConfig, Config};
use crate::output;
use anyhow::{bail, Result};
use reqwest::blocking::Client;
use std::io::{self, BufRead, Write};
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct SetupOptions {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub skip_test: bool,
    pub set_default: bool,
    pub non_interactive: bool,
}

fn prompt_line(prompt: &str, default: Option<&str>) -> Result<String> {
    let mut stdout = io::stdout().lock();
    if let Some(def) = default {
        write!(stdout, "{prompt} [{def}]: ")?;
    } else {
        write!(stdout, "{prompt}: ")?;
    }
    stdout.flush()?;

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        if let Some(def) = default {
            Ok(def.to_string())
        } else {
            Ok(String::new())
        }
    } else {
        Ok(trimmed.to_string())
    }
}

/// Run setup wizard (either scripted or interactive).
pub fn run_setup(opts: &SetupOptions, cfg_path: Option<&std::path::Path>) -> Result<()> {
    let mut cfg = Config::load(cfg_path)?;

    let name = match &opts.name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            if opts.non_interactive {
                bail!("--name is required in non-interactive mode");
            }
            prompt_line(
                "Provider identifier (e.g. openai, anthropic, ollama, deepseek)",
                Some("openai"),
            )?
        }
    };

    let kind_str = match &opts.kind {
        Some(k) if !k.trim().is_empty() => k.trim().to_lowercase(),
        _ => {
            if opts.non_interactive {
                "openai".to_string()
            } else {
                println!("\nSupported provider kinds:");
                println!("  1) openai   (OpenAI, DeepSeek, Groq, OpenRouter)");
                println!("  2) anthropic(Claude)");
                println!("  3) ollama   (Local models)");
                println!("  4) generic  (Custom OpenAI-compatible gateway)");
                prompt_line(
                    "Provider kind (openai, anthropic, ollama, generic)",
                    Some("openai"),
                )?
            }
        }
    };

    let kind = ProviderKind::from_str_loose(&kind_str);

    let default_url = match kind {
        ProviderKind::Anthropic => "https://api.anthropic.com/v1",
        ProviderKind::Ollama => "http://localhost:11434",
        _ => "https://api.openai.com/v1",
    };

    let base_url = match &opts.base_url {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => {
            if opts.non_interactive {
                default_url.to_string()
            } else {
                prompt_line("Base URL", Some(default_url))?
            }
        }
    };

    let default_model = match kind {
        ProviderKind::Anthropic => "claude-3-5-sonnet-20241022",
        ProviderKind::Ollama => "llama3",
        _ => "gpt-4o-mini",
    };

    let model = match &opts.model {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => {
            if opts.non_interactive {
                default_model.to_string()
            } else {
                prompt_line("Default model name", Some(default_model))?
            }
        }
    };

    let api_key = match &opts.api_key {
        Some(k) => k.trim().to_string(),
        _ => {
            if opts.non_interactive {
                String::new()
            } else {
                prompt_line("API key (or env:VAR_NAME, press enter for none)", None)?
            }
        }
    };

    let provider_cfg = AiProviderConfig {
        kind: kind.as_str().to_string(),
        base_url: base_url.clone(),
        api_key: api_key.clone(),
        default_model: model.clone(),
    };

    // Connection test
    if !opts.skip_test {
        if output::is_tty() && !opts.non_interactive {
            println!("\n▲ Testing connection to '{name}' ({kind_str}) at {base_url}...");
        }
        let client = Client::builder().build()?;
        let test_timeout = Duration::from_secs(10);
        match test_connection(&client, &name, &provider_cfg, test_timeout) {
            Ok(()) => {
                if output::is_tty() {
                    output::success(&format!("Connection to '{name}' verified successfully!"));
                }
            }
            Err(e) => {
                if opts.non_interactive {
                    bail!("Connection test failed for '{name}': {e}");
                } else {
                    eprintln!("✗ Connection test failed: {e}");
                    let proceed = prompt_line(
                        "Do you still want to save this configuration? (y/N)",
                        Some("N"),
                    )?;
                    if !proceed.eq_ignore_ascii_case("y") && !proceed.eq_ignore_ascii_case("yes") {
                        bail!("AI setup aborted by user due to failed connection test");
                    }
                }
            }
        }
    }

    // Save into config
    cfg.ai.providers.insert(name.clone(), provider_cfg);
    if opts.set_default || cfg.ai.default_provider.is_empty() {
        cfg.ai.default_provider = name.clone();
    }
    if !cfg.ai.fallback_chain.contains(&name) {
        cfg.ai.fallback_chain.push(name.clone());
    }

    let save_path = match cfg_path {
        Some(p) => p.to_path_buf(),
        None => Config::default_path(),
    };
    cfg.save(&save_path)?;

    output::success(&format!(
        "AI provider '{name}' ({kind_str}) configured successfully in {}",
        save_path.display()
    ));

    Ok(())
}
