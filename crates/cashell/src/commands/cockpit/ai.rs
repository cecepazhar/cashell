//! Hana AI Workspace Chat Tab — extracted from cockpit.rs (F-07).

use super::{cockpit_theme, CockpitApp, HitboxAction};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub fn draw_ai_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);

    let telem = crate::pet::MascotTelemetry {
        cpu_pct: app.latest_snap.cpu_pct,
        cpu_temp_c: app.latest_snap.cpu_temp_c,
        ram_pct: ((app.latest_snap.ram_used_mb as f64)
            / (app.latest_snap.ram_total_mb.max(1) as f64)
            * 100.0) as u8,
        swap_pct: ((app.latest_snap.swap_used_mb as f64)
            / (app.latest_snap.swap_total_mb.max(1) as f64)
            * 100.0) as u8,
        containers_down: app
            .latest_snap
            .docker_containers
            .iter()
            .filter(|c| c.state != "running")
            .count(),
        fleet_nodes_down: app
            .server_probe_results
            .iter()
            .filter(|(_, lat)| lat.is_none())
            .count(),
        uptime_monitors_down: app
            .latest_snap
            .uptime_monitors
            .iter()
            .filter(|m| !m.is_up)
            .count(),
        git_dirty_count: app
            .latest_snap
            .tracked_projects
            .iter()
            .filter(|p| p.is_dirty)
            .count(),
        turbo_active: app.turbo_profile_active != "ECO_BALANCED"
            && !app.turbo_profile_active.is_empty(),
        transient_action: if app.is_chat_processing {
            Some(crate::pet::PetActionState::Run)
        } else {
            None
        },
        xp_level: 3,
        tick: app.pet_tick,
        speed: app.cfg.mascot.speed.clone(),
        sensitivity: app.cfg.mascot.sensitivity.clone(),
        condition_tracking: app.cfg.mascot.condition_tracking,
        color_scheme: app.cfg.mascot.color_scheme.clone(),
        animation_style: app.cfg.mascot.animation_style.clone(),
        render_scale: "Compact".to_string(),
        matrix_speed: app.cfg.mascot.matrix_speed.clone(),
    };
    let expr = crate::pet::select_expression(&telem);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let bar_inner = bar_block.inner(chunks[0]);
    f.render_widget(bar_block, chunks[0]);

    let active_model = app.current_ai_model();
    let btn_model_w = (active_model.len() as u16) + 26;
    let btn_clear_w = 16u16;

    app.register_hitbox(
        Rect {
            x: bar_inner.x + 1,
            y: bar_inner.y,
            width: btn_model_w,
            height: 1,
        },
        HitboxAction::CycleAiModel,
    );
    app.register_hitbox(
        Rect {
            x: bar_inner.x + btn_model_w + 3,
            y: bar_inner.y,
            width: btn_clear_w,
            height: 1,
        },
        HitboxAction::ClearAiChat,
    );

    let mut bar_spans = vec![
        Span::styled(
            format!(" [🤖 Model: {active_model} (m)] "),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            " [🗑 Clear Chat] ",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if app.cfg.mascot.chat_avatar_enabled {
        bar_spans.push(Span::raw("  "));
        bar_spans.push(Span::styled(
            format!(" [🪙 #{:02} {}] ", expr.id, expr.label),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }
    bar_spans.push(Span::raw("  │  Gateway: "));
    bar_spans.push(Span::styled(
        "OmniRoute / 9router (localhost:20128 / :3007)",
        Style::default().fg(palette.accent),
    ));
    f.render_widget(Paragraph::new(Line::from(bar_spans)), bar_inner);

    // Split middle area horizontally if wide enough and mascot in chat is enabled
    let (chat_area, mascot_area) = if app.cfg.mascot.chat_avatar_enabled && chunks[1].width >= 82 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(chunks[1]);
        (cols[0], Some(cols[1]))
    } else {
        (chunks[1], None)
    };

    let hist_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Workspace AI Chat — Persona: {} ({}) ",
            app.cfg.persona.user_name, app.cfg.persona.assistant_role
        ))
        .border_style(Style::default().fg(palette.border));
    let hist_inner = hist_block.inner(chat_area);
    f.render_widget(hist_block, chat_area);

    let chat_avatar_on = app.cfg.mascot.chat_avatar_enabled;
    let items: Vec<ListItem> = app
        .chat_lines
        .iter()
        .map(|l| {
            let (display_text, style) = if l.starts_with("User:") {
                let text = if chat_avatar_on {
                    format!(
                        "👤 {}: {}",
                        app.cfg.persona.user_name,
                        l.strip_prefix("User:").unwrap_or(l).trim_start()
                    )
                } else {
                    l.clone()
                };
                (
                    text,
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else if l.starts_with("AI:") {
                let text = if chat_avatar_on {
                    format!(
                        "🪙 [#{:02} {}] {}: {}",
                        expr.id,
                        expr.label,
                        app.cfg.persona.assistant_role,
                        l.strip_prefix("AI:").unwrap_or(l).trim_start()
                    )
                } else {
                    l.clone()
                };
                (text, Style::default().fg(Color::White))
            } else if l.starts_with("Error:") {
                (
                    l.clone(),
                    Style::default()
                        .fg(palette.danger)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (l.clone(), Style::default().fg(palette.text_muted))
            };
            ListItem::new(Line::from(Span::styled(display_text, style)))
        })
        .collect();

    let chat_list = List::new(items)
        .highlight_style(Style::default().bg(palette.row_selected_bg))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(chat_list, hist_inner, &mut app.chat_state);

    // Companion Chat Reactor Panel on Right Side
    if let Some(m_area) = mascot_area {
        let m_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " 🪙 Companion Reactor [#{:02} {}] ",
                expr.id, expr.label
            ))
            .border_style(
                Style::default()
                    .fg(Color::Rgb(57, 255, 20))
                    .add_modifier(Modifier::BOLD),
            );
        let m_inner = m_block.inner(m_area);
        f.render_widget(m_block, m_area);

        let (m_lines, _) = crate::pet::render_sovereign_token_lines(
            m_inner.width as usize,
            m_inner.height.saturating_sub(2) as usize,
            &telem,
            app.cfg.output.matrix_rain,
        );

        let mut all_widget_lines = m_lines;
        let reaction_msg = if app.is_chat_processing {
            "⚡ Sovereign Thinking... generating token synthesis"
        } else {
            "✔ Reacting live to prompts & system health"
        };
        all_widget_lines.push(Line::from(Span::styled(
            reaction_msg,
            Style::default().fg(if app.is_chat_processing {
                Color::Yellow
            } else {
                palette.text_muted
            }),
        )));

        f.render_widget(
            Paragraph::new(all_widget_lines).alignment(Alignment::Center),
            m_inner,
        );
    }

    let in_block = Block::default()
        .borders(Borders::ALL)
        .title(if app.is_chat_processing {
            format!(" 🪙 AI is thinking... [#{:02} {}] ", expr.id, expr.label)
        } else {
            " Prompt Input ($ > Enter to send | 'm': Model | 'c': Clear) ".to_string()
        })
        .border_style(if app.is_chat_processing {
            Style::default().fg(palette.warning)
        } else {
            Style::default().fg(palette.success)
        });
    let in_inner = in_block.inner(chunks[2]);
    f.render_widget(in_block, chunks[2]);

    let prompt_line = Line::from(vec![
        Span::styled(
            "$ > ",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &app.chat_input,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "█",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(Paragraph::new(prompt_line), in_inner);
}

// ── Tab 10: Setting (Visual Settings GUI for config.toml) ─────────────────────
