//! About tab: big brandmark, what CAShell is, who builds it, the full feature
//! catalog (scrollable), Free vs Pro and support links.

use super::settings::{benefit_lines, BUY_URL, FOUNDER_URL, SPONSOR_URL};
use super::{cockpit_theme, CockpitApp, BRAND_GREEN};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Readiness shown next to each module. Kept honest: "Beta" = works but not
/// yet proven against real servers/data, or known gaps remain.
#[derive(Clone, Copy)]
enum Status {
    Ready,
    Beta,
    Pro,
}

struct Module {
    name: &'static str,
    key: &'static str,
    status: Status,
    items: &'static [&'static str],
}

const MODULES: &[Module] = &[
    Module {
        name: "Dashboard",
        key: "1",
        status: Status::Ready,
        items: &[
            "Live CPU (per core), load, temperature, RAM, swap, disk and network graphs",
            "SMART disk health, top applications by CPU/RAM/IO",
            "Fleet, container, uptime, analytics and service-health summaries in one view",
            "Sovereign Token avatar that reacts to system health and levels up (Lv.1-7)",
        ],
    },
    Module {
        name: "Server",
        key: "2",
        status: Status::Beta,
        items: &[
            "Register servers by SSH key, password (encrypted) or Tailscale import",
            "Host-key confirmation on first connect (TOFU), latency, CPU, RAM, disk per node",
            "Probe, health check and clone server entries",
        ],
    },
    Module {
        name: "Container",
        key: "3",
        status: Status::Ready,
        items: &[
            "Docker and Podman containers from this machine and every fleet server",
            "Start / stop (s), restart (r), edit (e), delete (x) with confirmation",
            "Add a container with the wizard (d); projects in hierarchy and tree views",
        ],
    },
    Module {
        name: "Uptime",
        key: "4",
        status: Status::Ready,
        items: &[
            "HTTP/TCP monitors with interval, group and public flag; add/edit/delete wizard",
            "Heartbeat history, uptime %, incidents and SSL-expiry tracking",
            "24/7 checks by the daemon; alerts to Telegram",
        ],
    },
    Module {
        name: "Analytics",
        key: "5",
        status: Status::Beta,
        items: &[
            "Self-hosted, privacy-friendly web analytics with a tiny tracker script",
            "Sites, page views, visitors, referrers and countries per site",
        ],
    },
    Module {
        name: "Topology",
        key: "6",
        status: Status::Beta,
        items: &["Map of servers, applications and databases with a node inspector"],
    },
    Module {
        name: "CRON",
        key: "7",
        status: Status::Beta,
        items: &["List, add and remove user crontab and CAShell-managed scheduled jobs"],
    },
    Module {
        name: "Log",
        key: "8",
        status: Status::Ready,
        items: &["journald, Docker/Podman container logs and agent logs with level filter"],
    },
    Module {
        name: "Terminal",
        key: "9",
        status: Status::Beta,
        items: &["Run commands locally or on a fleet server with quick presets"],
    },
    Module {
        name: "AI",
        key: "0",
        status: Status::Beta,
        items: &[
            "Chat with your own AI provider (BYOK: OpenAI, Anthropic, local, any OpenAI-compatible)",
            "Slash commands to switch tabs, check the fleet and restart services",
        ],
    },
    Module {
        name: "Settings",
        key: "-",
        status: Status::Ready,
        items: &[
            "AI, theme, Telegram, daemon, database retention, master password, mascot",
            "License activation and Factory Reset (Danger Zone)",
        ],
    },
    Module {
        name: "Turbo",
        key: "=",
        status: Status::Beta,
        items: &["Performance profiles and quick system tuning actions"],
    },
    Module {
        name: "Privacy mode",
        key: "Ctrl+E",
        status: Status::Pro,
        items: &[
            "Hide IPs, domains, host and user names on every tab for screen sharing",
            "Strict level also hides server, container, app, monitor, site and pet names",
        ],
    },
    Module {
        name: "Backup & profiles",
        key: "CLI",
        status: Status::Ready,
        items: &[
            "`cashell backup create | list | restore`: full state incl. encrypted secrets",
            "`cashell config export | import`: servers, monitors and sites as TOML",
        ],
    },
    Module {
        name: "Command line",
        key: "CLI",
        status: Status::Ready,
        items: &[
            "doctor, git-status, project, app (deploy, logs, rollback), server (firewall, db, docker)",
            "daemon (24/7 monitor + HTTP API), uptime, analytics, telegram, license, pro sync",
            "AI audit / fix / review of code, r2 uploads, markdown vault",
        ],
    },
];

