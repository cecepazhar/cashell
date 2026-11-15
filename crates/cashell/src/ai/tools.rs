//! Multi-vendor Tool Calling abstraction for OpenAI, Anthropic, and Ollama (AI-02).

use crate::actions::Action;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Definition of a tool that can be invoked by the AI model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        desc: impl Into<String>,
        params: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: desc.into(),
            parameters: params,
        }
    }
}

/// A parsed tool call returned by the AI provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Unified response from an AI invocation with tools enabled.
#[derive(Debug, Clone, PartialEq)]
pub enum AiResponse {
    /// Plain text answer (no tool calls requested).
    Text(String),
    /// Model requested one or more tool calls.
    ToolCalls(Vec<ToolCall>),
}

/// Canonical system tools available to the AI across Telegram, Cockpit, and CLI.
pub fn system_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        // Read tools (Safe, automatically executed)
        ToolDefinition::new(
            "get_status",
            "Probe system and daemon status (CPU, RAM, load, online services)",
            serde_json::json!({
                "type": "object",
                "properties": {},
            }),
        ),
        ToolDefinition::new(
            "get_uptime",
            "Query uptime monitors status summary and incident history",
            serde_json::json!({
                "type": "object",
                "properties": {},
            }),
        ),
        ToolDefinition::new(
            "list_containers",
            "List all registered or running Docker containers",
            serde_json::json!({
                "type": "object",
                "properties": {},
            }),
        ),
        ToolDefinition::new(
            "get_logs",
            "Fetch recent log lines for an application or container",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Target container or app name" },
                    "lines": { "type": "integer", "description": "Number of lines (default 50, max 200)" }
                },
                "required": ["app"]
            }),
        ),
        ToolDefinition::new(
            "get_analytics",
            "Query sovereign web analytics traffic metrics",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "site": { "type": "string", "description": "Website domain or ID" },
                    "days": { "type": "integer", "description": "Number of past days (default 7)" }
                },
                "required": ["site"]
            }),
        ),
        // Write tools (Mutating, requires interactive user confirmation)
        ToolDefinition::new(
            "restart_container",
            "Restart a target container (requires user confirmation)",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the container to restart" }
                },
                "required": ["name"]
            }),
        ),
        ToolDefinition::new(
            "stop_container",
            "Stop a target container (requires user confirmation)",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the container to stop" }
                },
                "required": ["name"]
            }),
        ),
        ToolDefinition::new(
            "start_container",
            "Start a target container (requires user confirmation)",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the container to start" }
                },
                "required": ["name"]
            }),
        ),
        ToolDefinition::new(
            "deploy_app",
            "Trigger deployment for an application (requires user confirmation)",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "app": { "type": "string", "description": "Name of the app to deploy" }
                },
                "required": ["app"]
            }),
        ),
    ]
}

