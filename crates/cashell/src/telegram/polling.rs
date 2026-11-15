//! Long polling worker task for Telegram Bot API (T-01, T-04, T-06, T-07).
//!
//! Features:
//! - Configurable polling timeout (default: 30s)
//! - Offset persistence in SQLite DB to prevent reprocessing old messages on restart
//! - Exponential backoff on transient network errors (1s -> 2s -> 4s -> max 30s)
//! - HTTP 409 Conflict detection for concurrent polling/webhook instances
//! - Allowlist & role enforcement and audit logging (T-02, T-06, T-07)
//! - Inline confirmation for mutating write actions (T-04)
//! - Sliding window rate limiter per user (T-06)
//! - Routed execution via [`TelegramRouter`] (T-03)

use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::auth::{check_telegram_auth, get_user_role};
use super::client::TelegramClient;
use super::confirm::ConfirmStore;
use super::rate_limit::RateLimiter;
use super::router::{RouterOutput, TelegramRouter};
use super::types::TelegramUser;
use crate::actions::{ActionContext, DockerApi};
use crate::db::Database;

/// Worker for Telegram long polling.
pub struct TelegramPoller {
    pub client: TelegramClient,
    pub db: Arc<Mutex<Database>>,
    pub router: Arc<TelegramRouter>,
    pub confirm_store: Arc<ConfirmStore>,
    pub rate_limiter: Arc<RateLimiter>,
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
        let confirm_store = Arc::new(ConfirmStore::new());
        let rate_limiter = Arc::new(RateLimiter::default());
        Self {
            client,
            db,
            router,
            confirm_store,
            rate_limiter,
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
        let confirm_store = Arc::new(ConfirmStore::new());
        let rate_limiter = Arc::new(RateLimiter::default());
        Self {
            client,
            db,
            router,
            confirm_store,
            rate_limiter,
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
    /// Returns the number of authorized messages and callbacks processed.
    pub async fn poll_once(&self) -> Result<usize> {
        self.confirm_store.prune_expired();

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
                let next_backoff = (current_backoff * 2).min(30);
                self.backoff_secs.store(next_backoff, Ordering::SeqCst);
                return Err(e);
            }
        };

        let mut processed_count = 0;
        let mut highest_offset = offset;

        for update in updates {
            let next_id = update.update_id + 1;
            if next_id > highest_offset {
                highest_offset = next_id;
            }

            // ── 1. Process inline keyboard callback queries (T-04) ──────────
            if let Some(cq) = update.callback_query {
                let user_id = cq.from.id;
                let chat_id = cq.message.as_ref().map(|m| m.chat.id).unwrap_or(user_id);

                let is_auth = {
                    let db = self.db.lock().unwrap();
                    check_telegram_auth(&db, self.cfg_chat_id.as_deref(), user_id, None)?
                };

                if !is_auth {
                    continue;
                }

                // Acknowledge callback immediately
                let _ = self.client.answer_callback_query(&cq.id, None, false).await;

                if let Some(ref data) = cq.data {
                    if let Some(action_id) = data.strip_prefix("confirm:") {
                        let pending_opt = self.confirm_store.take(action_id);
                        match pending_opt {
                            None => {
                                let _ = self
                                    .client
                                    .send_message(
                                        chat_id,
                                        "⏱ Kedaluwarsa: Batas waktu 60 detik telah lewat atau aksi telah diproses.",
                                    )
                                    .await;
                                let db = self.db.lock().unwrap();
                                let _ = db.log_audit(
                                    "telegram",
                                    &user_id.to_string(),
                                    "confirm_action",
                                    Some(&format!("{{\"action_id\":{:?}}}", action_id)),
                                    Some("expired"),
                                    Some("Action expired or not found"),
                                );
                            }
                            Some(pending) if pending.is_expired() => {
                                let _ = self
                                    .client
                                    .send_message(
                                        chat_id,
                                        "⏱ Kedaluwarsa: Batas waktu 60 detik telah lewat. Aksi tidak dijalankan.",
                                    )
                                    .await;
                                let db = self.db.lock().unwrap();
                                let _ = db.log_audit(
                                    "telegram",
                                    &user_id.to_string(),
                                    pending.action.action_name(),
                                    Some(&format!("{{\"action_id\":{:?}}}", action_id)),
                                    Some("expired"),
                                    Some("Confirmation timeout after 60s"),
                                );
                            }
                            Some(pending) if cq.from.id != pending.user_id => {
                                let _ = self
                                    .client
                                    .send_message(
                                        chat_id,
                                        "⛔ Akses ditolak: Hanya pengirim asli yang dapat mengonfirmasi aksi ini.",
                                    )
                                    .await;
                                let db = self.db.lock().unwrap();
                                let _ = db.log_audit(
                                    "telegram",
                                    &user_id.to_string(),
                                    pending.action.action_name(),
                                    Some(&format!("{{\"action_id\":{:?}}}", action_id)),
                                    Some("unauthorized_callback"),
                                    Some("Sender is not original action owner"),
                                );
                            }
                            Some(pending) => {
                                // Confirmed! Execute action
                                let ctx = ActionContext::new("telegram", user_id.to_string())
                                    .with_docker(self.router.docker.clone())
                                    .with_db(self.db.clone());
                                let out = TelegramRouter::run_action(pending.action, &ctx).await;
                                let _ = self.client.send_message(chat_id, &out).await;
                                processed_count += 1;
                            }
                        }
                    } else if let Some(action_id) = data.strip_prefix("cancel:") {
                        let _ = self.confirm_store.take(action_id);
                        let _ = self
                            .client
                            .send_message(chat_id, "❌ Aksi dibatalkan oleh pengguna.")
                            .await;
                        let db = self.db.lock().unwrap();
                        let _ = db.log_audit(
                            "telegram",
                            &user_id.to_string(),
                            "cancel_action",
                            Some(&format!("{{\"action_id\":{:?}}}", action_id)),
                            Some("cancelled"),
                            None,
                        );
                        processed_count += 1;
                    } else if let Some(cmd) = data.strip_prefix("cmd:") {
                        // Shortcut from inline menu
                        let role_opt = {
                            let db = self.db.lock().unwrap();
                            get_user_role(&db, self.cfg_chat_id.as_deref(), user_id)?
                        };
                        if let Some(role) = role_opt {
                            let output = self.router.handle_command(cmd, &cq.from, role).await;
                            self.dispatch_output(chat_id, user_id, output).await;
                            processed_count += 1;
                        }
                    }
                }
                continue;
            }

            // ── 2. Process incoming text messages ───────────────────────────
            if let Some(msg) = update.message {
                let chat_id = msg.chat.id;
                let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(chat_id);
                let text_opt = msg.text.as_deref();

                let role_opt = {
                    let db = self.db.lock().unwrap();
                    let is_auth =
                        check_telegram_auth(&db, self.cfg_chat_id.as_deref(), user_id, text_opt)?;
                    if is_auth {
                        get_user_role(&db, self.cfg_chat_id.as_deref(), user_id)?
                    } else {
                        None
                    }
                };

                let role = match role_opt {
                    Some(r) => r,
                    None => continue, // Unauthorized: dropped and logged
                };

                // T-06: Rate limiting per user
                if let Err(wait_secs) = self.rate_limiter.check(user_id) {
                    let rate_msg = format!(
                        "⚠️ Terlalu banyak permintaan (rate limit). Silakan tunggu {wait_secs} detik."
                    );
                    let _ = self.client.send_message(chat_id, &rate_msg).await;
                    let db = self.db.lock().unwrap();
                    let _ = db.log_audit(
                        "telegram",
                        &user_id.to_string(),
                        "rate_limit_exceeded",
                        text_opt.map(|t| format!("{{\"text\":{:?}}}", t)).as_deref(),
                        Some("rate_limited"),
                        Some("User exceeded command rate limit"),
                    );
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

                    let output = self.router.handle_command(text, &user, role).await;
                    self.dispatch_output(chat_id, user_id, output).await;
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

    async fn dispatch_output(&self, chat_id: i64, user_id: i64, output: RouterOutput) {
        match output {
            RouterOutput::Text(reply) => {
                if let Err(e) = self.client.send_message(chat_id, &reply).await {
                    eprintln!("⚠ [Telegram] Failed sending reply to chat {chat_id}: {e}");
                }
            }
            RouterOutput::Menu { text, keyboard } => {
                if let Err(e) = self
                    .client
                    .send_message_with_keyboard(chat_id, &text, Some(&keyboard))
                    .await
                {
                    eprintln!("⚠ [Telegram] Failed sending menu to chat {chat_id}: {e}");
                }
            }
            RouterOutput::RequireConfirmation {
                action,
                description,
            } => {
                let action_name = action.action_name().to_string();
                let (action_id, keyboard) =
                    self.confirm_store
                        .create_pending(user_id, chat_id, action, &description);

                let prompt_text = format!(
                    "⚠️ *Konfirmasi Diperlukan*\n\nApakah Anda yakin ingin menjalankan aksi berikut?\n• *{description}*\n\n⏳ *Kedaluwarsa dalam 60 detik.*"
                );

                if let Err(e) = self
                    .client
                    .send_message_with_keyboard(chat_id, &prompt_text, Some(&keyboard))
                    .await
                {
                    eprintln!("⚠ [Telegram] Failed sending confirmation prompt to {chat_id}: {e}");
                }

                let db = self.db.lock().unwrap();
                let _ = db.log_audit(
                    "telegram",
                    &user_id.to_string(),
                    &action_name,
                    Some(&format!("{{\"action_id\":{:?}}}", action_id)),
                    Some("pending_confirmation"),
                    None,
                );
            }
        }
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
