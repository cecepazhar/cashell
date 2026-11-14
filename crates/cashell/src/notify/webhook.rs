//! Webhook notification dispatchers for Discord and Slack.

use anyhow::bail;
use reqwest::Client;
use std::time::Duration;

use super::{NotificationEvent, Notifier, NotifyFuture};

/// Discord incoming webhook notifier.
pub struct DiscordNotifier {
    pub webhook_url: String,
    pub client: Client,
}

impl DiscordNotifier {
    pub fn new(webhook_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            webhook_url: webhook_url.into(),
            client,
        }
    }
}

impl Notifier for DiscordNotifier {
    fn send<'a>(&'a self, event: &'a NotificationEvent) -> NotifyFuture<'a> {
        Box::pin(async move {
            if self.webhook_url.trim().is_empty() {
                return Ok(());
            }

            let content = event.format_markdown();
            let payload = serde_json::json!({
                "content": content,
            });

            let resp = self
                .client
                .post(&self.webhook_url)
                .json(&payload)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("discord webhook failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                bail!("discord webhook returned HTTP {status}: {err_body}");
            }

            Ok(())
        })
    }
}

/// Slack incoming webhook notifier.
pub struct SlackNotifier {
    pub webhook_url: String,
    pub client: Client,
}

impl SlackNotifier {
    pub fn new(webhook_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            webhook_url: webhook_url.into(),
            client,
        }
    }
}

impl Notifier for SlackNotifier {
    fn send<'a>(&'a self, event: &'a NotificationEvent) -> NotifyFuture<'a> {
        Box::pin(async move {
            if self.webhook_url.trim().is_empty() {
                return Ok(());
            }

            let text = event.format_markdown();
            let payload = serde_json::json!({
                "text": text,
            });

            let resp = self
                .client
                .post(&self.webhook_url)
                .json(&payload)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("slack webhook failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                bail!("slack webhook returned HTTP {status}: {err_body}");
            }

            Ok(())
        })
    }
}