/// Convert a generic [`ToolCall`] into a strongly-typed [`Action`].
pub fn tool_call_to_action(call: &ToolCall) -> Result<Action> {
    match call.name.as_str() {
        "get_status" => Ok(Action::GetStatus),
        "get_uptime" => Ok(Action::GetUptime),
        "list_containers" => Ok(Action::ListContainers),
        "get_logs" => {
            let app = call
                .arguments
                .get("app")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing 'app' parameter for get_logs"))?
                .to_string();
            let lines = call
                .arguments
                .get("lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .min(200) as usize;
            Ok(Action::GetLogs { app, lines })
        }
        "get_analytics" => {
            let site = call
                .arguments
                .get("site")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let days = call
                .arguments
                .get("days")
                .and_then(|v| v.as_u64())
                .unwrap_or(7) as u32;
            Ok(Action::GetAnalytics { site, days })
        }
        "restart_container" => {
            let name = call
                .arguments
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing 'name' parameter for restart_container"))?
                .to_string();
            Ok(Action::RestartContainer { name })
        }
        "stop_container" => {
            let name = call
                .arguments
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing 'name' parameter for stop_container"))?
                .to_string();
            Ok(Action::StopContainer { name })
        }
        "start_container" => {
            let name = call
                .arguments
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing 'name' parameter for start_container"))?
                .to_string();
            Ok(Action::StartContainer { name })
        }
        "deploy_app" => {
            let app = call
                .arguments
                .get("app")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing 'app' parameter for deploy_app"))?
                .to_string();
            Ok(Action::DeployApp { app })
        }
        unknown => bail!("unsupported tool call: {unknown}"),
    }
}

/// Format tool definitions for OpenAI-compatible `/chat/completions` API.
pub fn format_openai_tools(tools: &[ToolDefinition]) -> serde_json::Value {
    let list: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect();
    serde_json::Value::Array(list)
}

/// Format tool definitions for Anthropic `/messages` API.
pub fn format_anthropic_tools(tools: &[ToolDefinition]) -> serde_json::Value {
    let list: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect();
    serde_json::Value::Array(list)
}

/// Format tool definitions for Ollama `/api/chat` API.
pub fn format_ollama_tools(tools: &[ToolDefinition]) -> serde_json::Value {
    // Ollama accepts OpenAI function calling structure in /api/chat tools field
    format_openai_tools(tools)
}

/// Parse OpenAI `/chat/completions` response body into unified [`AiResponse`].
pub fn parse_openai_response(body: &serde_json::Value) -> Result<AiResponse> {
    let choice = body
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| anyhow::anyhow!("OpenAI response missing 'choices[0]'"))?;

    let message = choice
        .get("message")
        .ok_or_else(|| anyhow::anyhow!("OpenAI choice missing 'message'"))?;

    if let Some(tool_calls) = message.get("tool_calls").and_then(|tc| tc.as_array()) {
        if !tool_calls.is_empty() {
            let mut calls = Vec::new();
            for (idx, tc) in tool_calls.iter().enumerate() {
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&format!("call_{idx}"))
                    .to_string();
                let func = tc
                    .get("function")
                    .ok_or_else(|| anyhow::anyhow!("OpenAI tool_call missing 'function'"))?;
                let name = func
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("OpenAI tool_call missing 'function.name'"))?
                    .to_string();

                let args_val = func.get("arguments").unwrap_or(&serde_json::Value::Null);
                let arguments = match args_val {
                    serde_json::Value::String(s) => {
                        serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({}))
                    }
                    val => val.clone(),
                };

                calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            return Ok(AiResponse::ToolCalls(calls));
        }
    }

    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();

    if content.is_empty() {
        bail!("empty response content from OpenAI");
    }
    Ok(AiResponse::Text(content))
}

/// Parse Anthropic `/messages` response body into unified [`AiResponse`].
pub fn parse_anthropic_response(body: &serde_json::Value) -> Result<AiResponse> {
    let content_arr = body
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow::anyhow!("Anthropic response missing 'content' array"))?;

    let mut tool_calls = Vec::new();
    let mut text_parts = Vec::new();

    for item in content_arr {
        let block_type = item
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        if block_type == "tool_use" {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Anthropic tool_use missing 'name'"))?
                .to_string();
            let arguments = item
                .get("input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        } else if block_type == "text" {
            if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                text_parts.push(t);
            }
        }
    }

    if !tool_calls.is_empty() {
        return Ok(AiResponse::ToolCalls(tool_calls));
    }

    let combined_text = text_parts.join("\n");
    if combined_text.is_empty() {
        bail!("empty content in Anthropic response");
    }
    Ok(AiResponse::Text(combined_text))
}

