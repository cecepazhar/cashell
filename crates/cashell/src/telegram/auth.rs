//! Telegram Authorization, Allowlist Checking, and Role-Based Access Control (T-02, T-07).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::Database;

/// Role assigned to a Telegram user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TelegramRole {
    Admin,
    Operator,
    Viewer,
}

impl std::str::FromStr for TelegramRole {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let role = match s.trim().to_ascii_lowercase().as_str() {
            "admin" => Self::Admin,
            "operator" => Self::Operator,
            "viewer" => Self::Viewer,
            _ => Self::Viewer,
        };
        Ok(role)
    }
}

impl TelegramRole {
    pub fn parse_role(s: &str) -> Self {
        s.parse().unwrap_or(Self::Viewer)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    /// True if this role can execute mutating (Write) actions.
    pub fn can_write(&self) -> bool {
        matches!(self, Self::Admin | Self::Operator)
    }

    /// True if this role can manage users (/users add, remove, role).
    pub fn can_manage_users(&self) -> bool {
        matches!(self, Self::Admin)
    }
}

/// Retrieve the role of a Telegram user if authorized.
pub fn get_user_role(
    db: &Database,
    cfg_chat_id: Option<&str>,
    user_id: i64,
) -> Result<Option<TelegramRole>> {
    if let Some(cfg_id) = cfg_chat_id {
        let trimmed = cfg_id.trim();
        if !trimmed.is_empty() && trimmed == user_id.to_string() {
            return Ok(Some(TelegramRole::Admin));
        }
    }

    if let Some(role_str) = db.get_telegram_user_role(user_id)? {
        return Ok(Some(TelegramRole::parse_role(&role_str)));
    }

    if db.is_telegram_user_allowed(user_id)? {
        return Ok(Some(TelegramRole::Admin));
    }

    Ok(None)
}

/// Check if a Telegram user is authorized.
///
/// If unauthorized, logs an entry in `audit_log` (source: "telegram", action: "unauthorized_message", result: "rejected")
/// and returns `false`.
pub fn check_telegram_auth(
    db: &Database,
    cfg_chat_id: Option<&str>,
    user_id: i64,
    incoming_text: Option<&str>,
) -> Result<bool> {
    if let Some(_role) = get_user_role(db, cfg_chat_id, user_id)? {
        return Ok(true);
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
