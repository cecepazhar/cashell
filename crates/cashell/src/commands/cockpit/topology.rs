//! Topology tab: one collapsible tree per server.
//!
//! Server (status, IP, latency, load) → services from `[daemon.nodes]`,
//! app containers and database/infra containers that run on it. Built only
//! from real data: the fleet snapshot, Docker/Podman containers (tagged
//! `node::name` for remote servers) and the config.

use super::{cockpit_theme, CockpitApp};
use crate::systat::{DockerContainerInfo, FleetNodeResume, SysSnapshot};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Image keywords that mark a container as database / infrastructure.
const INFRA_HINTS: &[&str] = &[
    "postgres",
    "mysql",
    "mariadb",
    "redis",
    "mongo",
    "valkey",
    "clickhouse",
    "elastic",
    "rabbitmq",
    "kafka",
    "minio",
    "traefik",
    "caddy",
    "nginx-proxy",
    "coolify-db",
    "coolify-redis",
    "memcached",
    "etcd",
    "keydb",
    "sqlite",
];

pub struct TopoServer<'a> {
    pub node: &'a FleetNodeResume,
    pub key: String,
    pub services: Vec<String>,
    pub apps: Vec<&'a DockerContainerInfo>,
    pub infra: Vec<&'a DockerContainerInfo>,
}

impl TopoServer<'_> {
    fn running(&self) -> usize {
        self.apps
            .iter()
            .chain(self.infra.iter())
            .filter(|c| c.state == "running")
            .count()
    }
    fn down(&self) -> usize {
        self.apps.len() + self.infra.len() - self.running()
    }
}

fn is_local(n: &FleetNodeResume) -> bool {
    n.ip == "127.0.0.1"
}

fn is_infra(c: &DockerContainerInfo) -> bool {
    let hay = format!("{} {}", c.image, c.name).to_ascii_lowercase();
    INFRA_HINTS.iter().any(|h| hay.contains(h))
}

/// "core-vuars2ijmodo…-20261122T082309" → "core-vuars2ijmodo…"
pub fn short_name(name: &str, max: usize) -> String {
    let base = name.rsplit("::").next().unwrap_or(name);
    // drop a trailing "-YYYYMMDDTHHMMSS" deploy stamp
    let base = match base.rsplit_once('-') {
        Some((head, tail))
            if tail.len() == 15
                && tail.get(8..9) == Some("T")
                && tail.chars().filter(|c| c.is_ascii_digit()).count() == 14 =>
        {
            head
        }
        _ => base,
    };
    if base.chars().count() > max {
        let cut: String = base.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        base.to_string()
    }
}

/// Group the snapshot into one entry per server.
pub fn build<'a>(snap: &'a SysSnapshot, cfg: &crate::config::Config) -> Vec<TopoServer<'a>> {
    snap.fleet_nodes
        .iter()
        .map(|node| {
            // Config key for remote nodes is the fleet name; local is "local".
            let key = if is_local(node) {
                "local".to_string()
            } else {
                node.name.clone()
            };
            let services = cfg
                .daemon
                .nodes
                .get(&key)
                .map(|n| {
                    n.services
                        .iter()
                        .map(|s| format!("{}:{}", s.name, s.port))
                        .collect()
                })
                .unwrap_or_default();
            let mine = snap
                .docker_containers
                .iter()
                .filter(|c| match c.name.split_once("::") {
                    Some((owner, _)) => owner == node.name,
                    None => is_local(node),
                });
            let (infra, apps): (Vec<_>, Vec<_>) = mine.partition(|c| is_infra(c));
            TopoServer {
                node,
                key,
                services,
                apps,
                infra,
            }
        })
        .collect()
}

fn status_style(node: &FleetNodeResume) -> (&'static str, Color) {
    match (node.status.as_str(), node.latency_ms) {
        ("ONLINE", Some(ms)) if ms >= 300 => ("▲", Color::Yellow),
        ("ONLINE", _) => ("●", Color::Green),
        ("STANDBY", _) => ("◌", Color::Yellow),
        _ => ("✖", Color::Red),
    }
}

fn pct(v: Option<f32>) -> String {
    v.map_or("--".to_string(), |p| format!("{p:.0}%"))
}