/// Parse Ollama `/api/chat` (or OpenAI-compatible) response into unified [`AiResponse`].
pub fn parse_ollama_response(body: &serde_json::Value) -> Result<AiResponse> {
    // 1. Check native Ollama /api/chat message format
    if let Some(msg) = body.get("message") {
        if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
            if !tool_calls.is_empty() {
                let mut calls = Vec::new();
                for (idx, tc) in tool_calls.iter().enumerate() {
                    let func = tc.get("function").unwrap_or(tc);
                    let name = func
                        .get("name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("Ollama tool call missing function name"))?
                        .to_string();
                    let args_val = func.get("arguments").unwrap_or(&serde_json::Value::Null);
                    let arguments = match args_val {
                        serde_json::Value::String(s) => {
                            serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({}))
                        }
                        val => val.clone(),
                    };
                    let id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&format!("ollama_{idx}"))
                        .to_string();
                    calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                return Ok(AiResponse::ToolCalls(calls));
            }
        }
        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                return Ok(AiResponse::Text(content.to_string()));
            }
        }
    }

    // 2. Check OpenAI-compatible format (Ollama /v1/chat/completions)
    if body.get("choices").is_some() {
        return parse_openai_response(body);
    }

    // 3. Fallback to /api/generate plain response
    if let Some(resp) = body.get("response").and_then(|r| r.as_str()) {
        if !resp.is_empty() {
            return Ok(AiResponse::Text(resp.to_string()));
        }
    }

    bail!("empty response from Ollama")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tool() -> ToolDefinition {
        ToolDefinition::new(
            "restart_service",
            "Restart a system service",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "service_name": { "type": "string" }
                },
                "required": ["service_name"]
            }),
        )
    }

    #[test]
    fn test_format_openai_tools() {
        let tools = vec![sample_tool()];
        let json = format_openai_tools(&tools);
        assert_eq!(json[0]["type"], "function");
        assert_eq!(json[0]["function"]["name"], "restart_service");
    }

    #[test]
    fn test_format_anthropic_tools() {
        let tools = vec![sample_tool()];
        let json = format_anthropic_tools(&tools);
        assert_eq!(json[0]["name"], "restart_service");
        assert!(json[0].get("input_schema").is_some());
    }

    #[test]
    fn test_parse_openai_tool_calls_recorded() {
        let recorded_json = serde_json::json!({
            "id": "chatcmpl-test-123",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_abc123",
                                "type": "function",
                                "function": {
                                    "name": "restart_service",
                                    "arguments": "{\"service_name\":\"caddy\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ]
        });

        let resp = parse_openai_response(&recorded_json).expect("should parse tool call");
        match resp {
            AiResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_abc123");
                assert_eq!(calls[0].name, "restart_service");
                assert_eq!(calls[0].arguments["service_name"], "caddy");
            }
            _ => panic!("expected ToolCalls"),
        }
    }

    #[test]
    fn test_parse_anthropic_tool_calls_recorded() {
        let recorded_json = serde_json::json!({
            "id": "msg_01X9Z",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "I will restart Caddy for you."
                },
                {
                    "type": "tool_use",
                    "id": "toolu_01A09q90tc1q09qlkj",
                    "name": "restart_service",
                    "input": {
                        "service_name": "caddy"
                    }
                }
            ],
            "stop_reason": "tool_use"
        });

        let resp = parse_anthropic_response(&recorded_json).expect("should parse tool call");
        match resp {
            AiResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "toolu_01A09q90tc1q09qlkj");
                assert_eq!(calls[0].name, "restart_service");
                assert_eq!(calls[0].arguments["service_name"], "caddy");
            }
            _ => panic!("expected ToolCalls"),
        }
    }

    #[test]
    fn test_parse_ollama_tool_calls_recorded() {
        let recorded_json = serde_json::json!({
            "model": "llama3.1",
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    {
                        "function": {
                            "name": "restart_service",
                            "arguments": {
                                "service_name": "caddy"
                            }
                        }
                    }
                ]
            },
            "done": true
        });

        let resp = parse_ollama_response(&recorded_json).expect("should parse tool call");
        match resp {
            AiResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "restart_service");
                assert_eq!(calls[0].arguments["service_name"], "caddy");
            }
            _ => panic!("expected ToolCalls"),
        }
    }

    #[test]
    fn test_parse_plain_text_responses() {
        let openai_text = serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": "Hello world!" }
            }]
        });
        assert_eq!(
            parse_openai_response(&openai_text).unwrap(),
            AiResponse::Text("Hello world!".into())
        );

        let anthropic_text = serde_json::json!({
            "content": [{ "type": "text", "text": "Anthropic says hi!" }]
        });
        assert_eq!(
            parse_anthropic_response(&anthropic_text).unwrap(),
            AiResponse::Text("Anthropic says hi!".into())
        );

        let ollama_text = serde_json::json!({
            "message": { "role": "assistant", "content": "Ollama says hi!" }
        });
        assert_eq!(
            parse_ollama_response(&ollama_text).unwrap(),
            AiResponse::Text("Ollama says hi!".into())
        );
    }
}
