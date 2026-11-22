//! AI operator for the AI tab.
//!
//! The model gets CAShell tools. Read tools (status, servers, containers,
//! logs, monitors, sites, settings) run at once and their results go back to
//! the model. Write tools (add/remove server, monitor, site, change a
//! setting, container start/stop/restart, send Telegram) are never run by the
//! model: they become a confirmation card the user accepts with `y` or
//! rejects with `n`. Every executed write is recorded in the audit log.

use super::CockpitApp;
use crate::ai::tools::{AiResponse, ToolCall, ToolDefinition};
use serde_json::{json, Value};

/// A write tool waiting for the user's `y` / `n`.
#[derive(Debug, Clone)]
pub struct PendingTool {
    pub call: ToolCall,
    pub summary: String,
}

/// Settings the AI may change, with the accepted values.
const SETTING_KEYS: &[(&str, &str)] = &[
    ("theme", "theme name"),
    ("matrix_rain", "true | false"),
    ("privacy_level", "Standard | Strict"),
    (
        "mascot.color_scheme",
        "Dynamic Reactive | Phosphor Green | Cyan Cyber | Amber Retro | Crimson Red | Neon Violet",
    ),
    ("mascot.speed", "Fast | Normal | Slow | Zen"),
    ("daemon.interval_secs", "10..3600"),
    (
        "daemon.cpu_temp_alert_c",
        "0 (off) or 50..110 — Telegram alert when the CPU gets this hot",
    ),
    ("database.retention_days", "1..3650"),
];

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> ToolDefinition {
    ToolDefinition::new(
        name,
        desc,
        json!({ "type": "object", "properties": props, "required": required }),
    )
}

pub fn is_write(name: &str) -> bool {
    matches!(
        name,
        "add_server"
            | "remove_server"
            | "add_monitor"
            | "remove_monitor"
            | "add_site"
            | "update_setting"
            | "container_action"
            | "send_telegram"
    )
}

pub fn tools() -> Vec<ToolDefinition> {
    let settings = SETTING_KEYS
        .iter()
        .map(|(k, v)| format!("{k} ({v})"))
        .collect::<Vec<_>>()
        .join("; ");
    vec![
        tool("get_overview", "Live status of this machine and the fleet: CPU, temperature, RAM, disk, containers, servers, monitors that are down.", json!({}), &[]),
        tool("list_servers", "Registered fleet servers with IP, user, login method, status, latency and load.", json!({}), &[]),
        tool("list_containers", "Docker/Podman containers on this machine and fleet servers (remote ones are named 'server::name').", json!({"server": {"type": "string", "description": "Only this server (optional)"}}), &[]),
        tool("container_logs", "Last lines of a container's logs (local or 'server::name').", json!({"name": {"type": "string"}, "lines": {"type": "integer", "description": "default 60, max 300"}}), &["name"]),
        tool("list_monitors", "Uptime monitors with target URL, up/down, latency and uptime %.", json!({}), &[]),
        tool("list_sites", "Analytics sites tracked by CAShell.", json!({}), &[]),
        tool("get_settings", "Current CAShell settings the assistant can read or change, and whether Telegram is set up.", json!({}), &[]),
        tool("add_server", "Register a fleet server (SSH key or Tailscale login; passwords must be added by the user in the Server tab). Needs confirmation.", json!({
            "name": {"type": "string"}, "ip": {"type": "string"}, "user": {"type": "string"},
            "auth": {"type": "string", "enum": ["key", "tailscale"]},
            "key_path": {"type": "string", "description": "private key path, empty = ssh-agent/default"},
            "ssh_port": {"type": "integer"}
        }), &["name", "ip", "user"]),
        tool("remove_server", "Remove a registered fleet server. Needs confirmation.", json!({"name": {"type": "string"}}), &["name"]),
        tool("add_monitor", "Add an uptime monitor for an http(s) URL. Needs confirmation.", json!({"name": {"type": "string"}, "url": {"type": "string"}, "group": {"type": "string"}}), &["name", "url"]),
        tool("remove_monitor", "Remove an uptime monitor by name. Needs confirmation.", json!({"name": {"type": "string"}}), &["name"]),
        tool("add_site", "Track a website in Analytics. Needs confirmation.", json!({"name": {"type": "string"}, "domain": {"type": "string"}}), &["name", "domain"]),
        tool("update_setting", &format!("Change one CAShell setting. Needs confirmation. Keys: {settings}"), json!({"key": {"type": "string"}, "value": {"type": "string"}}), &["key", "value"]),
        tool("container_action", "Start, stop or restart a local container. Needs confirmation.", json!({"name": {"type": "string"}, "action": {"type": "string", "enum": ["start", "stop", "restart"]}}), &["name", "action"]),
        tool("send_telegram", "Send a message to the configured Telegram chat. Needs confirmation.", json!({"message": {"type": "string"}}), &["message"]),
    ]
}

