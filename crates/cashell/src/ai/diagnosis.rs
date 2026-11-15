//! AI Log Diagnosis and Auto-Remediation engine (AI-07).

use crate::actions::Action;
use crate::ai::tools::{system_tool_definitions, tool_call_to_action, AiResponse};
use crate::config::Config;
use anyhow::Result;
use reqwest::blocking::Client;

/// Structured diagnosis finding produced by the unified AI engine.
#[derive(Debug, Clone, PartialEq)]
pub struct LogDiagnosisResult {
    pub summary: String,
    pub root_cause: String,
    pub proposed_fix: String,
    pub proposed_action: Option<Action>,
}

/// Run diagnosis on a slice of log lines using the unified AI tool engine.
pub fn diagnose_logs(
    cfg: &Config,
    logs: &[(String, String, String)],
) -> Result<LogDiagnosisResult> {
    if logs.is_empty() {
        return Ok(LogDiagnosisResult {
            summary: "No log entries to analyze".to_string(),
            root_cause: "Log buffer is currently empty".to_string(),
            proposed_fix: "Check your application stream or refresh logs".to_string(),
            proposed_action: None,
        });
    }

    let mut log_text = String::new();
    for (lvl, src, msg) in logs.iter().rev().take(40) {
        log_text.push_str(&format!("[{lvl}] {src}: {msg}\n"));
    }

    let sys_prompt = "You are an autonomous DevOps expert diagnostics agent for CAShell. \
                      Analyze the provided system/container logs, detect root causes, and propose remediation. \
                      If a container restart, app redeploy, or container stop/start will fix the issue, \
                      invoke the corresponding tool call (restart_container, deploy_app, etc.). \
                      Always be precise and actionable.";

    let user_prompt = format!(
        "Please diagnose the following error logs and propose a solution:\n```\n{log_text}\n```"
    );

    let tools = system_tool_definitions();
    let client = Client::builder().build()?;

    let response =
        crate::ai::query_with_tools(&client, cfg, Some(sys_prompt), &user_prompt, &tools)?;

    match response {
        AiResponse::Text(explanation) => Ok(LogDiagnosisResult {
            summary: "Analysis complete".to_string(),
            root_cause: explanation,
            proposed_fix: "Manual review recommended".to_string(),
            proposed_action: None,
        }),
        AiResponse::ToolCalls(calls) => {
            let proposed_action = if let Some(first_call) = calls.first() {
                tool_call_to_action(first_call).ok()
            } else {
                None
            };

            let action_desc = if let Some(ref act) = proposed_action {
                format!("Proposed action: {} ({:?})", act.action_name(), act)
            } else {
                "No automated remediation action available".to_string()
            };

            Ok(LogDiagnosisResult {
                summary: "Automated remediation proposed".to_string(),
                root_cause: "Detected actionable failure pattern in service logs".to_string(),
                proposed_fix: action_desc,
                proposed_action,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnose_empty_logs() {
        let cfg = Config::default();
        let res = diagnose_logs(&cfg, &[]).unwrap();
        assert_eq!(res.summary, "No log entries to analyze");
        assert!(res.proposed_action.is_none());
    }

    #[test]
    fn test_diagnosis_result_structure() {
        let res = LogDiagnosisResult {
            summary: "CrashLoopBackoff detected".to_string(),
            root_cause: "Container web exited with code 137 (OOM)".to_string(),
            proposed_fix: "Restart container web with more memory".to_string(),
            proposed_action: Some(Action::RestartContainer {
                name: "web".to_string(),
            }),
        };

        assert_eq!(res.summary, "CrashLoopBackoff detected");
        assert!(res.proposed_action.is_some());
        assert_eq!(
            res.proposed_action.as_ref().unwrap().action_name(),
            "restart_container"
        );
        assert_eq!(
            res.proposed_action.as_ref().unwrap().kind(),
            crate::actions::ActionKind::Write
        );
    }
}
