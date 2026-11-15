//! Telegram Bot integration module for CAShell (T-01 s/d T-07).
//!
//! Provides long polling worker (`getUpdates`), allowlist & role-based access control,
//! deterministic command routing via typed actions (`actions::execute`),
//! mutating write action inline confirmation dialogs (T-04),
//! per-user rate limiting & audit logging (T-06),
//! multi-user role management with Pro gating (T-07),
//! and setup wizard CLI (T-02).

pub mod auth;
pub mod client;
pub mod confirm;
pub mod polling;
pub mod rate_limit;
pub mod router;
pub mod types;

#[cfg(test)]
mod tests;

pub use auth::{check_telegram_auth, get_user_role, TelegramRole};
pub use client::{truncate_message, TelegramClient, MAX_TELEGRAM_MESSAGE_LEN};
pub use confirm::{ConfirmStore, PendingAction, CONFIRMATION_TIMEOUT};
pub use polling::TelegramPoller;
pub use rate_limit::RateLimiter;
pub use router::{RouterOutput, TelegramRouter};
pub use types::{
    InlineKeyboardButton, InlineKeyboardMarkup, TelegramCallbackQuery, TelegramChat,
    TelegramMessage, TelegramUpdate, TelegramUser,
};
