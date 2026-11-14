//! Telegram Authorization and Allowlist Checking (T-02).

use crate::db::Database;
use anyhow::Result;

/// Check if a Telegram user is authorized.
///
/// An authorized user is either:
/// 1. Present in `telegram_allowlist` in the database.
/// 2. Matches `cfg_chat_id` (e.g. configured `daemon.telegram_chat_id` or `uptime.telegram_chat_id`).
///
/// If unauthorized, logs an entry in `audit_log` (source: "telegram", action: "unauthorized_message", result: "rejected")
/// and returns `false`.
pub fn check_telegram_auth(
    db: &Database,
    cfg_chat_id: Option<&str>,
    user_id: i64,
    incoming_text: Option<&str>,
) -> Result<bool> {
    if db.is_telegram_user_allowed(user_id)? {
        return Ok(true);
    }

    if let Some(cfg_id) = cfg_chat_id {
        let trimmed = cfg_id.trim();
        if !trimmed.is_empty() && trimmed == user_id.to_string() {
            return Ok(true);
        }
    }

    // Unauthorized attempt: log to audit_log (F-03, T-02)
    let details = serde_json::json!({
        "user_id": user_id,
        "text": incoming_text,
    });

    let _ = db.log_audit(
        "telegram",
        &user_id.to_string(),
        "unauthorized_message",
        Some(&details.to_string()),
        Some("rejected"),
        Some("User not in allowlist"),
    );

    Ok(false)
}
