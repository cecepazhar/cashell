//! Notification dispatcher subsystem for CAShell (F-04).
//!
//! Provides a unified `Notifier` abstraction, standard event definitions,
//! and an asynchronous non-blocking retry queue (up to 5 retries with exponential backoff)
//! so that slow or failing notifications never block system health probes.

pub mod telegram;
pub mod webhook;

pub use telegram::TelegramNotifier;
pub use webhook::{DiscordNotifier, SlackNotifier};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Boxed future returned by notifiers to support dynamic dispatch.
pub type NotifyFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// Unified notification event types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationEvent {
    /// Service or node monitor went down.
    Down { monitor: String },
    /// Service or node recovered from outage.
    Recovered { monitor: String, downtime_secs: u64 },
    /// TLS/SSL certificate is expiring soon.
    SslExpiring { domain: String, days: u32 },
    /// High disk space utilization detected.
    DiskHigh { mount: String, percent: u8 },
    /// Dokploy / Docker deployment finished.
    DeployResult {
        app: String,
        success: bool,
        message: String,
    },
}

impl NotificationEvent {
    /// Human-readable Markdown message for chat platforms.
    pub fn format_markdown(&self) -> String {
        match self {
            Self::Down { monitor } => {
                format!("🚨 *[ALERT: DOWN]* Monitor `{monitor}` is down!")
            }
            Self::Recovered {
                monitor,
                downtime_secs,
            } => {
                format!("✅ *[RECOVERED]* Monitor `{monitor}` is back online (downtime: {downtime_secs}s).")
            }
            Self::SslExpiring { domain, days } => {
                format!("⚠️ *[SSL EXPIRING]* Certificate for `{domain}` expires in {days} day(s).")
            }
            Self::DiskHigh { mount, percent } => {
                format!("⚠️ *[DISK WARNING]* Mount `{mount}` disk usage is at {percent}%.")
            }
            Self::DeployResult {
                app,
                success,
                message,
            } => {
                if *success {
                    format!("🚀 *[DEPLOY SUCCESS]* `{app}`: {message}")
                } else {
                    format!("❌ *[DEPLOY FAILED]* `{app}`: {message}")
                }
            }
        }
    }

    /// Plain text format without markdown tags.
    pub fn format_plain(&self) -> String {
        match self {
            Self::Down { monitor } => format!("[ALERT: DOWN] Monitor {monitor} is down!"),
            Self::Recovered {
                monitor,
                downtime_secs,
            } => format!(
                "[RECOVERED] Monitor {monitor} is back online (downtime: {downtime_secs}s)."
            ),
            Self::SslExpiring { domain, days } => {
                format!("[SSL EXPIRING] Certificate for {domain} expires in {days} day(s).")
            }
            Self::DiskHigh { mount, percent } => {
                format!("[DISK WARNING] Mount {mount} disk usage is at {percent}%.")
            }
            Self::DeployResult {
                app,
                success,
                message,
            } => {
                if *success {
                    format!("[DEPLOY SUCCESS] {app}: {message}")
                } else {
                    format!("[DEPLOY FAILED] {app}: {message}")
                }
            }
        }
    }
}

/// Core trait implemented by all notification channels (Telegram, Discord, Slack, etc.).
pub trait Notifier: Send + Sync {
    /// Asynchronously send a notification event.
    fn send<'a>(&'a self, event: &'a NotificationEvent) -> NotifyFuture<'a>;
}

/// Asynchronous notification dispatcher backed by a non-blocking MPSC channel
/// and an exponential-backoff retry queue (max 5 retries).
#[derive(Clone)]
pub struct NotificationDispatcher {
    tx: mpsc::Sender<NotificationEvent>,
}

impl NotificationDispatcher {
    /// Create a dispatcher with default buffer capacity (100) and base backoff (500ms).
    pub fn new(notifiers: Vec<Arc<dyn Notifier>>) -> (Self, tokio::task::JoinHandle<()>) {
        Self::with_config(notifiers, 100, Duration::from_millis(500))
    }