pub const SYSTEM_PROMPT: &str =
    "You are the CAShell operator, built into a local-first server cockpit. \
Answer in English, short and practical. Use the tools to look at real data before answering \
questions about servers, containers, uptime, analytics, logs or settings; never invent values. \
To change something, call the matching write tool: CAShell shows it to the user for confirmation, \
so do not ask for permission in text first. When analysing, point out problems and the next step.";

fn arg<'a>(call: &'a ToolCall, k: &str) -> &'a str {
    call.arguments
        .get(k)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
}

/// One-line description shown on the confirmation card.
pub fn describe(call: &ToolCall) -> String {
    match call.name.as_str() {
        "add_server" => format!(
            "add server '{}' → {}@{} ({})",
            arg(call, "name"),
            arg(call, "user"),
            arg(call, "ip"),
            if arg(call, "auth").is_empty() {
                "key"
            } else {
                arg(call, "auth")
            }
        ),
        "remove_server" => format!("remove server '{}'", arg(call, "name")),
        "add_monitor" => format!(
            "add uptime monitor '{}' → {}",
            arg(call, "name"),
            arg(call, "url")
        ),
        "remove_monitor" => format!("remove uptime monitor '{}'", arg(call, "name")),
        "add_site" => format!(
            "track site '{}' ({}) in Analytics",
            arg(call, "name"),
            arg(call, "domain")
        ),
        "update_setting" => format!("set {} = {}", arg(call, "key"), arg(call, "value")),
        "container_action" => format!("{} container '{}'", arg(call, "action"), arg(call, "name")),
        "send_telegram" => format!("send Telegram message: \"{}\"", arg(call, "message")),
        other => other.to_string(),
    }
}