pub fn draw_topology_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let muted = Style::default().fg(palette.text_muted);
    let text = Style::default().fg(palette.text);
    let bold = |c: Color| Style::default().fg(c).add_modifier(Modifier::BOLD);

    let servers = build(&app.latest_snap, &app.cfg);
    if servers.is_empty() {
        f.render_widget(
            Paragraph::new("No servers yet. Add one in the Server tab [2].")
                .style(muted)
                .block(Block::default().borders(Borders::ALL).title(" Topology ")),
            area,
        );
        return;
    }
    let sel = app.selected_topology_node.min(servers.len() - 1);
    app.selected_topology_node = sel;

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(area);
    let (Some(&left), Some(&right)) = (split.first(), split.get(1)) else {
        return;
    };

    // ── Left: tree ───────────────────────────────────────────────────────
    let online = servers.iter().filter(|s| s.node.status == "ONLINE").count();
    let total_c: usize = servers.iter().map(|s| s.apps.len() + s.infra.len()).sum();
    let down_c: usize = servers.iter().map(TopoServer::down).sum();
    let mut lines: Vec<Line> = Vec::new();
    let mut sel_line = 0u16;
    for (i, s) in servers.iter().enumerate() {
        let collapsed = app.topology_collapsed.contains(&s.node.name);
        let is_sel = i == sel;
        if is_sel {
            sel_line = lines.len() as u16;
            app.topology_selected_name = s.node.name.clone();
        }
        let (dot, color) = status_style(s.node);
        let row_bg = if is_sel {
            Style::default().bg(palette.row_selected_bg)
        } else {
            Style::default()
        };
        let lat = s
            .node
            .latency_ms
            .map_or("--".to_string(), |ms| format!("{ms}ms"));
        let mut head = vec![
            Span::styled(if collapsed { "▸ " } else { "▾ " }, muted),
            Span::styled(format!("{dot} "), Style::default().fg(color)),
            Span::styled(
                format!("{:<22}", short_name(&s.node.name, 22)),
                bold(palette.text),
            ),
            Span::styled(
                format!("{:<16}", s.node.ip),
                Style::default().fg(palette.accent),
            ),
            Span::styled(format!("{lat:>6}  "), muted),
            Span::styled(
                format!(
                    "CPU {:>4} RAM {:>4} DISK {:>4}  ",
                    pct(s.node.cpu_pct),
                    pct(s.node.ram_pct.map(f32::from)),
                    pct(s.node.disk_pct.map(f32::from))
                ),
                text,
            ),
            Span::styled(
                format!("{} running", s.running()),
                Style::default().fg(Color::Green),
            ),
        ];
        if s.down() > 0 {
            head.push(Span::styled(
                format!(" · {} down", s.down()),
                Style::default().fg(Color::Red),
            ));
        }
        lines.push(Line::from(head).style(row_bg));
        if collapsed {
            continue;
        }
        let branch = |last: bool| if last { "   └─ " } else { "   ├─ " };
        let sections: Vec<(&str, Vec<Span>)> = {
            let mut v: Vec<(&str, Vec<Span>)> = Vec::new();
            if !s.services.is_empty() {
                v.push(("Services", vec![Span::styled(s.services.join(" · "), text)]));
            }
            let chips = |list: &[&DockerContainerInfo]| -> Vec<Span<'static>> {
                let mut out = Vec::new();
                for (k, c) in list.iter().enumerate() {
                    if k > 0 {
                        out.push(Span::styled(" · ", Style::default().fg(Color::DarkGray)));
                    }
                    let (d, col) = if c.state == "running" {
                        ("●", Color::Green)
                    } else {
                        ("✖", Color::Red)
                    };
                    out.push(Span::styled(format!("{d} "), Style::default().fg(col)));
                    out.push(Span::styled(
                        short_name(&c.name, 24),
                        Style::default().fg(palette.text),
                    ));
                }
                out
            };
            if !s.apps.is_empty() {
                v.push(("Apps", chips(&s.apps)));
            }
            if !s.infra.is_empty() {
                v.push(("Data & infra", chips(&s.infra)));
            }
            if v.is_empty() {
                v.push((
                    "Empty",
                    vec![Span::styled("no services or containers found", muted)],
                ));
            }
            v
        };
        let n = sections.len();
        for (k, (label, spans)) in sections.into_iter().enumerate() {
            let count = match label {
                "Apps" => format!(" ({})", s.apps.len()),
                "Data & infra" => format!(" ({})", s.infra.len()),
                _ => String::new(),
            };
            let mut row = vec![
                Span::styled(branch(k + 1 == n), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<18}", format!("{label}{count}")),
                    bold(palette.accent),
                ),
            ];
            row.extend(spans);
            lines.push(Line::from(row));
        }
        lines.push(Line::from(""));
    }

    let visible = left.height.saturating_sub(2);
    let scroll = sel_line.saturating_sub(visible / 3);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " Topology · {online}/{} servers online · {total_c} containers ({down_c} down) ",
                        servers.len()
                    ))
                    .border_style(Style::default().fg(palette.accent)),
            ),
        left,
    );

    // ── Right: inspector ─────────────────────────────────────────────────
    let mut d: Vec<Line> = Vec::new();
    if let Some(s) = servers.get(sel) {
        let (dot, color) = status_style(s.node);
        let kv = |k: &'static str, v: String, c: Color| {
            Line::from(vec![
                Span::styled(format!("{k:<12}"), muted),
                Span::styled(v, Style::default().fg(c)),
            ])
        };
        d.push(Line::from(Span::styled(
            s.node.name.clone(),
            bold(palette.text),
        )));
        d.push(kv("Status", format!("{dot} {}", s.node.status), color));
        d.push(kv("Address", s.node.ip.clone(), palette.accent));
        d.push(kv(
            "Latency",
            s.node
                .latency_ms
                .map_or("unreachable".into(), |ms| format!("{ms} ms")),
            palette.text,
        ));
        d.push(kv("Role", s.node.role.clone(), palette.text));
        d.push(kv(
            "Load",
            format!(
                "CPU {} · RAM {} · Disk {}",
                pct(s.node.cpu_pct),
                pct(s.node.ram_pct.map(f32::from)),
                pct(s.node.disk_pct.map(f32::from))
            ),
            palette.text,
        ));
        d.push(kv(
            "Containers",
            format!(
                "{} apps · {} data/infra · {} running · {} down",
                s.apps.len(),
                s.infra.len(),
                s.running(),
                s.down()
            ),
            palette.text,
        ));
        if !s.services.is_empty() {
            d.push(kv("Services", s.services.join(", "), palette.text));
        }
        let down: Vec<&&DockerContainerInfo> = s
            .apps
            .iter()
            .chain(s.infra.iter())
            .filter(|c| c.state != "running")
            .collect();
        if !down.is_empty() {
            d.push(Line::from(""));
            d.push(Line::from(Span::styled(
                "Needs attention",
                bold(Color::Red),
            )));
            for c in down.iter().take(8) {
                d.push(Line::from(vec![
                    Span::styled("  ✖ ", Style::default().fg(Color::Red)),
                    Span::styled(short_name(&c.name, 30), text),
                    Span::styled(format!("  {}", c.status_detail), muted),
                ]));
            }
        }
        if s.key != "local" && !app.cfg.daemon.nodes.contains_key(&s.key) {
            d.push(Line::from(""));
            d.push(Line::from(Span::styled(
                "Not in [daemon.nodes]: services unknown.",
                muted,
            )));
        }
    }
    d.push(Line::from(""));
    d.push(Line::from(Span::styled(
        "↑/↓ server  ←/→ collapse/expand  r probe  3 containers",
        bold(palette.accent),
    )));
    f.render_widget(
        Paragraph::new(d).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Inspector ")
                .border_style(Style::default().fg(palette.accent)),
        ),
        right,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_name_drops_node_prefix_and_deploy_stamp() {
        assert_eq!(
            short_name("Hostinger VPS::core-abc-20261122T082309", 30),
            "core-abc"
        );
        assert_eq!(short_name("db-postgres", 30), "db-postgres");
        assert_eq!(
            short_name("averyveryverylongcontainername", 10),
            "averyvery…"
        );
    }

    #[test]
    fn test_infra_detection_by_image() {
        let c = DockerContainerInfo {
            name: "db".into(),
            image: "docker.io/library/postgres:16".into(),
            ..Default::default()
        };
        assert!(is_infra(&c));
        let a = DockerContainerInfo {
            name: "web".into(),
            image: "ghcr.io/me/web:1".into(),
            ..Default::default()
        };
        assert!(!is_infra(&a));
    }
}
