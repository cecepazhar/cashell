//! Telegram Bot notification dispatcher.

use anyhow::bail;
use reqwest::Client;
use std::time::Duration;

use super::{NotificationEvent, Notifier, NotifyFuture};

/// Telegram notifier using the Bot API (`sendMessage`).
pub struct TelegramNotifier {
    pub bot_token: String,
    pub chat_id: String,
    pub client: Client,
    pub base_url: String,
}

impl TelegramNotifier {
    /// Create a new Telegram notifier pointing to the official Bot API.
    pub fn new(bot_token: impl Into<String>, chat_id: impl Into<String>) -> Self {
        Self::with_base_url(bot_token, chat_id, "https://api.telegram.org")
    }

    /// Create a Telegram notifier with a custom base URL (useful for testing or local bot API server).
    pub fn with_base_url(
        bot_token: impl Into<String>,
        chat_id: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            bot_token: bot_token.into(),
            chat_id: chat_id.into(),
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }
}

impl Notifier for TelegramNotifier {
    fn send<'a>(&'a self, event: &'a NotificationEvent) -> NotifyFuture<'a> {
        Box::pin(async move {
            if self.bot_token.trim().is_empty() || self.chat_id.trim().is_empty() {
                return Ok(());
            }

            let url = format!("{}/bot{}/sendMessage", self.base_url, self.bot_token);
            let text = event.format_markdown();

            let payload = serde_json::json!({
                "chat_id": self.chat_id,
                "text": text,
                "parse_mode": "Markdown",
            });

            let resp = self
                .client
                .post(&url)
                .json(&payload)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("telegram request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                bail!("telegram API returned HTTP {status}: {err_body}");
            }

            Ok(())
        })
    }
}