/// Run a read tool and return its result as compact JSON text.
pub fn run_read(app: &mut CockpitApp, call: &ToolCall) -> String {
    let snap = &app.latest_snap;
    let v = match call.name.as_str() {
        "get_overview" => json!({
            "host": app.hostname,
            "cpu_pct": snap.cpu_pct,
            "cpu_temp_c": snap.cpu_temp_c,
            "ram_used_mb": snap.ram_used_mb, "ram_total_mb": snap.ram_total_mb,
            "swap_used_mb": snap.swap_used_mb,
            "disk_used_gb": snap.disk_used_gb, "disk_total_gb": snap.disk_total_gb,
            "containers_total": snap.docker_containers.len(),
            "containers_down": snap.docker_containers.iter().filter(|c| c.state != "running").map(|c| &c.name).collect::<Vec<_>>(),
            "servers": snap.fleet_nodes.iter().map(|n| json!({"name": n.name, "status": n.status, "latency_ms": n.latency_ms})).collect::<Vec<_>>(),
            "monitors_down": snap.uptime_monitors.iter().filter(|m| !m.is_up).map(|m| &m.name).collect::<Vec<_>>(),
        }),
        "list_servers" => json!(snap
            .fleet_nodes
            .iter()
            .map(|n| {
                let cfg_node = app.cfg.daemon.nodes.get(&n.name);
                json!({
                    "name": n.name, "ip": n.ip, "status": n.status, "latency_ms": n.latency_ms,
                    "cpu_pct": n.cpu_pct, "ram_pct": n.ram_pct, "disk_pct": n.disk_pct,
                    "user": cfg_node.map(|c| c.user.clone()),
                    "auth": cfg_node.map(|c| c.auth.label()),
                })
            })
            .collect::<Vec<_>>()),
        "list_containers" => {
            let only = arg(call, "server").to_string();
            json!(snap.docker_containers.iter().filter(|c| {
                only.is_empty() || c.name.starts_with(&format!("{only}::"))
            }).map(|c| json!({"name": c.name, "image": c.image, "state": c.state, "status": c.status_detail, "ports": c.ports})).collect::<Vec<_>>())
        }
        "container_logs" => {
            let name = arg(call, "name").to_string();
            let lines = call
                .arguments
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(60)
                .min(300);
            json!({ "name": name, "logs": container_logs(app, &name, lines) })
        }
        "list_monitors" => json!(snap
            .uptime_monitors
            .iter()
            .map(|m| json!({
                "name": m.name, "target": m.target, "up": m.is_up, "latency_ms": m.latency_ms,
                "status_code": m.status_code, "uptime_pct": m.uptime_pct,
            }))
            .collect::<Vec<_>>()),
        "list_sites" => json!(app
            .analytic_sites()
            .into_iter()
            .filter(|s| !s.id.is_empty())
            .map(|s| json!({"id": s.id, "name": s.name, "domain": s.domain}))
            .collect::<Vec<_>>()),
        "get_settings" => json!({
            "theme": app.cfg.output.theme,
            "matrix_rain": app.cfg.output.matrix_rain,
            "privacy_level": app.cfg.output.privacy_level,
            "mascot.color_scheme": app.cfg.mascot.color_scheme,
            "mascot.speed": app.cfg.mascot.speed,
            "daemon.interval_secs": app.cfg.daemon.interval_secs,
            "daemon.cpu_temp_alert_c": app.cfg.daemon.cpu_temp_alert_c,
            "database.retention_days": app.cfg.database.retention_days,
            "telegram_configured": !crate::notify::telegram_token(&app.cfg).trim().is_empty()
                && (!app.cfg.daemon.telegram_chat_id.is_empty() || !app.cfg.uptime.telegram_chat_id.is_empty()),
            "ai_provider": app.cfg.ai.default_provider,
        }),
        other => json!({ "error": format!("unknown read tool {other}") }),
    };
    v.to_string()
}

fn container_logs(app: &CockpitApp, name: &str, lines: u64) -> String {
    let n = lines.to_string();
    let out = match name.split_once("::") {
        Some((server, cname)) => {
            let Some(node) = app.cfg.daemon.nodes.get(server) else {
                return format!("unknown server '{server}'");
            };
            if crate::actions::validate_identifier(cname).is_err() {
                return "invalid container name".into();
            }
            let remote = format!(
                "docker logs --tail {n} {cname} 2>&1 || podman logs --tail {n} {cname} 2>&1"
            );
            crate::host_guard::output(&mut crate::fleet_ssh::ssh_command(
                server,
                node,
                Some(&remote),
            ))
        }
        None => {
            if crate::actions::validate_identifier(name).is_err() {
                return "invalid container name".into();
            }
            crate::host_guard::output(
                std::process::Command::new("docker").args(["logs", "--tail", &n, name]),
            )
            .ok()
            .filter(|o| o.status.success())
            .map_or_else(
                || {
                    crate::host_guard::output(
                        std::process::Command::new("podman").args(["logs", "--tail", &n, name]),
                    )
                },
                Ok,
            )
        }
    };
    match out {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            // keep the tail; logs may be long
            let tail: String = s
                .chars()
                .rev()
                .take(6000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            tail
        }
        Err(e) => format!("could not read logs: {e}"),
    }
}

