//! 24/7 Background Uptime Engine & Heartbeat Runner (U-01..U-07).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use colored::Colorize;

use super::heartbeat::send_heartbeat_ping;
use super::prober::probe_endpoint;
use super::ssl::evaluate_ssl_alert_tier;
use super::types::{IncidentStateMachine, MonitorConfig, StateTransition};
use crate::config::Config;
use crate::db::Database;
use crate::notify::{send_alert, NotificationEvent};

/// Upper bound on probes in flight at once, so a large monitor list cannot
/// exhaust sockets on a small VPS.
const MAX_CONCURRENT_PROBES: usize = 32;

/// How often the scheduler wakes to check which monitors are due.
const SCHEDULER_TICK_SECS: u64 = 5;

/// Floor for a monitor's own `interval_secs`.
const MIN_MONITOR_INTERVAL_SECS: u64 = 10;

/// Uptime Engine background worker running state machines and heartbeat pings.
pub struct UptimeEngine {
    pub config: Config,
    pub db: Arc<Mutex<Database>>,
    pub state_machines: Arc<Mutex<HashMap<String, IncidentStateMachine>>>,
}

impl UptimeEngine {
    pub fn new(config: Config, db: Arc<Mutex<Database>>) -> Self {
        Self {
            config,
            db,
            state_machines: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Run a single probe tick across all configured monitors (U-01..U-06).
    pub async fn run_tick(&self) {
        let monitors = {
            let db = self.db.lock().unwrap();
            super::load_configured_monitors(&self.config, &db)
        };

        self.probe_and_process(monitors).await;
    }

    /// Probe the given monitors concurrently (bounded by
    /// `MAX_CONCURRENT_PROBES`) and handle each result as soon as it lands, so
    /// one slow or timing-out endpoint never delays alerts for the others.
    async fn probe_and_process(&self, monitors: Vec<MonitorConfig>) {
        use futures_util::stream::{self, StreamExt};

        // Each probe runs as its own task: if it were only a future inside
        // this stream it would stop being polled while we await an alert
        // below, and its wall-clock timeout could fire on a healthy endpoint.
        let mut results = stream::iter(monitors.into_iter().map(|m| {
            tokio::spawn(async move {
                let res = probe_endpoint(&m).await;
                (m, res)
            })
        }))
        .buffer_unordered(MAX_CONCURRENT_PROBES);

        while let Some(joined) = results.next().await {
            let Ok((m, res)) = joined else {
                continue; // probe task panicked; next tick retries it
            };

            // 1. Record probe check to SQLite (U-04)
            {
                let db = self.db.lock().unwrap();
                let _ = db.record_uptime_check(
                    &m.name,
                    res.is_up,
                    res.latency_ms,
                    res.status_code,
                    res.error.as_deref(),
                    res.cert_expires_at.as_deref(),
                );
            }

            // 2. Incident State Machine processing & Anti-flapping (U-02)
            let transition = {
                let mut sm_map = self.state_machines.lock().unwrap();
                let sm = sm_map
                    .entry(m.name.clone())
                    .or_insert_with(|| IncidentStateMachine::new(m.name.clone(), 3));
                sm.process_probe(res.is_up, res.error.as_deref())
            };

            if let Some(event) = transition {
                match event {
                    StateTransition::IncidentStarted { cause } => {
                        let incident_id = {
                            let db = self.db.lock().unwrap();
                            let id = db.record_incident_start(&m.name, Some(&cause)).unwrap_or(0);
                            let details = serde_json::json!({
                                "monitor": m.name,
                                "target": m.target,
                                "cause": cause,
                            });
                            let _ = db.log_audit(
                                "uptime",
                                "engine",
                                "incident_down",
                                Some(&details.to_string()),
                                Some("down"),
                                Some(&cause),
                            );
                            id
                        };

                        // Store open incident id in state machine
                        {
                            let mut sm_map = self.state_machines.lock().unwrap();
                            if let Some(sm) = sm_map.get_mut(&m.name) {
                                sm.open_incident_id = Some(incident_id);
                            }
                        }

                        // Send alert notification (F-04)
                        let notify_event = NotificationEvent::Down {
                            monitor: m.name.clone(),
                        };
                        let alert_msg = notify_event.format_markdown();
                        eprintln!("{} {alert_msg}", "⚠".red());
                        let _ = send_alert(&self.config, &notify_event).await;
                    }
                    StateTransition::IncidentResolved { incident_id } => {
                        {
                            let db = self.db.lock().unwrap();
                            if let Some(id) = incident_id {
                                let _ = db.record_incident_end(id);
                            }
                            let details = serde_json::json!({
                                "monitor": m.name,
                                "target": m.target,
                            });
                            let _ = db.log_audit(
                                "uptime",
                                "engine",
                                "incident_recovered",
                                Some(&details.to_string()),
                                Some("recovered"),
                                None,
                            );
                        };

                        let notify_event = NotificationEvent::Recovered {
                            monitor: m.name.clone(),
                            downtime_secs: 0,
                        };
                        let alert_msg = notify_event.format_markdown();
                        println!("{} {alert_msg}", "✔".green());
                        let _ = send_alert(&self.config, &notify_event).await;
                    }
                }
            }

            // 3. SSL Certificate Expiry Alerting (U-03)
            if let Some(days_left) = res.cert_days_left {
                if let Some(tier) = evaluate_ssl_alert_tier(days_left) {
                    let should_alert = {
                        let db = self.db.lock().unwrap();
                        db.should_alert_ssl(&m.name, tier).unwrap_or(false)
                    };

                    if should_alert {
                        let notify_event = NotificationEvent::SslExpiring {
                            domain: m.target.clone(),
                            days: days_left.max(0) as u32,
                        };
                        let alert_msg = notify_event.format_markdown();
                        eprintln!("{} {alert_msg}", "⚠".yellow());
                        let _ = send_alert(&self.config, &notify_event).await;

                        let db = self.db.lock().unwrap();
                        let _ = db.record_ssl_alert(&m.name, tier);
                    }
                }
            }
        }
    }

    /// Background loop running probes, prune rollups, and outgoing heartbeat (U-01..U-07).
    pub async fn run_loop(self: Arc<Self>) {
        let mut probe_interval = tokio::time::interval(Duration::from_secs(SCHEDULER_TICK_SECS));
        probe_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Next-due time per monitor, honouring each monitor's own `interval_secs`.
        let mut next_due: HashMap<String, tokio::time::Instant> = HashMap::new();
        let mut rollup_interval = tokio::time::interval(Duration::from_secs(3600)); // hourly rollup
        let hb_url = self.config.uptime.heartbeat.url.trim().to_string();
        let hb_interval_mins = self.config.uptime.heartbeat.interval_mins.max(1);
        let mut hb_interval = tokio::time::interval(Duration::from_secs(hb_interval_mins * 60));

        println!("{} Uptime engine prober started.", "✔".green());

        loop {
            tokio::select! {
                _ = probe_interval.tick() => {
                    let now = tokio::time::Instant::now();
                    let monitors = {
                        let db = self.db.lock().unwrap();
                        super::load_configured_monitors(&self.config, &db)
                    };
                    // Forget monitors that were removed from config/DB.
                    next_due.retain(|name, _| monitors.iter().any(|m| &m.name == name));

                    let due: Vec<MonitorConfig> = monitors
                        .into_iter()
                        .filter(|m| next_due.get(&m.name).map_or(true, |t| *t <= now))
                        .collect();
                    for m in &due {
                        let every = m.interval_secs.max(MIN_MONITOR_INTERVAL_SECS);
                        next_due.insert(m.name.clone(), now + Duration::from_secs(every));
                    }
                    if !due.is_empty() {
                        self.probe_and_process(due).await;
                    }
                }
                _ = rollup_interval.tick() => {
                    let db = self.db.lock().unwrap();
                    let _ = db.prune_uptime_checks_and_rollup();
                }
                _ = hb_interval.tick() => {
                    if !hb_url.is_empty() {
                        let _ = send_heartbeat_ping(&hb_url).await;
                    }
                }
            }
        }
    }
}
