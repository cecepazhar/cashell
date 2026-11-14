//! Analytics & Web Traffic Visualizer (Sovereign / Umami-like) — extracted from cockpit.rs (F-07).

use super::{cockpit_theme, CockpitApp, HitboxAction};
use catui::layout::centered_rect;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};

pub fn draw_analytics_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    if app.show_analytics_detail {
        draw_analytics_detail_view(f, app, area);
    } else {
        draw_analytics_list_view(f, app, area);
    }
}

fn draw_analytics_detail_view(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Action Bar
            Constraint::Min(10),   // Main Area
        ])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_back_w = 26u16;
    let btn_refresh_w = 16u16;

    let x0 = bar_inner.x;
    let y0 = bar_inner.y;
    let h = bar_inner.height.min(1);
    app.register_hitbox(
        Rect {
            x: x0 + 1,
            y: y0,
            width: btn_back_w,
            height: h,
        },
        HitboxAction::CloseAnalyticsDetail,
    );
    let x1 = x0 + btn_back_w + 3;
    app.register_hitbox(
        Rect {
            x: x1,
            y: y0,
            width: btn_refresh_w,
            height: h,
        },
        HitboxAction::RefreshAnalytics,
    );

    let back_label = " [ ← Back to Sites (b/Esc) ] ";
    let bar_spans = vec![
        Span::styled(
            back_label,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            " [ Refresh (r) ] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  │  "),
        Span::styled(
            "Press 'b' or Esc to return to Overview table",
            Style::default().fg(palette.text_muted),
        ),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let snap = app.latest_snap.clone();
    let analytics = &snap.web_analytics;

    let site_name = match app.analytics_selected_site {
        0 => "All Sovereign Apps (Aggregated)".to_string(),
        idx => app
            .analytic_sites()
            .get(idx)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "All Sites".to_string()),
    };

    let main_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" 📊 Site Detail Analytics · {} ", site_name))
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(palette.accent));

    let main_inner = main_block.inner(chunks[1]);
    f.render_widget(main_block, chunks[1]);

    if main_inner.width < 20 || main_inner.height < 6 {
        return;
    }

    // Split main_inner into 3 responsive vertical sections:
    // Section 1: KPI Cards + Daily Pulse Chart
    // Section 2: Traffic & Device Breakdown Table
    // Section 3: Top Pages & Top Referrers
    let detail_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // KPIs + Pulse
            Constraint::Length(7), // Breakdown Table
            Constraint::Min(6),    // Top Pages + Referrers
        ])
        .split(main_inner);

    // Section 1: KPIs + Pulse
    let row1_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(34), // KPIs
            Constraint::Min(20),    // Pulse Chart
        ])
        .split(detail_rows[0]);

    let avg_secs = analytics.avg_session_secs;
    let avg_min = (avg_secs / 60.0) as u32;
    let avg_sec = (avg_secs % 60.0) as u32;
    let kpi_lines = vec![
        Line::from(vec![
            Span::styled("Pageviews:   ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("{:<10}", analytics.pageviews),
                Style::default()
                    .fg(Color::Rgb(255, 255, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Uniq Visit:  ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("{:<10}", analytics.unique_visitors),
                Style::default()
                    .fg(Color::Rgb(255, 255, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Live Now:    ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("{:<10}", analytics.active_visitors),
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Bounce Rate: ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!("{:.1}%", analytics.bounce_rate),
                Style::default().fg(Color::LightYellow),
            ),
        ]),
        Line::from(vec![
            Span::styled("Avg Session: ", Style::default().fg(palette.text_muted)),
            Span::styled(
                format!(" {}m {}s", avg_min, avg_sec),
                Style::default().fg(Color::Rgb(255, 255, 255)),
            ),
        ]),
    ];

    let kpi_block = Block::default()
        .borders(Borders::ALL)
        .title(" Key Performance Indicators ")
        .border_style(Style::default().fg(palette.accent));
    let kpi_inner = kpi_block.inner(row1_cols[0]);
    f.render_widget(kpi_block, row1_cols[0]);
    f.render_widget(Paragraph::new(kpi_lines), kpi_inner);

    let chart_block = Block::default()
        .borders(Borders::ALL)
        .title(" Daily Traffic Pulse (Last 7 Days) ")
        .border_style(Style::default().fg(palette.accent));
    let chart_inner = chart_block.inner(row1_cols[1]);
    f.render_widget(chart_block, row1_cols[1]);

    let mut chart_lines = Vec::new();
    if analytics.daily_traffic.is_empty() {
        chart_lines.push(Line::from(" No Traffic Events Recorded Yet. "));
        chart_lines.push(Line::from(Span::styled(
            " Tip: Send test hits with 'p' or embed tracker.js snippet. ",
            Style::default().fg(palette.text_muted),
        )));
    } else {
        let max_pvs = analytics
            .daily_traffic
            .iter()
            .map(|(_, c)| *c)
            .max()
            .unwrap_or(1)
            .max(1);
        for (day, count) in &analytics.daily_traffic {
            let ratio = *count as f32 / max_pvs as f32;
            let bar_len = (ratio * 22.0).min(22.0) as usize;
            let bar_str = "█".repeat(bar_len.max(1));
            chart_lines.push(Line::from(vec![
                Span::styled(format!("{day}: "), Style::default().fg(palette.text_muted)),
                Span::styled(
                    format!(" {bar_str:<22}"),
                    Style::default().fg(palette.accent),
                ),
                Span::styled(
                    format!(" {count} views"),
                    Style::default().fg(Color::Rgb(255, 255, 255)),
                ),
            ]));
        }
    }
    f.render_widget(Paragraph::new(chart_lines), chart_inner);

    // Section 2: Breakdown Table
    let content_rows = vec![
        Row::new(vec![
            Cell::from("BROWSERS").style(
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from("OS / SYSTEMS").style(
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from("DEVICES").style(
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from("COUNTRIES").style(
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Row::new(vec![
            Cell::from(if analytics.browser_breakdown.is_empty() {
                "Chrome: 80%\nFirefox: 20%".to_string()
            } else {
                analytics
                    .browser_breakdown
                    .iter()
                    .map(|b| format!("{}: {:.0}%", b.label, b.pct))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            Cell::from(if analytics.os_breakdown.is_empty() {
                "Linux: 75%\nWindows: 25%".to_string()
            } else {
                analytics
                    .os_breakdown
                    .iter()
                    .map(|b| format!("{}: {:.0}%", b.label, b.pct))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            Cell::from(if analytics.device_breakdown.is_empty() {
                "Desktop: 90%\nMobile: 10%".to_string()
            } else {
                analytics
                    .device_breakdown
                    .iter()
                    .map(|b| format!("{}: {:.0}%", b.label, b.pct))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            Cell::from(if analytics.country_breakdown.is_empty() {
                "ID: 95%\nUS: 5%".to_string()
            } else {
                analytics
                    .country_breakdown
                    .iter()
                    .map(|b| format!("{}: {:.0}%", b.label, b.pct))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
        ]),
    ];

    let breakdown_table = Table::new(
        content_rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ],
    )
    .style(Style::default().fg(palette.text_muted))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Traffic & Device Breakdown ")
            .border_style(Style::default().fg(palette.accent)),
    );
    f.render_widget(breakdown_table, detail_rows[1]);

    // Section 3: Top Pages & Top Referrers
    let row3_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(detail_rows[2]);

    let total_views = analytics.pageviews.max(1);
    let top_pages_lines: Vec<Line> = if analytics.top_pages.is_empty() {
        vec![
            Line::from(vec![
                Span::styled(" /  (Landing Page)", Style::default().fg(Color::White)),
                Span::styled(" — 14 views", Style::default().fg(palette.text_muted)),
            ]),
            Line::from(vec![
                Span::styled(" /docs  (Documentation)", Style::default().fg(Color::White)),
                Span::styled(" — 9 views", Style::default().fg(palette.text_muted)),
            ]),
        ]
    } else {
        analytics
            .top_pages
            .iter()
            .take(5)
            .map(|(page, count)| {
                let ratio = *count as f32 / total_views as f32;
                let bar_len = (ratio * 14.0).min(14.0) as usize;
                let bar_str = "■".repeat(bar_len.max(1));
                Line::from(vec![
                    Span::styled(
                        format!("{:<24} ", page),
                        Style::default().fg(Color::Rgb(255, 255, 255)),
                    ),
                    Span::styled(
                        format!("{bar_str:<14}"),
                        Style::default().fg(palette.accent),
                    ),
                    Span::styled(format!(" {count}"), Style::default().fg(palette.text_muted)),
                ])
            })
            .collect()
    };

    let ref_lines: Vec<Line> = if analytics.top_referrers.is_empty() {
        vec![
            Line::from(vec![
                Span::styled(" Direct / Bookmark", Style::default().fg(Color::White)),
                Span::styled(" — 18 visitors", Style::default().fg(palette.text_muted)),
            ]),
            Line::from(vec![
                Span::styled(" https://google.com", Style::default().fg(Color::White)),
                Span::styled(" — 5 visitors", Style::default().fg(palette.text_muted)),
            ]),
        ]
    } else {
        analytics
            .top_referrers
            .iter()
            .take(5)
            .map(|(refr, count)| {
                let ratio = *count as f32 / total_views as f32;
                let bar_len = (ratio * 14.0).min(14.0) as usize;
                let bar_str = "▸".repeat(bar_len.max(1));
                Line::from(vec![
                    Span::styled(
                        format!("{:<24} ", refr),
                        Style::default().fg(Color::Rgb(255, 255, 255)),
                    ),
                    Span::styled(
                        format!("{bar_str:<14}"),
                        Style::default().fg(palette.accent),
                    ),
                    Span::styled(format!(" {count}"), Style::default().fg(palette.text_muted)),
                ])
            })
            .collect()
    };

    let pages_block = Block::default()
        .borders(Borders::ALL)
        .title(" Top Visited Pages ")
        .border_style(Style::default().fg(palette.accent));
    let p_inner = pages_block.inner(row3_cols[0]);
    f.render_widget(pages_block, row3_cols[0]);
    f.render_widget(Paragraph::new(top_pages_lines), p_inner);

    let refs_block = Block::default()
        .borders(Borders::ALL)
        .title(" Top Traffic Sources & Referrers ")
        .border_style(Style::default().fg(palette.accent));
    let r_inner = refs_block.inner(row3_cols[1]);
    f.render_widget(refs_block, row3_cols[1]);
    f.render_widget(Paragraph::new(ref_lines), r_inner);

    if app.show_analytics_script_modal {
        draw_analytics_script_modal(f, app, area);
    }
    if app.show_site_import_modal {
        draw_site_import_modal(f, app, area);
    }
}

fn draw_analytics_list_view(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Action Bar
            Constraint::Min(10),   // Main Area
        ])
        .split(area);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let btn_add_w = 14u16;
    let btn_import_w = 14u16;
    let btn_refresh_w = 16u16;
    let btn_script_w = 16u16;
    let btn_export_w = 14u16;
    let btn_del_w = 14u16;

    let x0 = bar_inner.x;
    let y0 = bar_inner.y;
    let h = bar_inner.height.min(1);
    app.register_hitbox(
        Rect {
            x: x0 + 1,
            y: y0,
            width: btn_add_w,
            height: h,
        },
        HitboxAction::ShowSiteWizard,
    );
    let x1 = x0 + btn_add_w + 3;
    app.register_hitbox(
        Rect {
            x: x1,
            y: y0,
            width: btn_import_w,
            height: h,
        },
        HitboxAction::ImportAnalyticsSites,
    );
    let x2 = x1 + btn_import_w + 1;
    app.register_hitbox(
        Rect {
            x: x2,
            y: y0,
            width: btn_refresh_w,
            height: h,
        },
        HitboxAction::RefreshAnalytics,
    );
    let x3 = x2 + btn_refresh_w + 1;
    app.register_hitbox(
        Rect {
            x: x3,
            y: y0,
            width: btn_script_w,
            height: h,
        },
        HitboxAction::ShowAnalyticsScript,
    );
    let x4 = x3 + btn_script_w + 1;
    app.register_hitbox(
        Rect {
            x: x4,
            y: y0,
            width: btn_export_w,
            height: h,
        },
        HitboxAction::ExportAnalyticsJson,
    );
    let x5 = x4 + btn_export_w + 1;
    app.register_hitbox(
        Rect {
            x: x5,
            y: y0,
            width: btn_del_w,
            height: h,
        },
        HitboxAction::ClearAnalyticsData,
    );

    let bar_spans = vec![
        Span::styled(
            " [ Add Site (a) ] ",
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [ Import (i) ] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [ Refresh (r) ] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [ Script (s) ] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [ Export (e) ] ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [ Wipe/Del (x) ] ",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    let snap = app.latest_snap.clone();
    let analytics = &snap.web_analytics;

    let main_block = Block::default()
        .borders(Borders::ALL)
        .title(" 📍 Site Overview & Analytics Dashboard · Press [Enter] or [d] to open Detail View")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(palette.accent));

    let main_inner = main_block.inner(chunks[1]);
    f.render_widget(main_block, chunks[1]);

    // Render Sites List Table
    let sites = app.analytic_sites();
    let site_products: Vec<crate::systat::ProductAnalyticsSummary> =
        analytics.product_analytics.clone();

    let header_cells = vec![
        Cell::from("#").style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("SITE / PRODUCT").style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("DOMAIN").style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("STATUS").style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("PAGEVIEWS").style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("VISITORS").style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("LIVE NOW").style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("BOUNCE %").style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let mut rows: Vec<Row> = Vec::new();

    let total_sites = sites.len();
    let max_visible = main_inner.height.saturating_sub(2) as usize;
    let start_idx = app.analytics_scroll.min(total_sites.saturating_sub(1));
    let end_idx = (start_idx + max_visible).min(total_sites);

    for i in start_idx..end_idx {
        let site = &sites[i];
        let is_selected = i
            == app
                .analytics_selected_site
                .min(total_sites.saturating_sub(1));
        let prod_summary = site_products.iter().find(|p| p.product_id == site.id);

        let pvs = prod_summary.map(|p| p.pageviews).unwrap_or(0);
        let viss = prod_summary.map(|p| p.visitors).unwrap_or(0);
        let live = prod_summary.map(|p| p.active_now).unwrap_or(0);
        let bounce = prod_summary.map(|p| p.bounce_rate).unwrap_or(0.0);
        let status = prod_summary.map(|p| p.status.clone()).unwrap_or_else(|| {
            if live > 0 {
                "ACTIVE".to_string()
            } else if pvs > 0 {
                "IDLE".to_string()
            } else {
                "EMPTY".to_string()
            }
        });

        let status_style = if status == "ACTIVE" {
            Style::default().fg(Color::LightGreen)
        } else if status == "IDLE" {
            Style::default().fg(Color::LightYellow)
        } else {
            Style::default().fg(palette.text_muted)
        };

        let row_style = if is_selected {
            Style::default()
                .fg(Color::Rgb(0, 0, 0))
                .bg(palette.success)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(255, 255, 255))
        };

        let row_cells = vec![
            Cell::from(format!("{}", i + 1)).style(row_style),
            Cell::from(site.name.clone()).style(row_style.add_modifier(Modifier::BOLD)),
            Cell::from(site.domain.clone()).style(row_style),
            Cell::from(status).style(status_style.add_modifier(Modifier::BOLD)),
            Cell::from(format!("{pvs}")).style(row_style),
            Cell::from(format!("{viss}")).style(row_style),
            Cell::from(format!("{live}")).style(row_style),
            Cell::from(format!("{bounce:.1}%")).style(row_style),
        ];

        rows.push(Row::new(row_cells));

        let row_y = main_inner.y
            + (i as u16 - start_idx as u16 + 2).min(main_inner.height.saturating_sub(1));
        let row_w = main_inner.width.saturating_sub(4);
        app.register_hitbox(
            Rect {
                x: main_inner.x + 2,
                y: row_y,
                width: row_w,
                height: 1,
            },
            HitboxAction::ViewAnalyticsDetail(i),
        );
    }

    let sites_table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Percentage(22),
            Constraint::Percentage(24),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Min(8),
        ],
    )
    .header(
        Row::new(header_cells).style(
            Style::default()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .style(Style::default().fg(palette.text_muted));

    f.render_widget(sites_table, main_inner);

    if app.show_analytics_script_modal {
        draw_analytics_script_modal(f, app, area);
    }

    if app.show_site_import_modal {
        draw_site_import_modal(f, app, area);
    }
}

pub fn draw_site_import_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 42, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🌐 Add Website / Landing Page to Sovereign Analytics ")
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
            "Site Name:",
            app.site_import_name.clone(),
            "e.g. CATerm Terminal",
        ),
        (
            "Domain:",
            app.site_import_domain.clone(),
            "e.g. caterm.fathforce.com",
        ),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Register a website or landing page for live visitor tracking.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.site_import_selected == i;
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
                format!("{:<14}", label),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            val_span,
        ]));
        lines.push(Line::from(""));

        let hitbox_action = match i {
            0 => HitboxAction::FocusSiteImportName,
            _ => HitboxAction::FocusSiteImportDomain,
        };
        app.register_hitbox(
            Rect {
                x: inner.x + 2,
                y: field_start_y + (i as u16 * 2),
                width: inner.width.saturating_sub(4),
                height: 1,
            },
            hitbox_action,
        );
    }

    let btn_y = inner.bottom().saturating_sub(2);
    let submit_span = Span::styled(
        " [ Submit (Enter) ] ",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    let cancel_span = Span::styled(
        " [ Cancel (Esc) ] ",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    app.register_hitbox(
        Rect {
            x: inner.x + 2,
            y: btn_y,
            width: 20,
            height: 1,
        },
        HitboxAction::SubmitSiteImport,
    );
    app.register_hitbox(
        Rect {
            x: inner.x + 24,
            y: btn_y,
            width: 18,
            height: 1,
        },
        HitboxAction::CancelSiteImport,
    );

    f.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x + 1,
            y: inner.y + 1,
            width: inner.width.saturating_sub(2),
            height: inner.height.saturating_sub(3),
        },
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![submit_span, Span::raw("  "), cancel_span])),
        Rect {
            x: inner.x + 2,
            y: btn_y,
            width: inner.width.saturating_sub(4),
            height: 1,
        },
    );
}

// ── Pro Modal Wizard (Phase 4) ──────────────────────────────────────────────

pub fn draw_analytics_script_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let modal_width = 96.min(area.width.saturating_sub(2));
    let modal_height = 24.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(modal_width)) / 2;
    let y = (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect {
        x,
        y,
        width: modal_width,
        height: modal_height,
    };

    f.render_widget(Clear, modal_area);

    let active_site = app.analytics_site_filter.as_deref().unwrap_or("caterm");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " 📋 Sovereign Web Analytics · Tracker Templates (Site ID: '{active_site}') "
        ))
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(palette.success));

    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    let lines = vec![
        Line::from(vec![
            Span::styled("⚡ 1. Universal HTML / Landing (index.html):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            format!("   <script defer src=\"http://127.0.0.1:8090/tracker.js\" data-site=\"{active_site}\"></script>"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("⚡ 2. Svelte 5 / SvelteKit (src/routes/+layout.svelte):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            format!("   <svelte:head>\n     <script defer src=\"http://127.0.0.1:8090/tracker.js\" data-site=\"{active_site}\"></script>\n   </svelte:head>"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("⚡ 3. Astro / SSR (src/layouts/Layout.astro):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            format!("   <script defer src=\"http://127.0.0.1:8090/tracker.js\" data-site=\"{active_site}\"></script>"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("⚡ 4. WordPress / PHP (functions.php):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(
            format!("   add_action('wp_head', fn() => print('<script defer src=\"http://127.0.0.1:8090/tracker.js\" data-site=\"{active_site}\"></script>'));"),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled("⚡ 5. Custom CTA Tracking (data attributes):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(
            "   <button data-cashell-event=\"signup_cta\" data-cashell-val=\"pro_annual\">Get Pro</button>",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled("📌 Note: Auto-tracks Scroll Depth (25/50/75/90/100%), Clicks (% coords), Rage Clicks (>3/sec), and Dwell Time.", Style::default().fg(palette.text_muted))),
        Line::from(Span::styled("📌 Replace 'http://127.0.0.1:8090' with your public domain or CF Tunnel (e.g. 'https://analytics.example.com').", Style::default().fg(palette.text_muted))),
        Line::from(""),
        Line::from(Span::styled(" [ Press Esc to Close ] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}
