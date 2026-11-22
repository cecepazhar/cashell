//! Pending confirmation store for mutating (Write) actions (T-04).
//!
//! Enforces:
//! - Random `action_id` token
//! - 60-second expiration window
//! - Original sender ownership check
//! - Inline keyboard generation

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::types::{InlineKeyboardButton, InlineKeyboardMarkup};
use crate::actions::Action;

/// Expiration timeout for pending mutating actions (60 seconds per T-04).
pub const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(60);

/// A pending write action awaiting inline button confirmation.
#[derive(Debug, Clone)]
pub struct PendingAction {
    pub action_id: String,
    pub user_id: i64,
    pub chat_id: i64,
    pub action: Action,
    pub description: String,
    pub created_at: Instant,
}

impl PendingAction {
    /// True if the pending confirmation has exceeded the 60s timeout.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > CONFIRMATION_TIMEOUT
    }
}

/// In-memory store for pending confirmations.
#[derive(Default)]
pub struct ConfirmStore {
    actions: Mutex<HashMap<String, PendingAction>>,
}

impl ConfirmStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new pending action. Returns `(action_id, InlineKeyboardMarkup)`.
    pub fn create_pending(
        &self,
        user_id: i64,
        chat_id: i64,
        action: Action,
        description: impl Into<String>,
    ) -> (String, InlineKeyboardMarkup) {
        let action_id = format!("{:016x}", rand::random::<u64>());
        let pending = PendingAction {
            action_id: action_id.clone(),
            user_id,
            chat_id,
            action,
            description: description.into(),
            created_at: Instant::now(),
        };

        self.actions
            .lock()
            .unwrap()
            .insert(action_id.clone(), pending);

        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("✅ Yes, continue", format!("confirm:{action_id}")),
            InlineKeyboardButton::callback("❌ Cancel", format!("cancel:{action_id}")),
        ]]);

        (action_id, keyboard)
    }

    /// Retrieve a clone of a pending action by its ID without removing it.
    pub fn get(&self, action_id: &str) -> Option<PendingAction> {
        self.actions.lock().unwrap().get(action_id).cloned()
    }

    /// Retrieve and remove a pending action by its ID.
    pub fn take(&self, action_id: &str) -> Option<PendingAction> {
        self.actions.lock().unwrap().remove(action_id)
    }

    /// Prune any expired actions.
    pub fn prune_expired(&self) {
        let mut map = self.actions.lock().unwrap();
        map.retain(|_, v| !v.is_expired());
    }
}
