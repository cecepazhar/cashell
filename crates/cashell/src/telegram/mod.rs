//! Telegram Bot integration module for CAShell (T-01, T-02, T-03).
//!
//! Provides long polling worker (`getUpdates`), allowlist access control,
//! deterministic command routing via typed actions (`actions::execute`),
//! and setup wizard CLI.

pub mod auth;
pub mod client;
pub mod polling;
pub mod router;
pub mod types;

#[cfg(test)]
mod tests;

pub use auth::check_telegram_auth;
pub use client::{truncate_message, TelegramClient, MAX_TELEGRAM_MESSAGE_LEN};
pub use polling::TelegramPoller;
pub use router::TelegramRouter;
pub use types::{TelegramChat, TelegramMessage, TelegramUpdate, TelegramUser};
