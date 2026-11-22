//! Sovereign Uptime Engine: Endpoint Probing, SSL Tracking, and Incident Lifecycle (U-01..U-07).

pub mod engine;
pub mod heartbeat;
pub mod prober;
pub mod ssl;
pub mod status_page;
pub mod types;

#[cfg(test)]
pub mod tests;

use anyhow::Result;

pub use engine::UptimeEngine;
pub use heartbeat::send_heartbeat_ping;
pub use prober::{probe_endpoint, probe_http, probe_ping, probe_tcp};
pub use ssl::{
    days_until_expiry, evaluate_ssl_alert_tier, parse_cert_expiry_from_der, probe_ssl_expiry,
};
pub use status_page::render_status_page;
pub use types::{
    IncidentStateMachine, MonitorConfig, MonitorState, MonitorStatusSummary, MonitorType,
    ProbeResult, StateTransition,
};

use crate::config::Config;
use crate::db::Database;

/// Default baseline endpoint targets to probe if not disabled.
pub fn default_endpoint_targets() -> Vec<MonitorConfig> {
    vec![MonitorConfig {
        name: "CAShell Local Daemon".to_string(),
        target: "http://127.0.0.1:8090/api/health".to_string(),
        monitor_type: MonitorType::Http,
        interval_secs: 60,
        timeout_ms: 2000,
        expected_status: None,
        expected_keyword: None,
        public: false,
        group: "Localhost".to_string(),
    }]
}

/// Load and merge all configured monitors from defaults, `config.toml`, and SQLite DB (U-01, U-05).
pub fn load_configured_monitors(cfg: &Config, db: &Database) -> Vec<MonitorConfig> {
    let mut monitors = default_endpoint_targets();

    // 1. Add monitors from config.toml [uptime.custom_monitors]
    for cm in &cfg.uptime.custom_monitors {
        if !monitors.iter().any(|m| m.name == cm.name) {
            let mtype = cm.monitor_type.parse().unwrap_or(MonitorType::Http);
            monitors.push(MonitorConfig {
                name: cm.name.clone(),
                target: cm.target.clone(),
                monitor_type: mtype,
                interval_secs: cm.interval_secs.max(20),
                timeout_ms: 5000,
                expected_status: cm.expected_status,
                expected_keyword: cm.expected_keyword.clone(),
                public: cm.public,
                group: if cm.group.trim().is_empty() {
                    "Custom".to_string()
                } else {
                    cm.group.clone()
                },
            });
        }
    }

    // 2. Add monitors from SQLite DB
    if let Ok(db_monitors) = db.get_all_uptime_monitors_full() {
        for dbm in db_monitors {
            if !monitors.iter().any(|m| m.name == dbm.name) {
                let mtype = dbm.monitor_type.parse().unwrap_or(MonitorType::Http);
                monitors.push(MonitorConfig {
                    name: dbm.name,
                    target: dbm.target,
                    monitor_type: mtype,
                    interval_secs: dbm.interval_secs.max(20),
                    timeout_ms: 5000,
                    expected_status: dbm.expected_status,
                    expected_keyword: dbm.expected_keyword,
                    public: dbm.public,
                    group: dbm.group_name,
                });
            }
        }
    }

    // 3. Filter out user-disabled monitors
    monitors.retain(|m| !cfg.uptime.disabled_monitors.contains(&m.name));

    // Sort by group then name
    monitors.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));

    monitors
}

/// Collect a comprehensive summary of all configured monitors using recorded DB checks (U-04, U-07).
pub fn get_all_monitors_summary(cfg: &Config, db: &Database) -> Result<Vec<MonitorStatusSummary>> {
    let monitors = load_configured_monitors(cfg, db);
    let mut summaries = Vec::new();

    for m in monitors {
        let history = db.get_monitor_history_bits(&m.name, 20).unwrap_or_default();
        let is_up = history.last().copied().unwrap_or(true);
        let open_incident = db.get_open_incident(&m.name)?;

        let state = if open_incident.is_some() {
            MonitorState::Down
        } else if !is_up {
            MonitorState::Degraded
        } else {
            MonitorState::Up
        };

        let pct_24h = db.get_uptime_percentage(&m.name, 1).unwrap_or(100.0);
        let pct_30d = db.get_uptime_percentage(&m.name, 30).unwrap_or(100.0);

        // Fetch latest probe check row
        let (latency_ms, status_code, cert_expires_at): (Option<u64>, Option<u16>, Option<String>) = db
            .conn()
            .query_row(
                "SELECT latency_ms, status_code, cert_expires_at FROM uptime_checks WHERE monitor = ?1 ORDER BY checked_at DESC, id DESC LIMIT 1",
                [&m.name],
                |row| {
                    let lat: Option<i64> = row.get(0)?;
                    let code: Option<i64> = row.get(1)?;
                    let exp: Option<String> = row.get(2)?;
                    Ok((lat.map(|l| l as u64), code.map(|c| c as u16), exp))
                },
            )
            .unwrap_or((None, None, None));

        let cert_days_left = if let Some(ref exp_str) = cert_expires_at {
            if let Ok(exp_dt) = chrono::DateTime::parse_from_rfc3339(exp_str) {
                let now = chrono::Utc::now();
                Some(ssl::days_until_expiry(
                    &exp_dt.with_timezone(&chrono::Utc),
                    &now,
                ))
            } else {
                None
            }
        } else {
            None
        };

        summaries.push(MonitorStatusSummary {
            name: m.name,
            target: m.target,
            group: m.group,
            state,
            is_up,
            latency_ms,
            status_code,
            uptime_pct_24h: pct_24h,
            uptime_pct_30d: pct_30d,
            history_20: history,
            cert_expires_at,
            cert_days_left,
            public: m.public,
        });
    }

    Ok(summaries)
}
