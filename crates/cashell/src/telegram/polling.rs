//! Long polling worker task for Telegram Bot API (T-01).
//!
//! Features:
//! - Configurable polling timeout (default: 30s)
//! - Offset persistence in SQLite DB to prevent reprocessing old messages on restart
//! - Exponential backoff on transient network errors (1s -> 2s -> 4s -> max 30s)
//! - HTTP 409 Conflict detection for concurrent polling/webhook instances
//! - Allowlist enforcement and audit logging for unauthorized senders (T-02)
//! - Routed execution via [`TelegramRouter`] (T-03)

use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::auth::check_telegram_auth;
use super::client::TelegramClient;
use super::router::TelegramRouter;
use super::types::TelegramUser;
use crate::actions::DockerApi;
use crate::db::Database;

/// Worker for Telegram long polling.
pub struct TelegramPoller {
    pub client: TelegramClient,
    pub db: Arc<Mutex<Database>>,
    pub router: Arc<TelegramRouter>,
    pub cfg_chat_id: Option<String>,
    pub timeout_secs: u64,
    pub backoff_secs: AtomicU64,
}

impl TelegramPoller {
    /// Create a new TelegramPoller with default settings.
    pub fn new(
        bot_token: impl Into<String>,
        db: Arc<Mutex<Database>>,
        docker: Arc<dyn DockerApi + Send + Sync>,
        cfg_chat_id: Option<String>,
    ) -> Self {
        let client = TelegramClient::new(bot_token);
        let router = Arc::new(TelegramRouter::new(docker, db.clone()));
        Self {
            client,
            db,
            router,
            cfg_chat_id,
            timeout_secs: 30,
            backoff_secs: AtomicU64::new(1),
        }
    }

    /// Customize base URL (for local mocks / testing).
    pub fn with_base_url(
        bot_token: impl Into<String>,
        base_url: impl Into<String>,
        db: Arc<Mutex<Database>>,
        docker: Arc<dyn DockerApi + Send + Sync>,
        cfg_chat_id: Option<String>,
    ) -> Self {
        let client = TelegramClient::with_base_url(bot_token, base_url);
        let router = Arc::new(TelegramRouter::new(docker, db.clone()));
        Self {
            client,
            db,
            router,
            cfg_chat_id,
            timeout_secs: 30,
            backoff_secs: AtomicU64::new(1),
        }
    }

    /// Set polling timeout in seconds.
    pub fn with_timeout_secs(mut self, timeout: u64) -> Self {
        self.timeout_secs = timeout;
        self
    }

    /// Poll once for incoming updates and dispatch them.
    /// Returns the number of authorized messages processed.
    pub async fn poll_once(&self) -> Result<usize> {
        let offset = {
            let db = self.db.lock().unwrap();
            db.get_telegram_offset().unwrap_or(0)
        };

        let updates = match self.client.get_updates(offset, self.timeout_secs).await {
            Ok(u) => {
                // Success: reset backoff
                self.backoff_secs.store(1, Ordering::SeqCst);
                u
            }
            Err(e) => {
                let err_msg = e.to_string();
                if err_msg.contains("Conflict (HTTP 409)") {
                    eprintln!(
                        "⚠ [Telegram] Conflict (HTTP 409): Another instance or webhook is active. Pausing polling for 10s..."
                    );
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    return Err(e);
                }

                let current_backoff = self.backoff_secs.load(Ordering::SeqCst);
                eprintln!(
                    "⚠ [Telegram] getUpdates error: {e}. Backing off for {current_backoff}s..."
                );
                tokio::time::sleep(Duration::from_secs(current_backoff)).await;
                // Double backoff up to 30s
                let next_backoff = (current_backoff * 2).min(30);
                self.backoff_secs.store(next_backoff, Ordering::SeqCst);
                return Err(e);
            }
        };

        let mut processed_count = 0;
        let mut highest_offset = offset;

        for update in updates {
            // Keep track of the next offset to advance
            let next_id = update.update_id + 1;
            if next_id > highest_offset {
                highest_offset = next_id;
            }

            if let Some(msg) = update.message {
                let chat_id = msg.chat.id;
                let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(chat_id);
                let text_opt = msg.text.as_deref();

                // T-02: Check allowlist authorization
                let authorized = {
                    let db = self.db.lock().unwrap();
                    check_telegram_auth(&db, self.cfg_chat_id.as_deref(), user_id, text_opt)?
                };

                if !authorized {
                    // Unauthorized message is silently dropped. Audit log was recorded in check_telegram_auth.
                    continue;
                }

                if let Some(text) = text_opt {
                    let user = msg.from.unwrap_or(TelegramUser {
                        id: user_id,
                        is_bot: false,
                        first_name: "User".into(),
                        last_name: None,
                        username: None,
                    });

                    let reply = self.router.handle_command(text, &user).await;
                    if let Err(e) = self.client.send_message(chat_id, &reply).await {
                        eprintln!("⚠ [Telegram] Failed sending reply to chat {chat_id}: {e}");
                    }
                    processed_count += 1;
                }
            }
        }

        // Advance and persist offset in database (T-01)
        if highest_offset > offset {
            let db = self.db.lock().unwrap();
            let _ = db.set_telegram_offset(highest_offset);
        }

        Ok(processed_count)
    }

    /// Run the polling worker loop continuously.
    pub async fn run(&self) {
        println!(
            "✔ Telegram long polling worker started (timeout={}s)",
            self.timeout_secs
        );
        loop {
            let _ = self.poll_once().await;
        }
    }
}
