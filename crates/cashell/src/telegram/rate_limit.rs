//! Per-user sliding-window rate limiter for Telegram (T-06).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Rate limiter tracking request timestamps per user ID.
pub struct RateLimiter {
    pub max_requests: usize,
    pub window: Duration,
    history: Mutex<HashMap<i64, Vec<Instant>>>,
}

impl RateLimiter {
    /// Create a rate limiter allowing `max_requests` within `window`.
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            history: Mutex::new(HashMap::new()),
        }
    }

    /// Check if user has exceeded rate limit.
    /// Returns `Ok(())` if permitted, or `Err(wait_secs)` if rate limited.
    pub fn check(&self, user_id: i64) -> Result<(), u64> {
        let now = Instant::now();
        let mut guard = self.history.lock().unwrap();
        let timestamps = guard.entry(user_id).or_default();

        // Evict expired timestamps
        timestamps.retain(|&t| now.duration_since(t) <= self.window);

        if timestamps.len() >= self.max_requests {
            let oldest = timestamps[0];
            let remaining = self.window.saturating_sub(now.duration_since(oldest));
            return Err(remaining.as_secs().max(1));
        }

        timestamps.push(now);
        Ok(())
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        // Default: 20 commands per 60 seconds
        Self::new(20, Duration::from_secs(60))
    }
}