/// Tools CAShell aims to cover from one terminal (direction, not a claim of parity).
const DIRECTION: [(&str, &str); 5] = [
    ("cPanel", "Server: firewall, database, domain & SSL"),
    (
        "Portainer / Dokploy",
        "Containers & deploys on Docker/Podman",
    ),
    ("Uptime Kuma", "Uptime: monitors, incidents, status page"),
    ("Umami / Google Analytics", "Self-hosted analytics you own"),
    ("btop / cron / logs", "Dashboard, CRON, Log & AI BYOK"),
];

const LOGO_W: u16 = 26;
const LOGO_H: u16 = 10; // text rows (20 pixel rows)

pub fn draw_about_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let is_pro = app.is_pro();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(LOGO_H + 2), Constraint::Min(5)])
        .split(area);
    let (Some(&top), Some(&body)) = (rows.first(), rows.get(1)) else {
        return;
    };

    let text = Style::default().fg(palette.text);
    let muted = Style::default().fg(palette.text_muted);
    let bold = |c: Color| Style::default().fg(c).add_modifier(Modifier::BOLD);
    let link = Style::default()
        .fg(palette.accent)
        .add_modifier(Modifier::UNDERLINED);

    // ── Hero: big logo + identity ───────────────────────────────────────────
    let hero = Block::default()
        .borders(Borders::ALL)
        .title(" ■ About CAShell ")
        .border_style(Style::default().fg(palette.accent));
    let hero_in = hero.inner(top);
    f.render_widget(hero, top);
    let show_logo = hero_in.width >= LOGO_W + 40 && hero_in.height >= LOGO_H;
    if show_logo {
        let rgb = |c: (u8, u8, u8)| Color::Rgb(c.0, c.1, c.2);
        let (ox, oy) = (hero_in.x + 1, hero_in.y);
        let buf = f.buffer_mut();
        for (x, y, ch, fgc, bgc) in crate::brand::cells(LOGO_W as usize, (LOGO_H * 2) as usize) {
            if let Some(cell) = buf.cell_mut((ox + x, oy + y)) {
                cell.set_char(ch);
                cell.set_fg(rgb(fgc));
                if let Some(b) = bgc {
                    cell.set_bg(rgb(b));
                }
            }
        }
    }
    let info_x = if show_logo { LOGO_W + 4 } else { 0 };
    let info_area = Rect::new(
        hero_in.x + info_x,
        hero_in.y,
        hero_in.width.saturating_sub(info_x),
        hero_in.height,
    );
    let hero_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("❯❯", bold(BRAND_GREEN)),
            Span::styled("_", bold(Color::White)),
            Span::styled(" CAShell ", bold(palette.accent)),
            Span::styled(
                format!("v{}", env!("CARGO_PKG_VERSION")),
                bold(palette.text),
            ),
            Span::styled(
                if is_pro { "  [PRO]" } else { "  [FREE]" },
                bold(if is_pro {
                    Color::Yellow
                } else {
                    palette.success
                }),
            ),
        ]),
        Line::from(Span::styled(
            "Zero-knowledge, local-first server cockpit & CLI",
            bold(palette.text),
        )),
        Line::from(Span::styled(
            "Manage servers, containers, uptime, analytics and deploys from one terminal. \
             Your data stays on your machine.",
            muted,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Made by ", muted),
            Span::styled("Cecep Azhar", bold(palette.text)),
            Span::styled(" · Fathforce", text),
        ]),
        Line::from(vec![
            Span::styled("fathforce.com", link),
            Span::styled(" · ", muted),
            Span::styled("cecepazhar.com", link),
            Span::styled(" · GitHub ", muted),
            Span::styled("cecepazhar", link),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("♥ Sponsor ", Style::default().fg(Color::LightRed)),
            Span::styled(SPONSOR_URL, link),
            Span::styled("   ↑/↓ scroll features", muted),
        ]),
    ];
    f.render_widget(
        Paragraph::new(hero_lines).wrap(Wrap { trim: false }),
        info_area,
    );

    // ── Body: feature catalog (left, scrolls) + Free/Pro/support (right) ────
    let cols = if body.width >= 110 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(body)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(body)
    };
    let (Some(&left), Some(&right)) = (cols.first(), cols.get(1)) else {
        return;
    };

    let mut l: Vec<Line> = Vec::new();
    for m in MODULES {
        let (badge, color) = match m.status {
            Status::Ready => (" READY ", palette.success),
            Status::Beta => (" BETA ", Color::Yellow),
            Status::Pro => (" PRO ", Color::Magenta),
        };
        l.push(Line::from(vec![
            Span::styled(format!("[{:>6}] ", m.key), muted),
            Span::styled(format!("{:<18}", m.name), bold(palette.accent)),
            Span::styled(
                badge,
                Style::default()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        for it in m.items {
            l.push(Line::from(vec![
                Span::styled("           ▸ ", Style::default().fg(BRAND_GREEN)),
                Span::styled(*it, text),
            ]));
        }
        l.push(Line::from(""));
    }
    l.push(Line::from(Span::styled(
        "WHERE IT IS GOING — one tool, many roles",
        bold(palette.accent),
    )));
    for (tool, role) in DIRECTION {
        l.push(Line::from(vec![
            Span::styled("    ▸ ", Style::default().fg(BRAND_GREEN)),
            Span::styled(format!("{tool:<26}"), bold(palette.text)),
            Span::styled(role, muted),
        ]));
    }
    l.push(Line::from(Span::styled(
        "    Some roles are ready today, others are still being built. \
         Other product names belong to their owners.",
        muted,
    )));

    let total = l.len() as u16;
    let visible = left.height.saturating_sub(2);
    let max_scroll = total.saturating_sub(visible);
    if app.about_scroll > max_scroll {
        app.about_scroll = max_scroll;
    }
    f.render_widget(
        Paragraph::new(l)
            .wrap(Wrap { trim: false })
            .scroll((app.about_scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " ★ Features ({} modules) — ↑/↓ scroll ",
                        MODULES.len()
                    ))
                    .border_style(Style::default().fg(palette.accent)),
            ),
        left,
    );

    let mut r = benefit_lines(&palette);
    r.remove(0);
    r.extend([
        Line::from(""),
        Line::from(vec![
            Span::styled("  Buy Pro       ", muted),
            Span::styled(BUY_URL, link),
        ]),
        Line::from(vec![
            Span::styled("  Founder (all) ", muted),
            Span::styled(FOUNDER_URL, link),
        ]),
        Line::from(Span::styled(
            "  Click [ FREE · Get Pro ▸ ] in the header to enter a key.",
            muted,
        )),
        Line::from(""),
        Line::from(Span::styled("SUPPORT & DONATIONS", bold(palette.accent))),
        Line::from(vec![
            Span::styled("  ♥ Sponsor  ", Style::default().fg(Color::LightRed)),
            Span::styled(SPONSOR_URL, link),
        ]),
        Line::from(vec![
            Span::styled("  ✉ Email    ", muted),
            Span::styled("hi@cecepazhar.com", link),
        ]),
        Line::from(Span::styled(
            "  Every bit of support keeps CAShell independent and its local features free.",
            muted,
        )),
    ]);
    f.render_widget(
        Paragraph::new(r).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" ★ Free vs Pro · Support ")
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        right,
    );
}