/// Execute a confirmed write tool. Returns a human-readable result.
pub fn run_write(app: &mut CockpitApp, call: &ToolCall) -> Result<String, String> {
    let res = match call.name.as_str() {
        "add_server" => {
            let (name, ip, user) = (arg(call, "name"), arg(call, "ip"), arg(call, "user"));
            if name.is_empty() || ip.is_empty() || user.is_empty() {
                return Err("name, ip and user are required".into());
            }
            if app.cfg.daemon.nodes.contains_key(name) {
                return Err(format!("server '{name}' already exists"));
            }
            let auth = if arg(call, "auth") == "tailscale" {
                crate::config::NodeAuth::Tailscale
            } else {
                crate::config::NodeAuth::Key
            };
            let port = call
                .arguments
                .get("ssh_port")
                .and_then(Value::as_u64)
                .unwrap_or(22);
            let services = if port != 22 {
                vec![crate::config::ServiceConfig {
                    name: "ssh".into(),
                    port: u16::try_from(port).map_err(|_| "invalid ssh_port")?,
                    protocol: "tcp".into(),
                }]
            } else {
                Vec::new()
            };
            app.cfg.daemon.nodes.insert(
                name.to_string(),
                crate::config::NodeConfig {
                    ip: ip.to_string(),
                    user: user.to_string(),
                    services,
                    active: true,
                    auth,
                    key_path: arg(call, "key_path").to_string(),
                },
            );
            Ok(format!("Server '{name}' added. Confirm its host key in the Server tab [2] before first use."))
        }
        "remove_server" => {
            let name = arg(call, "name");
            if app.cfg.daemon.nodes.remove(name).is_none() {
                return Err(format!("no server named '{name}'"));
            }
            Ok(format!("Server '{name}' removed."))
        }
        "add_monitor" => {
            let (name, url) = (arg(call, "name"), arg(call, "url"));
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err("url must start with http:// or https://".into());
            }
            let exists = app
                .cfg
                .uptime
                .custom_monitors
                .iter()
                .any(|m| m.name == name || m.target == url)
                || app
                    .latest_snap
                    .uptime_monitors
                    .iter()
                    .any(|m| m.target == url);
            if exists {
                return Err("a monitor with that name or URL already exists".into());
            }
            let group = if arg(call, "group").is_empty() {
                "Custom"
            } else {
                arg(call, "group")
            };
            app.cfg
                .uptime
                .custom_monitors
                .push(crate::config::UptimeMonitorConfig {
                    name: name.to_string(),
                    target: url.to_string(),
                    group: group.to_string(),
                    monitor_type: "http".into(),
                    interval_secs: 60,
                    expected_status: None,
                    expected_keyword: None,
                    public: false,
                });
            if let Some(ref d) = app.db {
                let _ = d.add_uptime_monitor(name, url);
            }
            Ok(format!("Monitor '{name}' added for {url}."))
        }
        "remove_monitor" => {
            let name = arg(call, "name").to_string();
            let before = app.cfg.uptime.custom_monitors.len();
            app.cfg.uptime.custom_monitors.retain(|m| m.name != name);
            let known = app
                .latest_snap
                .uptime_monitors
                .iter()
                .any(|m| m.name == name);
            if before == app.cfg.uptime.custom_monitors.len() && !known {
                return Err(format!("no monitor named '{name}'"));
            }
            if !app.cfg.uptime.disabled_monitors.contains(&name) {
                app.cfg.uptime.disabled_monitors.push(name.clone());
            }
            if let Some(ref d) = app.db {
                let _ = d.delete_uptime_monitor(&name);
            }
            app.latest_snap.uptime_monitors.retain(|m| m.name != name);
            Ok(format!("Monitor '{name}' removed."))
        }
        "add_site" => {
            let (name, domain) = (arg(call, "name"), arg(call, "domain"));
            if name.is_empty() || domain.is_empty() {
                return Err("name and domain are required".into());
            }
            let id = name.to_lowercase().replace(' ', "-");
            let Some(ref d) = app.db else {
                return Err("database not available".into());
            };
            d.add_analytic_site(&id, name, domain)
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "Site '{name}' ({domain}) is tracked. Site ID: {id}"
            ))
        }
        "update_setting" => apply_setting(app, arg(call, "key"), arg(call, "value")),
        "container_action" => {
            let (name, action) = (arg(call, "name").to_string(), arg(call, "action"));
            if !matches!(action, "start" | "stop" | "restart") {
                return Err("action must be start, stop or restart".into());
            }
            if name.contains("::") {
                return Err("actions on remote containers are not supported yet".into());
            }
            crate::actions::validate_identifier(&name).map_err(|e| e.to_string())?;
            let run = |engine: &str| {
                crate::host_guard::output(
                    std::process::Command::new(engine).args([action, name.as_str()]),
                )
            };
            let out = run("docker")
                .ok()
                .filter(|o| o.status.success())
                .map_or_else(|| run("podman"), Ok);
            match out {
                Ok(o) if o.status.success() => Ok(format!("Container '{name}': {action} done.")),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(e) => Err(e.to_string()),
            }
        }
        "send_telegram" => {
            let msg = arg(call, "message").to_string();
            if msg.is_empty() {
                return Err("message is empty".into());
            }
            let ev = crate::notify::NotificationEvent::Custom { text: msg };
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            rt.block_on(crate::notify::send_telegram_checked(&app.cfg, &ev))
                .map_err(|e| e.to_string())?;
            Ok("Telegram message sent.".into())
        }
        other => Err(format!("unknown write tool {other}")),
    };
    let changes_config = !matches!(
        call.name.as_str(),
        "container_action" | "send_telegram" | "add_site"
    );
    if res.is_ok() && changes_config && !app.save_config_silent() {
        return Err("change applied in memory but config.toml could not be saved".into());
    }
    if let Some(ref d) = app.db {
        let detail = match &res {
            Ok(m) => m.clone(),
            Err(e) => format!("failed: {e}"),
        };
        let _ = d.log_event(
            "INFO",
            "ai-operator",
            &format!("{}: {detail}", describe(call)),
        );
    }
    res
}

