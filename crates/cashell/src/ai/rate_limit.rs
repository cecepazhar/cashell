//! AI rate limits and daily token budget guards (AI-05).
//!
//! Enforces:
//! - Request rate limit (requests per minute sliding window)
//! - Daily token budget (tokens per day with UTC rollover)
//!
//! Rejects requests with actionable errors BEFORE any provider HTTP call is made.

use anyhow::{bail, Result};
use chrono::Utc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Global budget guard shared across CLI and daemon AI invocations.
static GLOBAL_BUDGET_GUARD: OnceLock<AiBudgetGuard> = OnceLock::new();

/// Get a reference to the global budget guard.
pub fn global_budget_guard() -> &'static AiBudgetGuard {
    GLOBAL_BUDGET_GUARD.get_or_init(AiBudgetGuard::new)
}

/// Estimate token count for a text slice (heuristic ~4 chars per token, minimum 1 if non-empty).
pub fn estimate_tokens(text: &str) -> u64 {
    if text.is_empty() {
        0
    } else {
        (text.len() as u64 / 4).max(1)
    }
}

pub struct AiBudgetGuard {
    state: Mutex<BudgetState>,
}

struct BudgetState {
    current_date: String,
    daily_tokens_used: u64,
    request_timestamps: Vec<Instant>,
}

impl Default for AiBudgetGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl AiBudgetGuard {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(BudgetState {
                current_date: String::new(),
                daily_tokens_used: 0,
                request_timestamps: Vec::new(),
            }),
        }
    }

    /// Check if the request is within rate limits and daily token budget.
    /// Returns `Err` without calling the provider if limits are exceeded.
    pub fn check_and_reserve(
        &self,
        daily_limit: u64,
        rpm_limit: u32,
        estimated_prompt_tokens: u64,
    ) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| anyhow::anyhow!("mutex poisoned: {e}"))?;
        let now = Instant::now();
        let today = Utc::now().format("%Y-%m-%d").to_string();

        // Rollover daily tokens if UTC date changed
        if state.current_date != today {
            state.current_date = today;
            state.daily_tokens_used = 0;
        }

        // 1. Check RPM (requests per minute, sliding 60s window)
        if rpm_limit > 0 {
            state
                .request_timestamps
                .retain(|&t| now.duration_since(t) < Duration::from_secs(60));
            if state.request_timestamps.len() >= rpm_limit as usize {
                bail!(
                    "AI rate limit exceeded: maximum {} requests/min (used {} in the last 60s). Wait before retrying.",
                    rpm_limit,
                    state.request_timestamps.len()
                );
            }
        }

        // 2. Check daily token limit
        if daily_limit > 0 {
            let projected = state.daily_tokens_used + estimated_prompt_tokens;
            if projected > daily_limit {
                bail!(
                    "AI daily token budget exceeded: used {} + estimated {} = {} > {} tokens/day. Wait for quota reset at 00:00 UTC or increase daily_token_limit.",
                    state.daily_tokens_used,
                    estimated_prompt_tokens,
                    projected,
                    daily_limit
                );
            }
        }

        // Reserve
        state.request_timestamps.push(now);
        state.daily_tokens_used += estimated_prompt_tokens;
        Ok(())
    }

    /// Record actual additional tokens used after completion.
    pub fn record_tokens(&self, additional_tokens: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.daily_tokens_used += additional_tokens;
        }
    }

    /// Inspect current usage: (daily_tokens_used, requests_in_last_60s).
    pub fn get_usage(&self) -> (u64, usize) {
        if let Ok(mut state) = self.state.lock() {
            let now = Instant::now();
            state
                .request_timestamps
                .retain(|&t| now.duration_since(t) < Duration::from_secs(60));
            (state.daily_tokens_used, state.request_timestamps.len())
        } else {
            (0, 0)
        }
    }

    /// Reset state for test isolation.
    pub fn reset_for_test(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.current_date = Utc::now().format("%Y-%m-%d").to_string();
            state.daily_tokens_used = 0;
            state.request_timestamps.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_estimation() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hi"), 1);
        assert_eq!(estimate_tokens("12345678"), 2);
    }

    #[test]
    fn test_rpm_limit_enforced() {
        let guard = AiBudgetGuard::new();
        // Limit: 2 req/min
        assert!(guard.check_and_reserve(0, 2, 10).is_ok());
        assert!(guard.check_and_reserve(0, 2, 10).is_ok());
        let err = guard.check_and_reserve(0, 2, 10).unwrap_err();
        assert!(err.to_string().contains("AI rate limit exceeded"));
    }

    #[test]
    fn test_daily_token_limit_enforced() {
        let guard = AiBudgetGuard::new();
        // Limit: 100 tokens/day
        assert!(guard.check_and_reserve(100, 0, 50).is_ok());
        assert!(guard.check_and_reserve(100, 0, 40).is_ok());
        // 50 + 40 + 20 = 110 > 100
        let err = guard.check_and_reserve(100, 0, 20).unwrap_err();
        assert!(err.to_string().contains("AI daily token budget exceeded"));
    }
}
