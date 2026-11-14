//! Telegram HTTP Client for Bot API interactions.

use anyhow::{bail, Context, Result};
use reqwest::Client;
use std::time::Duration;

use super::types::{TelegramResponse, TelegramUpdate, TelegramUser};

/// Max message length allowed in Telegram output (T-03).
pub const MAX_TELEGRAM_MESSAGE_LEN: usize = 4000;

/// Telegram HTTP client.
#[derive(Clone)]
pub struct TelegramClient {
    pub bot_token: String,
    pub base_url: String,
    pub http: Client,
}

impl TelegramClient {
    /// Create client using the default Telegram API base URL.
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self::with_base_url(bot_token, "https://api.telegram.org")
    }

    /// Create client with custom base URL (for testing or local proxy).
    pub fn with_base_url(bot_token: impl Into<String>, base_url: impl Into<String>) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .unwrap_or_default();

        Self {
            bot_token: bot_token.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// Query the bot profile via `getMe`.
    pub async fn get_me(&self) -> Result<TelegramUser> {
        let url = format!("{}/bot{}/getMe", self.base_url, self.bot_token);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("failed to send getMe request")?;

        let status = resp.status();
        let body: TelegramResponse<TelegramUser> = resp
            .json()
            .await
            .context("failed to parse getMe response JSON")?;

        if !body.ok || body.result.is_none() {
            bail!(
                "getMe failed (HTTP {status}): {}",
                body.description.unwrap_or_else(|| "unknown error".into())
            );
        }

        Ok(body.result.unwrap())
    }

    /// Long poll updates via `getUpdates` (T-01).
    pub async fn get_updates(&self, offset: i64, timeout_secs: u64) -> Result<Vec<TelegramUpdate>> {
        let url = format!("{}/bot{}/getUpdates", self.base_url, self.bot_token);
        let payload = serde_json::json!({
            "offset": offset,
            "timeout": timeout_secs,
            "limit": 100,
        });

        let resp = self
            .http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to send getUpdates request")?;

        let status = resp.status();
        if status.as_u16() == 409 {
            let err_text = resp.text().await.unwrap_or_default();
            bail!("Conflict (HTTP 409): Another instance or webhook is active: {err_text}");
        }

        let body: TelegramResponse<Vec<TelegramUpdate>> = resp
            .json()
            .await
            .context("failed to parse getUpdates response JSON")?;

        if !body.ok {
            if body.error_code == Some(409) {
                bail!(
                    "Conflict (HTTP 409): {}",
                    body.description.unwrap_or_default()
                );
            }
            bail!(
                "getUpdates returned error: {}",
                body.description.unwrap_or_else(|| "unknown".into())
            );
        }

        Ok(body.result.unwrap_or_default())
    }

    /// Send a message to a chat, automatically capping length to <= 4000 chars (T-03).
    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<()> {
        let url = format!("{}/bot{}/sendMessage", self.base_url, self.bot_token);

        // Cap text output at <= 4000 characters
        let safe_text = truncate_message(text, MAX_TELEGRAM_MESSAGE_LEN);

        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": safe_text,
        });

        let resp = self
            .http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("failed to send sendMessage request")?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            bail!("sendMessage failed with HTTP {status}: {err_body}");
        }

        Ok(())
    }
}

/// Truncate a message string to `max_len`, appending a truncation indicator if shortened.
pub fn truncate_message(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        let suffix = "\n...[truncated]";
        let take_len = max_len.saturating_sub(suffix.len());
        // Handle utf-8 boundary safely
        let mut boundary = take_len;
        while boundary > 0 && !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}{suffix}", &text[..boundary])
    }
}
