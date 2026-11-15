use super::*;
use crate::ai::provider::{detect_provider_kind, resolve_secret, ProviderKind};
use crate::ai::rate_limit::AiBudgetGuard;
use crate::ai::redact::redact_secrets;
use crate::ai::setup::{run_setup, SetupOptions};
use crate::ai::tools::{
    format_anthropic_tools, format_ollama_tools, format_openai_tools, parse_anthropic_response,
    parse_ollama_response, parse_openai_response, AiResponse, ToolDefinition,
};
use crate::config::{AiConfig, AiProviderConfig, Config};

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
fn unknown_provider_raises_actionable_error() {
    let cfg = AiConfig::default();
    let client = Client::builder().build().unwrap();
    let err = attempt_provider(&client, "unconfigured", &cfg, None, "hi", false)
        .expect_err("unconfigured provider must error");
    let msg = err.to_string();
    assert!(
        msg.contains("not configured"),
        "error must name the missing provider, got: {msg}"
    );
}

#[test]
fn test_ai01_generic_provider_and_setup_wizard() {
    let tmp_name = format!(
        "cashell_test_ai_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(tmp_name);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg_path = dir.join("cashell.toml");

    let opts = SetupOptions {
        name: Some("deepseek".to_string()),
        kind: Some("openai".to_string()),
        base_url: Some("https://api.deepseek.com/v1".to_string()),
        model: Some("deepseek-chat".to_string()),
        api_key: Some("sk-test-key-1234567890".to_string()),
        skip_test: true,
        set_default: true,
        non_interactive: true,
    };

    run_setup(&opts, Some(&cfg_path)).expect("setup wizard should succeed");

    let loaded = Config::load(Some(&cfg_path)).expect("config should load");
    assert_eq!(loaded.ai.default_provider, "deepseek");
    assert!(loaded.ai.fallback_chain.contains(&"deepseek".to_string()));

    let p = loaded
        .ai
        .providers
        .get("deepseek")
        .expect("provider should exist");
    assert_eq!(p.kind, "openai");
    assert_eq!(p.base_url, "https://api.deepseek.com/v1");
    assert_eq!(p.default_model, "deepseek-chat");
    assert_eq!(p.api_key, "sk-test-key-1234567890");

    assert_eq!(detect_provider_kind(p, "deepseek"), ProviderKind::OpenAi);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_ai01_detect_provider_kinds() {
    let anthropic_p = AiProviderConfig {
        kind: String::new(),
        base_url: "https://api.anthropic.com/v1".into(),
        api_key: "key".into(),
        default_model: "claude".into(),
    };
    assert_eq!(
        detect_provider_kind(&anthropic_p, "claude"),
        ProviderKind::Anthropic
    );

    let ollama_p = AiProviderConfig {
        kind: String::new(),
        base_url: "http://localhost:11434".into(),
        api_key: "".into(),
        default_model: "llama3".into(),
    };
    assert_eq!(
        detect_provider_kind(&ollama_p, "local"),
        ProviderKind::Ollama
    );

    let explicit_p = AiProviderConfig {
        kind: "generic".into(),
        base_url: "http://gateway.local/v1".into(),
        api_key: "custom".into(),
        default_model: "mod".into(),
    };
    assert_eq!(
        detect_provider_kind(&explicit_p, "gw"),
        ProviderKind::Generic
    );
}

#[test]
fn test_ai02_tool_calling_openai_anthropic_ollama() {
    let tool = ToolDefinition::new(
        "restart_service",
        "Restart a service",
        serde_json::json!({
            "type": "object",
            "properties": {
                "service": { "type": "string" }
            },
            "required": ["service"]
        }),
    );
    let tools = vec![tool];

    // 1. OpenAI format
    let openai_tools = format_openai_tools(&tools);
    assert_eq!(openai_tools[0]["type"], "function");
    assert_eq!(openai_tools[0]["function"]["name"], "restart_service");

    let openai_resp = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "restart_service",
                        "arguments": "{\"service\": \"nginx\"}"
                    }
                }]
            }
        }]
    });
    let parsed_openai = parse_openai_response(&openai_resp).unwrap();
    match parsed_openai {
        AiResponse::ToolCalls(calls) => {
            assert_eq!(calls[0].name, "restart_service");
            assert_eq!(calls[0].arguments["service"], "nginx");
        }
        _ => panic!("expected tool calls"),
    }

    // 2. Anthropic format
    let anthropic_tools = format_anthropic_tools(&tools);
    assert_eq!(anthropic_tools[0]["name"], "restart_service");

    let anthropic_resp = serde_json::json!({
        "content": [{
            "type": "tool_use",
            "id": "tool_1",
            "name": "restart_service",
            "input": { "service": "nginx" }
        }]
    });
    let parsed_anthropic = parse_anthropic_response(&anthropic_resp).unwrap();
    match parsed_anthropic {
        AiResponse::ToolCalls(calls) => {
            assert_eq!(calls[0].name, "restart_service");
            assert_eq!(calls[0].arguments["service"], "nginx");
        }
        _ => panic!("expected tool calls"),
    }

    // 3. Ollama format
    let ollama_tools = format_ollama_tools(&tools);
    assert_eq!(ollama_tools[0]["type"], "function");

    let ollama_resp = serde_json::json!({
        "message": {
            "role": "assistant",
            "tool_calls": [{
                "function": {
                    "name": "restart_service",
                    "arguments": { "service": "nginx" }
                }
            }]
        }
    });
    let parsed_ollama = parse_ollama_response(&ollama_resp).unwrap();
    match parsed_ollama {
        AiResponse::ToolCalls(calls) => {
            assert_eq!(calls[0].name, "restart_service");
            assert_eq!(calls[0].arguments["service"], "nginx");
        }
        _ => panic!("expected tool calls"),
    }
}