fn apply_setting(app: &mut CockpitApp, key: &str, value: &str) -> Result<String, String> {
    let num = |lo: u64, hi: u64| -> Result<u64, String> {
        let n: u64 = value.parse().map_err(|_| format!("{key} needs a number"))?;
        if n == 0 && key == "daemon.cpu_temp_alert_c" {
            return Ok(0);
        }
        if !(lo..=hi).contains(&n) {
            return Err(format!("{key} must be between {lo} and {hi}"));
        }
        Ok(n)
    };
    let one_of = |opts: &[&str]| -> Result<String, String> {
        opts.iter()
            .find(|o| o.eq_ignore_ascii_case(value))
            .map(|o| o.to_string())
            .ok_or_else(|| format!("{key} must be one of: {}", opts.join(", ")))
    };
    match key {
        "theme" => app.cfg.output.theme = value.to_string(),
        "matrix_rain" => {
            app.cfg.output.matrix_rain = matches!(
                value.to_ascii_lowercase().as_str(),
                "true" | "on" | "yes" | "1"
            )
        }
        "privacy_level" => app.cfg.output.privacy_level = one_of(&["Standard", "Strict"])?,
        "mascot.color_scheme" => {
            app.cfg.mascot.color_scheme = one_of(&[
                "Dynamic Reactive",
                "Phosphor Green",
                "Cyan Cyber",
                "Amber Retro",
                "Crimson Red",
                "Neon Violet",
            ])?
        }
        "mascot.speed" => app.cfg.mascot.speed = one_of(&["Fast", "Normal", "Slow", "Zen"])?,
        "daemon.interval_secs" => app.cfg.daemon.interval_secs = num(10, 3600)?,
        "daemon.cpu_temp_alert_c" => {
            app.cfg.daemon.cpu_temp_alert_c =
                u32::try_from(num(50, 110)?).map_err(|e| e.to_string())?
        }
        "database.retention_days" => {
            app.cfg.database.retention_days =
                u32::try_from(num(1, 3650)?).map_err(|e| e.to_string())?
        }
        _ => {
            return Err(format!(
                "'{key}' cannot be changed by the assistant. Allowed: {}",
                SETTING_KEYS
                    .iter()
                    .map(|(k, _)| *k)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    }
    let note = if key.starts_with("daemon.") {
        " Restart the daemon to apply it."
    } else {
        ""
    };
    Ok(format!("{key} set to {value}.{note}"))
}

/// Recent conversation as context for the next request.
fn history(app: &CockpitApp, max: usize) -> String {
    let n = app.chat_lines.len();
    app.chat_lines
        .iter()
        .skip(n.saturating_sub(max))
        .filter(|l| l.starts_with("User:") || l.starts_with("AI:"))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Handle one user message: ask the model with tools, run read tools,
/// queue write tools for confirmation, and post the answer to the chat.
pub fn ask(app: &mut CockpitApp, prompt: &str) {
    let client = match reqwest::blocking::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            app.add_ai_response(&format!("Error: {e}"));
            return;
        }
    };
    let defs = tools();
    let context = format!(
        "Conversation so far:\n{}\n\nUser request: {prompt}",
        history(app, 16)
    );
    let mut transcript = String::new();
    for _round in 0..3 {
        let user = if transcript.is_empty() {
            context.clone()
        } else {
            format!("{context}\n\nTool results so far:\n{transcript}\n\nUse them to answer, or call more tools if needed.")
        };
        let resp =
            match crate::ai::query_with_tools(&client, &app.cfg, Some(SYSTEM_PROMPT), &user, &defs)
            {
                Ok(r) => r,
                Err(e) => {
                    app.add_ai_response(&format!("Error: {e}"));
                    return;
                }
            };
        match resp {
            AiResponse::Text(t) => {
                app.add_ai_response(&t);
                return;
            }
            AiResponse::ToolCalls(calls) => {
                let mut ran_read = false;
                for call in calls {
                    if is_write(&call.name) {
                        let summary = describe(&call);
                        app.add_chat_line(&format!(
                            "Tool: ⏸ wants to {summary} — press y to run, n to cancel"
                        ));
                        app.agent_pending.push(PendingTool { call, summary });
                    } else {
                        app.add_chat_line(&format!("Tool: ▸ {}", call.name));
                        let out = run_read(app, &call);
                        transcript.push_str(&format!("{} → {}\n", call.name, out));
                        ran_read = true;
                    }
                }
                if !ran_read {
                    return; // only write tools: wait for the user
                }
            }
        }
    }
    app.add_ai_response("(stopped after 3 tool rounds)");
}

/// `y` / `n` on the first pending write tool.
pub fn resolve_pending(app: &mut CockpitApp, accept: bool) {
    if app.agent_pending.is_empty() {
        return;
    }
    let p = app.agent_pending.remove(0);
    if !accept {
        app.add_chat_line(&format!("Tool: ✖ cancelled: {}", p.summary));
        return;
    }
    match run_write(app, &p.call) {
        Ok(msg) => app.add_chat_line(&format!("Tool: ✔ {msg}")),
        Err(e) => app.add_chat_line(&format!("Error: {} failed: {e}", p.summary)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "1".into(),
            name: name.into(),
            arguments: args,
        }
    }

    #[test]
    fn test_every_tool_is_classified_and_described() {
        for t in tools() {
            let d = describe(&call(&t.name, json!({})));
            assert!(!d.is_empty());
        }
        assert!(is_write("add_server") && !is_write("list_servers"));
    }

    #[test]
    fn test_settings_are_validated() {
        let mut app = CockpitApp::new(&crate::config::Config::default());
        assert!(apply_setting(&mut app, "daemon.cpu_temp_alert_c", "85").is_ok());
        assert_eq!(app.cfg.daemon.cpu_temp_alert_c, 85);
        assert!(apply_setting(&mut app, "daemon.cpu_temp_alert_c", "300").is_err());
        assert!(apply_setting(&mut app, "privacy_level", "strict").is_ok());
        assert_eq!(app.cfg.output.privacy_level, "Strict");
        assert!(apply_setting(&mut app, "ai.api_key", "x").is_err());
    }

    #[test]
    fn test_write_tools_reject_bad_input_without_side_effects() {
        let mut app = CockpitApp::new(&crate::config::Config::default());
        let r = run_write(
            &mut app,
            &call("add_monitor", json!({"name": "x", "url": "ftp://a"})),
        );
        assert!(r.is_err());
        let r = run_write(
            &mut app,
            &call(
                "container_action",
                json!({"name": "a;rm -rf /", "action": "stop"}),
            ),
        );
        assert!(r.is_err());
    }
}
