//! Uptime Monitoring Grid & Modals (HTTP/HTTPS, TCP, TLS) — extracted from cockpit.rs (F-07).

use super::{cockpit_theme, CockpitApp, HitboxAction};
use catui::layout::centered_rect;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
    Frame,
};

pub fn draw_uptime_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(8)])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let displayed: Vec<(usize, crate::systat::UptimeMonitor)> = app
        .latest_snap
        .uptime_monitors
        .iter()
        .cloned()
        .enumerate()
        .collect();
    let count = displayed.len();
    if !displayed.is_empty() && app.selected_monitor >= count {
        app.selected_monitor = count - 1;
    }

    // ── Toolbar row 1: Core Actions (Add, Probe, Edit, Delete, Group, Sort) ──
    let alert_on = app.cfg.uptime.telegram_alerts_enabled;
    let mut toolbar_spans: Vec<Span> = vec![
        Span::styled(
            " [ADD] (m) ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            " [PROBE] (p) ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            " [EDIT] (e) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            " [DEL] (x) ",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  │  "),
        Span::styled(
            format!(" [GRP:{}] ", app.uptime_group_filter),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(" [SRT:{}] ", app.uptime_sort_mode.label_short()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    let up_count = app
        .latest_snap
        .uptime_monitors
        .iter()
        .filter(|m| m.is_up)
        .count();
    let total_count = app.latest_snap.uptime_monitors.len();
    let pct = if total_count > 0 {
        (up_count as f32 / total_count as f32) * 100.0
    } else {
        100.0
    };
    toolbar_spans.push(Span::raw("  │  "));
    toolbar_spans.push(Span::styled(
        format!("Health: {up_count}/{total_count} Up ({pct:.1}%)"),
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(Paragraph::new(Line::from(toolbar_spans)), bar_inner);

    // Register toolbar hitboxes with cumulative x-offset
    let bar_y = bar_inner.y;
    let mut cur_x = bar_inner.x + 1;
    let widths: [u16; 6] = [12, 14, 12, 12, 14, 14];
    let actions: [HitboxAction; 6] = [
        HitboxAction::AddMonitor,
        HitboxAction::ProbeMonitors,
        HitboxAction::EditMonitor,
        HitboxAction::RemoveMonitor,
        HitboxAction::CycleUptimeGroup,
        HitboxAction::CycleUptimeSort,
    ];
    for (w, act) in widths.iter().zip(actions.iter()) {
        if cur_x + w < bar_inner.x + bar_inner.width {
            app.register_hitbox(
                Rect {
                    x: cur_x,
                    y: bar_y,
                    width: *w,
                    height: 1,
                },
                act.clone(),
            );
        }
        cur_x = cur_x.saturating_add(*w).saturating_add(2);
    }

    // ── Toolbar row 2: Advanced Actions (Export, Import, Alerts, Detail) ──
    let adv_y = bar_y + 1;
    if adv_y < bar_inner.bottom() {
        let adv_spans: Vec<Span> = vec![
            Span::styled(
                " [EXPORT] ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                " [IMPORT] ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                if alert_on {
                    " [ALERTS:ON] "
                } else {
                    " [ALERTS:OFF] "
                },
                Style::default()
                    .fg(if alert_on {
                        Color::Green
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                " [TEST TG ALERT] ",
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  │  "),
            Span::styled(
                " [V] View Detail (Enter) ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        f.render_widget(
            Paragraph::new(Line::from(adv_spans)),
            Rect {
                x: bar_inner.x,
                y: adv_y,
                width: bar_inner.width,
                height: 1,
            },
        );

        let mut ax = bar_inner.x + 1;
        for (label_w, act) in [
            (10, HitboxAction::ExportUptimeJson),
            (10, HitboxAction::ImportUptimeJson),
            (15, HitboxAction::ToggleMonitorAlerts),
            (17, HitboxAction::TestTelegramAlert),
            (24, HitboxAction::ViewMonitorDetail(app.selected_monitor)),
        ] {
            if ax + label_w < bar_inner.x + bar_inner.width {
                app.register_hitbox(
                    Rect {
                        x: ax,
                        y: adv_y,
                        width: label_w,
                        height: 1,
                    },
                    act,
                );
            }
            ax = ax.saturating_add(label_w).saturating_add(2);
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " 🌐 Uptime Monitors │ Filter: {} │ Sort: {} │ [{}]",
            app.uptime_group_filter,
            app.uptime_sort_mode.label(),
            if alert_on { "ALERTS:ON" } else { "ALERTS:OFF" }
        ))
        .border_style(Style::default().fg(palette.border));
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);

    let rows: Vec<Row> = displayed
        .iter()
        .enumerate()
        .map(|(i, (_orig_idx, m))| {
            let is_sel = i == app.selected_monitor;
            let bg = if is_sel {
                palette.row_selected_bg
            } else {
                Color::Reset
            };
            let (status_badge, badge_color) = if m.is_up {
                (format!("● UP ({:.1}%)", m.uptime_pct), palette.success)
            } else {
                (format!("✖ DOWN ({:.1}%)", m.uptime_pct), palette.danger)
            };

            let latency_str = m
                .latency_ms
                .map(|l| format!("{l}ms"))
                .unwrap_or_else(|| "--".to_string());
            let code_str = match m.status_code {
                Some(c) if (200..=299).contains(&c) => format!("{c} OK"),
                Some(c) if (300..=399).contains(&c) => format!("{c} REDIR"),
                Some(401) => "401 AUTH".to_string(),
                Some(403) => "403 FORBID".to_string(),
                Some(404) if m.is_up => "404 UP".to_string(),
                Some(404) => "404 NOT FOUND".to_string(),
                Some(c) if (500..=599).contains(&c) => format!("{c} ERR"),
                Some(c) if m.is_up => format!("{c} UP"),
                Some(c) => format!("{c} ERR"),
                None => {
                    if let Some(ref err) = m.error {
                        format!("ERR: {err}")
                    } else if !m.is_up {
                        "ERR: Down".to_string()
                    } else {
                        "--".to_string()
                    }
                }
            };
            let code_color = if !m.is_up {
                palette.danger
            } else if m
                .status_code
                .map(|c| (200..=299).contains(&c))
                .unwrap_or(false)
            {
                palette.chart[0]
            } else {
                palette.warning
            };

            let mut history_spans: Vec<Span> = Vec::with_capacity(20);
            let pad_needed = 20usize.saturating_sub(m.history.len());
            for _ in 0..pad_needed {
                history_spans.push(Span::styled(
                    "█",
                    Style::default().fg(Color::Rgb(60, 60, 60)),
                ));
            }
            for &up in m.history.iter().take(20) {
                let col = if up {
                    if let Some(ms) = m.latency_ms {
                        if ms > 500 {
                            Color::Yellow
                        } else {
                            Color::Green
                        }
                    } else {
                        Color::Green
                    }
                } else {
                    Color::Red
                };
                history_spans.push(Span::styled("█", Style::default().fg(col)));
            }

            let eye_span = Span::styled(
                "[V]",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            );

            Row::new(vec![
                Cell::from(format!("{i:>3}")).style(
                    Style::default()
                        .fg(palette.text_muted)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(m.name.clone()).style(
                    Style::default()
                        .fg(if !m.is_up {
                            Color::LightRed
                        } else {
                            Color::White
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(m.target.clone()).style(Style::default().fg(Color::Cyan)),
                Cell::from(m.group.clone()).style(Style::default().fg(palette.text_muted)),
                Cell::from(status_badge).style(
                    Style::default()
                        .fg(badge_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(latency_str).style(Style::default().fg(palette.accent)),
                Cell::from(Line::from(history_spans)),
                Cell::from(code_str)
                    .style(Style::default().fg(code_color).add_modifier(Modifier::BOLD)),
                Cell::from(Line::from(vec![eye_span])),
            ])
            .style(Style::default().bg(bg))
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(18),
            Constraint::Min(26),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Length(9),
            Constraint::Length(22),
            Constraint::Length(14),
            Constraint::Length(4),
        ],
    )
    .header(
        Row::new(vec![
            "#",
            "MONITOR",
            "TARGET URL",
            "GROUP",
            "STATUS",
            "LATENCY",
            "HEARTBEATS (20)",
            "HTTP / ERR",
            "⊕",
        ])
        .style(
            Style::default()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ),
    );
    f.render_widget(table, inner);

    // ── Mouse hitboxes for table rows and Detail buttons ──
    for (i, (orig_idx, _)) in displayed.iter().enumerate() {
        if (i as u16) < inner.height.saturating_sub(1) {
            let row_y = inner.y + 1 + (i as u16);
            let row_action = if i == app.selected_monitor {
                HitboxAction::ViewMonitorDetail(*orig_idx)
            } else {
                HitboxAction::SelectMonitor(i)
            };
            app.register_hitbox(
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
                row_action,
            );
        }
    }
}

pub fn draw_uptime_detail_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let popup_area = centered_rect(68, 60, area);
    f.render_widget(Clear, popup_area);

    let orig_idx = app.uptime_detail_index;
    let monitor = match app.latest_snap.uptime_monitors.get(orig_idx) {
        Some(m) => m,
        None => {
            app.show_uptime_detail_modal = false;
            return;
        }
    };

    let title = format!(" 📈 Endpoint Detail — {} ", monitor.name);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let modal_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(12),   // Detail content
            Constraint::Length(3), // Bottom Action Toolbar
        ])
        .split(inner);

    let mut history_spans: Vec<Span> = Vec::with_capacity(20);
    let pad_needed = 20usize.saturating_sub(monitor.history.len());
    for _ in 0..pad_needed {
        history_spans.push(Span::styled(
            "█",
            Style::default().fg(Color::Rgb(60, 60, 60)),
        ));
    }
    for &up in monitor.history.iter().take(20) {
        let col = if up {
            if let Some(ms) = monitor.latency_ms {
                if ms > 500 {
                    Color::Yellow
                } else {
                    Color::Green
                }
            } else {
                Color::Green
            }
        } else {
            Color::Red
        };
        history_spans.push(Span::styled("█", Style::default().fg(col)));
    }

    let status_label = if monitor.is_up {
        Span::styled(
            "● UP (Healthy)",
            Style::default()
                .fg(palette.success)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "✖ DOWN (Unreachable)",
            Style::default()
                .fg(palette.danger)
                .add_modifier(Modifier::BOLD),
        )
    };

    let latency_span = match monitor.latency_ms {
        Some(l) => Span::styled(
            format!("{l} ms"),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        None => Span::styled("--".to_string(), Style::default().fg(Color::DarkGray)),
    };

    let code_span = match monitor.status_code {
        Some(c) => {
            let col = if monitor.is_up {
                palette.success
            } else {
                palette.danger
            };
            Span::styled(
                c.to_string(),
                Style::default().fg(col).add_modifier(Modifier::BOLD),
            )
        }
        None => {
            if let Some(ref err) = monitor.error {
                Span::styled(
                    format!("ERR: {err}"),
                    Style::default()
                        .fg(palette.danger)
                        .add_modifier(Modifier::BOLD),
                )
            } else if !monitor.is_up {
                Span::styled(
                    "ERR: Down".to_string(),
                    Style::default()
                        .fg(palette.danger)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled("--".to_string(), Style::default().fg(Color::DarkGray))
            }
        }
    };

    let alert_state = if app.cfg.uptime.telegram_alerts_enabled {
        Span::styled(
            "● ENABLED",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("○ DISABLED", Style::default().fg(Color::DarkGray))
    };

    let has_token = !app.cfg.uptime.telegram_bot_token.is_empty()
        || !app.cfg.daemon.telegram_bot_token.is_empty();
    let token_status = if has_token {
        Span::styled(" [✓ Token Configured]", Style::default().fg(Color::Green))
    } else {
        Span::styled(
            " [⚠ NO BOT TOKEN SET — open Settings (-) tab]",
            Style::default().fg(Color::Yellow),
        )
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("Endpoint:     ", Style::default().fg(palette.text_muted)),
            Span::styled(
                monitor.name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Target URL:   ", Style::default().fg(palette.text_muted)),
            Span::styled(
                monitor.target.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Group:        ", Style::default().fg(palette.text_muted)),
            Span::styled(monitor.group.clone(), Style::default().fg(palette.accent)),
        ]),
        Line::from(vec![
            Span::styled("Status:       ", Style::default().fg(palette.text_muted)),
            status_label,
            Span::raw("    "),
            Span::styled("Latency: ", Style::default().fg(palette.text_muted)),
            latency_span,
            Span::raw("    "),
            Span::styled("HTTP Code: ", Style::default().fg(palette.text_muted)),
            code_span,
        ]),
    ];

    if let Some(ref err) = monitor.error {
        if !monitor.is_up {
            lines.push(Line::from(vec![
                Span::styled(
                    "Failure Cause: ",
                    Style::default()
                        .fg(palette.danger)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("ERR: {err}"), Style::default().fg(Color::LightRed)),
            ]));
        }
    }

    lines.extend([
        Line::from(vec![
            Span::styled("Uptime Rate:  ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("{:.1}%", monitor.uptime_pct),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Heartbeats (Last 20 Probes): ",
            Style::default()
                .fg(palette.text_muted)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(history_spans),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Telegram Alerts:  ",
                Style::default().fg(palette.text_muted),
            ),
            alert_state,
            token_status,
        ]),
    ]);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }),
        modal_chunks[0],
    );

    // Bottom Action Button Toolbar inside modal
    let btn_bar_block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(palette.border));
    let btn_bar_inner = btn_bar_block.inner(modal_chunks[1]);
    f.render_widget(btn_bar_block, modal_chunks[1]);

    let btn_spans = vec![
        Span::styled(
            " [ ↻ Probe (p) ] ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            if app.cfg.uptime.telegram_alerts_enabled {
                " [ 🔔 Alerts: ON (t) ] "
            } else {
                " [ 🔕 Alerts: OFF (t) ] "
            },
            Style::default()
                .fg(if app.cfg.uptime.telegram_alerts_enabled {
                    Color::Green
                } else {
                    Color::DarkGray
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [ ✏ Edit (e) ] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [ 🗑 Delete (x) ] ",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [ ✖ Close (Esc) ] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    f.render_widget(Paragraph::new(Line::from(btn_spans)), btn_bar_inner);

    let btn_y = btn_bar_inner.y;
    let mut bx = btn_bar_inner.x;
    for (bw, act) in [
        (17u16, HitboxAction::ProbeMonitors),
        (22u16, HitboxAction::ToggleMonitorAlerts),
        (16u16, HitboxAction::EditMonitor),
        (18u16, HitboxAction::RemoveMonitor),
        (18u16, HitboxAction::CloseMonitorDetail),
    ] {
        if bx + bw <= btn_bar_inner.right() {
            app.register_hitbox(
                Rect {
                    x: bx,
                    y: btn_y,
                    width: bw,
                    height: 1,
                },
                act,
            );
        }
        bx = bx.saturating_add(bw).saturating_add(1);
    }
}

// ── Tab 4: Server (Multi-Node Server Resources & Latency Matrix) ─────────────

pub fn draw_monitor_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 42, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🌐 Add Uptime Monitor — Endpoint Wizard ")
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 4 {
        return;
    }

    let fields = [
        (
            "Monitor Name:",
            app.monitor_name_buf.clone(),
            "e.g. Payment Gateway",
        ),
        (
            "Target URL:",
            app.monitor_url_buf.clone(),
            "e.g. https://api.fathforce.com/health",
        ),
        (
            "Group:",
            app.monitor_group_buf.clone(),
            "e.g. CA Ecosystem, Production, Custom",
        ),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Add an HTTP/HTTPS or TCP endpoint to Uptime Kuma monitoring.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.monitor_wizard_field == i;
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
                format!("{:<18}", label),
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
                HitboxAction::FocusMonitorField(i),
            );
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab/Down: Next │ BackTab/Up: Prev │ Enter: Save │ Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let btn_save = Span::styled(
        " [✔ Add Monitor] (Enter) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let btn_cancel = Span::styled(
        " [✖ Cancel] (Esc) ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    );
    lines.push(Line::from(vec![btn_save, Span::raw("  "), btn_cancel]));

    f.render_widget(Paragraph::new(lines), inner);

    let btn_y = inner.y + 6;
    if btn_y < inner.bottom() {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: 25,
                height: 1,
            },
            HitboxAction::SubmitMonitor,
        );
        app.register_hitbox(
            Rect {
                x: inner.x + 27,
                y: btn_y,
                width: 18,
                height: 1,
            },
            HitboxAction::CancelMonitor,
        );
    }
}

// ── Quit Confirmation Modal ──────────────────────────────────────────────────