#[test]
fn test_ai04_secret_redaction_before_provider_send() {
    let dirty_prompt = "Checking failure for sk-live-12345678901234567890 \
                        with token 987654321:ABCdefGHIjklMNOpqrSTUvwxYZ123456789 \
                        and Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.token123 \
                        on server postgres://user:SuperSecretPassword123@localhost:5432/cashell \
                        and license key CASH99-TEST-SECRET-ABCD";

    let extra = ["CASH99-TEST-SECRET-ABCD"];
    let clean = redact_secrets(dirty_prompt, &extra);

    assert!(!clean.contains("sk-live-12345678901234567890"));
    assert!(!clean.contains("987654321:ABCdefGHIjklMNOpqrSTUvwxYZ123456789"));
    assert!(!clean.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.token123"));
    assert!(!clean.contains("SuperSecretPassword123"));
    assert!(!clean.contains("CASH99-TEST-SECRET-ABCD"));

    assert!(clean.contains("[REDACTED_API_KEY]"));
    assert!(clean.contains("[REDACTED_TELEGRAM_TOKEN]"));
    assert!(clean.contains("[REDACTED_TOKEN]"));
    assert!(clean.contains("postgres://user:[REDACTED]@localhost:5432/cashell"));
    assert!(clean.contains("[REDACTED]"));
}

#[test]
fn test_ai05_token_and_request_rate_limits() {
    let guard = AiBudgetGuard::new();

    // 1. Test request per minute limit (RPM)
    let rpm_limit = 3;
    let daily_limit = 0;
    assert!(guard.check_and_reserve(daily_limit, rpm_limit, 10).is_ok());
    assert!(guard.check_and_reserve(daily_limit, rpm_limit, 10).is_ok());
    assert!(guard.check_and_reserve(daily_limit, rpm_limit, 10).is_ok());

    let rpm_err = guard
        .check_and_reserve(daily_limit, rpm_limit, 10)
        .expect_err("4th request in 1 minute must exceed limit");
    let msg = rpm_err.to_string();
    assert!(
        msg.contains("AI rate limit exceeded"),
        "error must indicate rate limit exceeded, got: {msg}"
    );

    // 2. Test daily token budget limit
    let token_guard = AiBudgetGuard::new();
    let rpm_limit = 0;
    let daily_limit = 100;

    assert!(token_guard
        .check_and_reserve(daily_limit, rpm_limit, 40)
        .is_ok());
    assert!(token_guard
        .check_and_reserve(daily_limit, rpm_limit, 50)
        .is_ok());

    // 40 + 50 + 20 = 110 > 100
    let token_err = token_guard
        .check_and_reserve(daily_limit, rpm_limit, 20)
        .expect_err("total 110 must exceed daily limit of 100");
    let msg2 = token_err.to_string();
    assert!(
        msg2.contains("AI daily token budget exceeded"),
        "error must indicate token budget exceeded, got: {msg2}"
    );
}
