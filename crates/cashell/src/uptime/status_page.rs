//! Static Standalone HTML Status Page Generator (U-05).
//!
//! Generates a single-file, offline-capable HTML status page with embedded CSS
//! and zero external CDN/JavaScript dependencies.
//!
//! Security requirement:
//! Only monitors marked with `public = true` are included in the generated HTML.
//! Private monitor URLs/targets are strictly omitted.

use super::types::{MonitorState, MonitorStatusSummary};
use crate::db::IncidentRecord;

/// Generate self-contained HTML status page content (U-05).
pub fn render_status_page(
    title: &str,
    monitors: &[MonitorStatusSummary],
    incidents: &[IncidentRecord],
) -> String {
    // Security enforcement: filter ONLY public monitors (U-05)
    let public_monitors: Vec<&MonitorStatusSummary> =
        monitors.iter().filter(|m| m.public).collect();

    let all_operational = public_monitors.iter().all(|m| m.state == MonitorState::Up);
    let any_down = public_monitors
        .iter()
        .any(|m| m.state == MonitorState::Down);

    let (badge_text, badge_color, header_bg) = if public_monitors.is_empty() {
        ("No public services configured", "#64748b", "#334155")
    } else if any_down {
        ("Major Outage Detected", "#ef4444", "#7f1d1d")
    } else if !all_operational {
        ("Partial System Degradation", "#f59e0b", "#78350f")
    } else {
        ("All Systems Operational", "#10b981", "#064e3b")
    };

    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
    html.push_str(&format!("<title>{title} — Status</title>\n"));
    html.push_str(r#"<style>
:root {
  --bg: #0f172a;
  --card-bg: #1e293b;
  --text: #f8fafc;
  --muted: #94a3b8;
  --border: #334155;
  --success: #10b981;
  --warning: #f59e0b;
  --danger: #ef4444;
}
* { box-sizing: border-box; margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; }
body { background: var(--bg); color: var(--text); padding: 2rem 1rem; line-height: 1.5; }
.container { max-width: 800px; margin: 0 auto; }
header { margin-bottom: 2rem; display: flex; justify-content: space-between; align-items: center; }
h1 { font-size: 1.5rem; font-weight: 700; }
.badge { display: inline-block; padding: 0.75rem 1.25rem; border-radius: 8px; font-weight: 600; font-size: 1rem; margin-bottom: 2rem; width: 100%; text-align: center; }
.card { background: var(--card-bg); border: 1px solid var(--border); border-radius: 8px; margin-bottom: 1.5rem; padding: 1.25rem; }
.card-header { font-size: 1.1rem; font-weight: 600; margin-bottom: 1rem; display: flex; justify-content: space-between; }
.monitor-item { display: flex; justify-content: space-between; align-items: center; padding: 0.75rem 0; border-bottom: 1px solid var(--border); }
.monitor-item:last-child { border-bottom: none; }
.status-pill { font-size: 0.8rem; font-weight: 600; padding: 0.25rem 0.5rem; border-radius: 4px; text-transform: uppercase; }
.pill-up { background: rgba(16, 185, 129, 0.2); color: var(--success); }
.pill-degraded { background: rgba(245, 158, 11, 0.2); color: var(--warning); }
.pill-down { background: rgba(239, 68, 68, 0.2); color: var(--danger); }
.metric { color: var(--muted); font-size: 0.85rem; }
.history-bar { display: inline-flex; gap: 2px; align-items: center; }
.dot { width: 6px; height: 16px; border-radius: 2px; }
.dot-up { background: var(--success); }
.dot-down { background: var(--danger); }
.incident-item { border-left: 3px solid var(--border); padding-left: 1rem; margin-bottom: 1rem; }
.incident-date { font-size: 0.8rem; color: var(--muted); }
footer { margin-top: 3rem; text-align: center; color: var(--muted); font-size: 0.8rem; }
</style>
</head>
<body>
<div class="container">
  <header>
    <h1>"#);
    html.push_str(title);
    html.push_str("</h1>\n    <div class=\"metric\">Status Page</div>\n  </header>\n\n");

    // Status Banner
    html.push_str(&format!(
        "  <div class=\"badge\" style=\"background: {header_bg}; color: {badge_color};\">{badge_text}</div>\n\n"
    ));

    // Services Card
    html.push_str(
        "  <div class=\"card\">\n    <div class=\"card-header\">Services & Endpoints</div>\n",
    );
    if public_monitors.is_empty() {
        html.push_str("    <p class=\"metric\">No public services are currently configured.</p>\n");
    } else {
        for m in &public_monitors {
            let (pill_class, status_label) = match m.state {
                MonitorState::Up => ("pill-up", "Operational"),
                MonitorState::Degraded => ("pill-degraded", "Degraded"),
                MonitorState::Down => ("pill-down", "Outage"),
            };

            let latency = m
                .latency_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "-".to_string());

            html.push_str("    <div class=\"monitor-item\">\n");
            html.push_str(&format!(
                "      <div>\n        <div style=\"font-weight: 600;\">{}</div>\n        <div class=\"metric\">Uptime 30d: {:.1}% · Latency: {}</div>\n      </div>\n",
                m.name, m.uptime_pct_30d, latency
            ));
            html.push_str("      <div style=\"display: flex; gap: 1rem; align-items: center;\">\n");

            // Sparkline history dots
            html.push_str("        <div class=\"history-bar\">\n");
            for is_up in &m.history_20 {
                let dot_class = if *is_up { "dot-up" } else { "dot-down" };
                html.push_str(&format!(
                    "          <div class=\"dot {dot_class}\"></div>\n"
                ));
            }
            html.push_str("        </div>\n");

            html.push_str(&format!(
                "        <span class=\"status-pill {pill_class}\">{status_label}</span>\n"
            ));
            html.push_str("      </div>\n    </div>\n");
        }
    }
    html.push_str("  </div>\n\n");

    // Incidents Card
    html.push_str("  <div class=\"card\">\n    <div class=\"card-header\">Past Incidents</div>\n");
    if incidents.is_empty() {
        html.push_str("    <p class=\"metric\">No recorded incidents in recent history.</p>\n");
    } else {
        for inc in incidents.iter().take(5) {
            let ended = inc.ended_at.as_deref().unwrap_or("Ongoing");
            let cause = inc.cause.as_deref().unwrap_or("Unspecified incident");
            html.push_str("    <div class=\"incident-item\">\n");
            html.push_str(&format!(
                "      <div style=\"font-weight: 600;\">Service Outage — {}</div>\n",
                inc.monitor
            ));
            html.push_str(&format!(
                "      <div class=\"incident-date\">Started: {} · Resolved: {}</div>\n",
                inc.started_at, ended
            ));
            html.push_str(&format!(
                "      <p style=\"font-size: 0.9rem; margin-top: 0.25rem;\">{}</p>\n",
                cause
            ));
            html.push_str("    </div>\n");
        }
    }
    html.push_str("  </div>\n\n");

    html.push_str("  <footer>Powered by CAShell Sovereign Fleet Monitor</footer>\n");
    html.push_str("</div>\n</body>\n</html>");

    html
}