    /// Create a dispatcher with explicit queue capacity and base retry backoff duration.
    pub fn with_config(
        notifiers: Vec<Arc<dyn Notifier>>,
        capacity: usize,
        base_backoff: Duration,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<NotificationEvent>(capacity);

        let handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                for notifier in &notifiers {
                    let notifier = notifier.clone();
                    let event = event.clone();
                    tokio::spawn(async move {
                        let mut backoff = base_backoff;
                        let max_retries = 5;
                        for attempt in 1..=max_retries {
                            match notifier.send(&event).await {
                                Ok(()) => break,
                                Err(e) => {
                                    if attempt == max_retries {
                                        eprintln!(
                                            "⚠ [notify] notification dropped after {max_retries} attempts: {e}"
                                        );
                                    } else {
                                        tokio::time::sleep(backoff).await;
                                        backoff = backoff.saturating_mul(2);
                                    }
                                }
                            }
                        }
                    });
                }
            }
        });

        (Self { tx }, handle)
    }

    /// Dispatch a notification asynchronously.
    ///
    /// Never blocks the caller. If the internal channel is saturated, returns `false`
    /// and logs a warning instead of blocking probing loops.
    pub fn dispatch(&self, event: NotificationEvent) -> bool {
        match self.tx.try_send(event) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                eprintln!("⚠ [notify] dispatcher channel queue full; dropping event");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                eprintln!("⚠ [notify] dispatcher background worker closed");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    #[test]
    fn test_notification_event_formatting() {
        let ev1 = NotificationEvent::Down {
            monitor: "api-prod".into(),
        };
        assert!(ev1.format_markdown().contains("ALERT: DOWN"));
        assert!(ev1.format_markdown().contains("api-prod"));
        assert_eq!(
            ev1.format_plain(),
            "[ALERT: DOWN] Monitor api-prod is down!"
        );

        let ev2 = NotificationEvent::Recovered {
            monitor: "api-prod".into(),
            downtime_secs: 120,
        };
        assert!(ev2.format_markdown().contains("RECOVERED"));
        assert!(ev2.format_markdown().contains("120s"));

        let ev3 = NotificationEvent::SslExpiring {
            domain: "example.com".into(),
            days: 7,
        };
        assert!(ev3.format_markdown().contains("7 day(s)"));

        let ev4 = NotificationEvent::DiskHigh {
            mount: "/var".into(),
            percent: 92,
        };
        assert!(ev4.format_markdown().contains("92%"));

        let ev5 = NotificationEvent::DeployResult {
            app: "cashell-web".into(),
            success: true,
            message: "v1.0.4 deployed".into(),
        };
        assert!(ev5.format_markdown().contains("DEPLOY SUCCESS"));
        assert!(ev5.format_markdown().contains("cashell-web"));
    }

    #[tokio::test]
    async fn test_telegram_and_webhooks_mock_dispatch() {
        let telegram_hits = Arc::new(AtomicUsize::new(0));
        let discord_hits = Arc::new(AtomicUsize::new(0));
        let slack_hits = Arc::new(AtomicUsize::new(0));

        let t_hits = telegram_hits.clone();
        let d_hits = discord_hits.clone();
        let s_hits = slack_hits.clone();

        let app = Router::new()
            .route(
                "/bot12345/sendMessage",
                post(move |Json(payload): Json<serde_json::Value>| {
                    let hits = t_hits.clone();
                    async move {
                        assert!(payload.get("text").is_some());
                        hits.fetch_add(1, Ordering::SeqCst);
                        Json(serde_json::json!({"ok": true}))
                    }
                }),
            )
            .route(
                "/discord",
                post(move |Json(payload): Json<serde_json::Value>| {
                    let hits = d_hits.clone();
                    async move {
                        assert!(payload.get("content").is_some());
                        hits.fetch_add(1, Ordering::SeqCst);
                        axum::http::StatusCode::NO_CONTENT
                    }
                }),
            )
            .route(
                "/slack",
                post(move |Json(payload): Json<serde_json::Value>| {
                    let hits = s_hits.clone();
                    async move {
                        assert!(payload.get("text").is_some());
                        hits.fetch_add(1, Ordering::SeqCst);
                        "ok"
                    }
                }),
            );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let base_url = format!("http://{addr}");
        let telegram = Arc::new(TelegramNotifier::with_base_url(
            "12345", "chat999", &base_url,
        ));
        let discord = Arc::new(DiscordNotifier::new(format!("{base_url}/discord")));
        let slack = Arc::new(SlackNotifier::new(format!("{base_url}/slack")));

        let event = NotificationEvent::Down {
            monitor: "db-master".into(),
        };

        // 1. Direct send test
        telegram.send(&event).await.unwrap();
        assert_eq!(telegram_hits.load(Ordering::SeqCst), 1);

        discord.send(&event).await.unwrap();
        assert_eq!(discord_hits.load(Ordering::SeqCst), 1);

        slack.send(&event).await.unwrap();
        assert_eq!(slack_hits.load(Ordering::SeqCst), 1);

        // 2. Dispatcher async queue test
        let (dispatcher, _handle) = NotificationDispatcher::with_config(
            vec![telegram, discord, slack],
            10,
            Duration::from_millis(5),
        );

        let dispatched = dispatcher.dispatch(NotificationEvent::Recovered {
            monitor: "db-master".into(),
            downtime_secs: 45,
        });
        assert!(dispatched);

        // Give background tasks a brief window to complete
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(telegram_hits.load(Ordering::SeqCst), 2);
        assert_eq!(discord_hits.load(Ordering::SeqCst), 2);
        assert_eq!(slack_hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_dispatcher_retry_exponential_backoff() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_clone = attempts.clone();

        let app = Router::new().route(
            "/flaky",
            post(move || {
                let attempts = attempts_clone.clone();
                async move {
                    let cur = attempts.fetch_add(1, Ordering::SeqCst);
                    if cur < 2 {
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR
                    } else {
                        axum::http::StatusCode::OK
                    }
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let flaky_notifier = Arc::new(DiscordNotifier::new(format!("http://{addr}/flaky")));
        let (dispatcher, _handle) = NotificationDispatcher::with_config(
            vec![flaky_notifier],
            10,
            Duration::from_millis(10),
        );

        dispatcher.dispatch(NotificationEvent::DiskHigh {
            mount: "/".into(),
            percent: 95,
        });

        // First attempt (0) -> fail, wait 10ms
        // Second attempt (1) -> fail, wait 20ms
        // Third attempt (2) -> success!
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
