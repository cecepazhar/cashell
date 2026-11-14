//! Fleet & Node Overview Tab & Wizard — extracted from cockpit.rs (F-07).

use super::{cockpit_theme, format_core_span, CockpitApp, HitboxAction};
use catui::layout::centered_rect;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};

pub fn draw_server_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_add_w = 14u16;
    let btn_edit_w = 14u16;
    let btn_clone_w = 15u16;
    let btn_active_w = 16u16;
    let btn_ssh_w = 15u16;
    let btn_probe_w = 15u16;
    let btn_disk_w = 17u16;
    let btn_del_w = 14u16;

    let mut cur_x = bar_inner.x + 1;
    let y = bar_inner.y;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_add_w,
            height: 1,
        },
        HitboxAction::AddServer,
    );
    cur_x += btn_add_w + 1;
    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_edit_w,
            height: 1,
        },
        HitboxAction::EditServer,
    );
    cur_x += btn_edit_w + 1;
    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_clone_w,
            height: 1,
        },
        HitboxAction::CloneServer,
    );
    cur_x += btn_clone_w + 1;
    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_active_w,
            height: 1,
        },
        HitboxAction::ToggleServerActive,
    );
    cur_x += btn_active_w + 1;
    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_ssh_w,
            height: 1,
        },
        HitboxAction::TestServerSsh,
    );
    cur_x += btn_ssh_w + 1;
    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_probe_w,
            height: 1,
        },
        HitboxAction::ProbeServers,
    );
    cur_x += btn_probe_w + 1;
    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_disk_w,
            height: 1,
        },
        HitboxAction::OpenDiskHealth,
    );
    cur_x += btn_disk_w + 1;
    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_del_w,
            height: 1,
        },
        HitboxAction::RemoveServer,
    );

    let bar_spans = vec![
        Span::styled(
            " [+ Add] (a) ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [✎ Edit] (e) ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [❐ Clone] (c) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [⏸ Active] (v) ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [⚡ SSH] (t) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [🔍 Probe] (p) ",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [💽 Health] (d) ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [🗑 Del] (x) ",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let sub_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    let list_block = Block::default()
        .borders(Borders::ALL)
        .title(" Multi-Node Latency & Fleet Matrix ")
        .border_style(Style::default().fg(palette.border));
    let list_inner = list_block.inner(sub_chunks[0]);
    f.render_widget(list_block, sub_chunks[0]);

    let mut rows = Vec::new();
    for (i, (node_name, lat)) in app.server_probe_results.iter().enumerate() {
        let is_sel = i == app.selected_server;
        let bg = if is_sel {
            palette.row_selected_bg
        } else {
            Color::Reset
        };
        let is_active = app
            .cfg
            .daemon
            .nodes
            .get(node_name)
            .map(|n| n.active)
            .unwrap_or(true);

        let (lat_str, status_str, status_col) = if !is_active {
            ("--".to_string(), "◌ INACTIVE", palette.text_muted)
        } else {
            match lat {
                Some(ms) => (format!("{ms}ms"), "● REACHABLE", palette.success),
                None => ("--".to_string(), "✖ UNREACHABLE", palette.danger),
            }
        };
        rows.push(
            Row::new(vec![
                Cell::from(node_name.clone()).style(
                    Style::default()
                        .fg(if is_active {
                            Color::White
                        } else {
                            palette.text_muted
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(status_str)
                    .style(Style::default().fg(status_col).add_modifier(Modifier::BOLD)),
                Cell::from(lat_str).style(Style::default().fg(palette.accent)),
            ])
            .style(Style::default().bg(bg)),
        );
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(16),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(vec!["SERVER NODE", "STATUS", "RTT LATENCY"]).style(
            Style::default()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ),
    );
    f.render_widget(table, list_inner);

    for i in 0..app.server_probe_results.len() {
        if (i as u16) < list_inner.height.saturating_sub(1) {
            app.register_hitbox(
                Rect {
                    x: list_inner.x,
                    y: list_inner.y + 1 + (i as u16),
                    width: list_inner.width,
                    height: 1,
                },
                HitboxAction::SelectServer(i),
            );
        }
    }

    let sel_name = app
        .server_probe_results
        .get(app.selected_server)
        .map(|(n, _)| n.as_str())
        .unwrap_or("ThinkPad X1");
    let detail_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Node Resources — {} ", sel_name))
        .border_style(Style::default().fg(palette.accent));
    let detail_inner = detail_block.inner(sub_chunks[1]);
    f.render_widget(detail_block, sub_chunks[1]);

    let snap = &app.latest_snap;
    let mut details = Vec::new();
    if sel_name.contains("X1") || sel_name.contains("Local") {
        details.push(Line::from(vec![
            Span::styled("Hardware:     ", Style::default().fg(palette.text_muted)),
            Span::styled(
                "ThinkPad X1 Carbon (Intel 8-Core / Btrfs NVMe)",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        details.push(Line::from(vec![
            Span::styled("CPU Cores:    ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!(
                    "{} cores │ Usage: {:.1}% │ Temp: {:.1}°C",
                    snap.cpus.len(),
                    snap.cpu_pct,
                    snap.cpu_temp_c
                ),
                Style::default().fg(palette.accent),
            ),
        ]));
        details.push(Line::from(vec![
            Span::styled("Memory (RAM): ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!(
                    "{}/{} MB (Cached: {} MB, Free: {} MB)",
                    snap.ram_used_mb, snap.ram_total_mb, snap.ram_cached_mb, snap.ram_available_mb
                ),
                Style::default().fg(palette.chart[0]),
            ),
        ]));
        details.push(Line::from(vec![
            Span::styled("Storage/NVMe: ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("Root /: {}G / {}G", snap.disk_used_gb, snap.disk_total_gb),
                Style::default().fg(palette.success),
            ),
        ]));
        if let Some(disk) = snap.disk_health.first() {
            let health_color =
                if disk.health_status.contains("HEALTHY") || disk.health_status.contains("GOOD") {
                    palette.success
                } else if disk.health_status.contains("WARN") {
                    palette.warning
                } else {
                    palette.danger
                };
            let temp_color = if disk.temperature_c > 75.0 {
                palette.danger
            } else if disk.temperature_c > 60.0 {
                palette.warning
            } else {
                palette.success
            };
            details.push(Line::from(vec![
                Span::styled("SSD SMART:    ", Style::default().fg(palette.text_muted)),
                Span::styled(
                    format!("[● {}] ", disk.health_status),
                    Style::default()
                        .fg(health_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("[● {:.0}°C NORMAL] ", disk.temperature_c),
                    Style::default().fg(temp_color),
                ),
                Span::styled(
                    format!(
                        "TBW: {:.2} TB (Wear: {}%)",
                        disk.written_bytes_tb, disk.wear_level_pct
                    ),
                    Style::default().fg(palette.accent),
                ),
            ]));
        }
        details.push(Line::from(vec![
            Span::styled("Thermal State:", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!(
                    "{:.1}°C (Fan: {})",
                    snap.cpu_temp_c,
                    snap.fan_speed.as_deref().unwrap_or("Dynamic")
                ),
                Style::default().fg(if snap.cpu_temp_c > 85.0 {
                    palette.danger
                } else {
                    palette.success
                }),
            ),
        ]));
        details.push(Line::from(""));
        details.push(Line::from(Span::styled(
            "Per-Core Telemetry Grid:",
            Style::default()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        )));
        for row in 0..4 {
            let left_spans = format_core_span(snap.cpus.get(row), row, 6);
            let right_spans = format_core_span(snap.cpus.get(row + 4), row + 4, 6);
            let mut l = left_spans;
            l.push(Span::styled(" │ ", Style::default().fg(palette.text_muted)));
            l.extend(right_spans);
            details.push(Line::from(l));
        }
    } else {
        let node_cfg = app.cfg.daemon.nodes.get(sel_name);
        let ip = node_cfg
            .map(|n| n.ip.as_str())
            .unwrap_or("100.x.y.z (Tailscale Mesh)");
        let user = node_cfg.map(|n| n.user.as_str()).unwrap_or("root");
        let svcs = node_cfg
            .map(|n| {
                n.services
                    .iter()
                    .map(|s| format!("{}:{}", s.name, s.port))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "ssh:22, http:80".to_string());
        let active_state = node_cfg.map(|n| n.active).unwrap_or(true);

        details.push(Line::from(vec![
            Span::styled("Node ID:       ", Style::default().fg(palette.text_muted)),
            Span::styled(
                sel_name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        details.push(Line::from(vec![
            Span::styled("Tailscale IP:  ", Style::default().fg(palette.text_muted)),
            Span::styled(ip, Style::default().fg(palette.accent)),
        ]));
        details.push(Line::from(vec![
            Span::styled("SSH Username:  ", Style::default().fg(palette.text_muted)),
            Span::styled(user, Style::default().fg(Color::White)),
        ]));
        details.push(Line::from(vec![
            Span::styled("Services:      ", Style::default().fg(palette.text_muted)),
            Span::styled(svcs.clone(), Style::default().fg(palette.warning)),
        ]));
        details.push(Line::from(vec![
            Span::styled("Node State:    ", Style::default().fg(palette.text_muted)),
            Span::styled(
                if active_state {
                    "● ACTIVE (Probing & Armada Enabled)"
                } else {
                    "◌ INACTIVE (Probes Disabled)"
                },
                Style::default()
                    .fg(if active_state {
                        palette.success
                    } else {
                        palette.text_muted
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        details.push(Line::from(""));
        details.push(Line::from(Span::styled(
            "Management Controls: [e] Edit Node · [c] Clone Node · [v] Toggle Active · [t] Test SSH · [x] Remove",
            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD),
        )));
    }

    f.render_widget(Paragraph::new(details), detail_inner);
}

pub fn draw_server_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 52, area);
    f.render_widget(Clear, popup_area);

    let is_edit = app.server_wizard_is_edit;
    let orig_name = app.server_wizard_orig_name.clone();
    let wizard_title = if is_edit {
        format!(" ✎ Edit Fleet Server — Node Manager ({orig_name})")
    } else {
        " 🖥 Add Fleet Server — Node Wizard".to_string()
    };

    let block = Block::default()
        .title(wizard_title)
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(if is_edit {
                    palette.warning
                } else {
                    palette.accent
                })
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let fields = [
        (
            "Server Name:",
            app.server_name_buf.clone(),
            "e.g. vps-sg, worker-1",
        ),
        (
            "Host / IP Address:",
            app.server_ip_buf.clone(),
            "e.g. 100.127.232.32",
        ),
        (
            "SSH Username:",
            app.server_user_buf.clone(),
            "e.g. root, cecep",
        ),
        (
            "Services (CSV):",
            app.server_services_buf.clone(),
            "e.g. http:80,ssh:22",
        ),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        if is_edit {
            "Edit the fleet node IP, SSH user, and exposed services. Renaming the key is supported."
        } else {
            "Register a fleet node with IP & services. Auto-probes upon saving."
        },
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.server_wizard_field == i;
        let prefix = if is_focused { "▶ " } else { "  " };
        let val_span = if is_focused {
            Span::styled(
                format!(" [ {}█ ] ", val),
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            let display_val = if val.is_empty() {
                *placeholder
            } else {
                val.as_str()
            };
            Span::styled(
                format!(" [ {} ] ", display_val),
                Style::default().fg(if val.is_empty() {
                    Color::DarkGray
                } else {
                    Color::White
                }),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(
                prefix,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<20}", label),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            val_span,
        ]));

        let row_y = field_start_y + (i as u16);
        if row_y < inner.bottom() {
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
                HitboxAction::FocusServerField(i),
            );
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab/Down: Next │ BackTab/Up: Prev │ Enter/Ctrl+S: Save │ Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let btn_save_str = if is_edit {
        " [✔ Save Changes] (Enter) "
    } else {
        " [✔ Add Server] (Enter) "
    };
    let btn_cancel_str = " [✖ Cancel] (Esc) ";

    let btn_save = Span::styled(
        btn_save_str,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let btn_cancel = Span::styled(
        btn_cancel_str,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    );

    let btn_y = inner.bottom().saturating_sub(2);
    let save_width = btn_save_str.chars().count() as u16;
    let cancel_width = btn_cancel_str.chars().count() as u16;
    let cancel_x = inner.x + save_width + 2;

    if btn_y < inner.bottom() && btn_y >= inner.y {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: save_width,
                height: 1,
            },
            HitboxAction::SubmitServer,
        );
        app.register_hitbox(
            Rect {
                x: cancel_x,
                y: btn_y,
                width: cancel_width,
                height: 1,
            },
            HitboxAction::CancelServer,
        );
    }

    f.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        },
    );
    if btn_y < inner.bottom() && btn_y >= inner.y {
        f.render_widget(
            Paragraph::new(Line::from(vec![btn_save, Span::raw("  "), btn_cancel])),
            Rect {
                x: inner.x,
                y: btn_y,
                width: inner.width,
                height: 1,
            },
        );
    }
}
