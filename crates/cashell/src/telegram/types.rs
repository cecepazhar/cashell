//! Telegram Bot API data models.

use serde::{Deserialize, Serialize};

/// Telegram user representation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramUser {
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}

/// Telegram chat representation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(default)]
    pub r#type: String,
    pub title: Option<String>,
    pub username: Option<String>,
}

/// Telegram message representation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub from: Option<TelegramUser>,
    pub chat: TelegramChat,
    #[serde(default)]
    pub date: i64,
    pub text: Option<String>,
}

/// Telegram update object returned by `getUpdates`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

/// Generic response envelope from Telegram Bot API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub error_code: Option<i32>,
    pub description: Option<String>,
}
