//! Deterministic Telegram Command Router (T-03).
//!
//! Maps Telegram slash commands directly to strongly-typed [`Action`]s,
//! executing them via [`crate::actions::execute`] and formatting results.

use std::sync::{Arc, Mutex};

use super::client::{truncate_message, MAX_TELEGRAM_MESSAGE_LEN};
use super::types::TelegramUser;
use crate::actions::{self, Action, ActionContext, DockerApi};
use crate::db::Database;

/// Router for deterministic Telegram slash commands.
#[derive(Clone)]
pub struct TelegramRouter {
    pub docker: Arc<dyn DockerApi + Send + Sync>,
    pub db: Arc<Mutex<Database>>,
}

impl TelegramRouter {
    pub fn new(docker: Arc<dyn DockerApi + Send + Sync>, db: Arc<Mutex<Database>>) -> Self {
        Self { docker, db }
    }

    /// Process an incoming message text from an authorized Telegram user and return response string.
    pub async fn handle_command(&self, text: &str, user: &TelegramUser) -> String {
        let trimmed = text.trim();
        if !trimmed.starts_with('/') {
            return "Send /help to view available commands.".to_string();
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let cmd = parts[0];
        // Strip bot mention if in group, e.g. "/status@CashellBot" -> "/status"
        let base_cmd = cmd.split('@').next().unwrap_or(cmd);

        let ctx = ActionContext::new("telegram", user.id.to_string())
            .with_docker(self.docker.clone())
            .with_db(self.db.clone());

        match base_cmd {
            "/help" => Self::help_text(),
            "/start" => "👋 Welcome to CAShell Telegram Bot!\nYou are authorized. Send /help to view available commands.".to_string(),
            "/status" => {
                Self::run_action(Action::GetStatus, &ctx).await
            }
            "/uptime" => {
                Self::run_action(Action::GetUptime, &ctx).await
            }
            "/logs" => {
                if parts.len() < 2 {
                    return "Usage: /logs <app> [lines≤200]".to_string();
                }
                let app = parts[1];
                let lines = if parts.len() >= 3 {
                    parts[2].parse::<usize>().unwrap_or(50).min(200)
                } else {
                    50
                };
                Self::run_action(Action::GetLogs { app: app.to_string(), lines }, &ctx).await
            }
            "/containers" => {
                Self::run_action(Action::ListContainers, &ctx).await
            }
            "/restart" => {
                if parts.len() < 2 {
                    return "Usage: /restart <container>".to_string();
                }
                let name = parts[1];
                Self::run_action(Action::RestartContainer { name: name.to_string() }, &ctx).await
            }
            "/deploy" => {
                if parts.len() < 2 {
                    return "Usage: /deploy <app>".to_string();
                }
                let app = parts[1];
                Self::run_action(Action::DeployApp { app: app.to_string() }, &ctx).await
            }
            "/analytics" => {
                let site = parts.get(1).copied().unwrap_or_default().to_string();
                let days = parts.get(2).and_then(|d| d.parse::<u32>().ok()).unwrap_or(7);
                Self::run_action(Action::GetAnalytics { site, days }, &ctx).await
            }
            _ => "Unknown command. Send /help for available commands.".to_string(),
        }
    }

    async fn run_action(action: Action, ctx: &ActionContext) -> String {
        let name = action.action_name();
        match actions::execute(action, ctx).await {
            Ok(res) => {
                let badge = if res.ok { "✔" } else { "✖" };
                let body = format!("{badge} [{name}]\n{}", res.output.trim());
                truncate_message(&body, MAX_TELEGRAM_MESSAGE_LEN)
            }
            Err(e) => {
                let body = format!("✖ [{name}] Error: {e}");
                truncate_message(&body, MAX_TELEGRAM_MESSAGE_LEN)
            }
        }
    }

    pub fn help_text() -> String {
        "CAShell Telegram Commands:
/status - System and daemon status
/uptime - Endpoint & fleet uptime summary
/logs <app> [n] - View container logs (n ≤ 200)
/containers - List docker containers
/restart <container> - Restart a target container
/deploy <app> - Deploy an application
/analytics [site] [days] - Sovereign analytics summary
/help - Show this help menu"
            .to_string()
    }
}
