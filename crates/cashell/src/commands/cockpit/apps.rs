//! Application & Container Workspaces (Dokploy, Docker, Project Repos) — extracted from cockpit.rs (F-07).

use super::{cockpit_theme, CockpitApp, HitboxAction, ProjectView};
use catui::layout::centered_rect;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};

pub fn draw_project_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    let action_bar = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let action_inner = action_bar.inner(chunks[0]);
    f.render_widget(action_bar, chunks[0]);

    let (btn_hier_bg, btn_hier_fg) = if app.project_view == ProjectView::Hierarchy {
        (palette.highlight_bg, palette.highlight_fg)
    } else {
        (Color::Reset, palette.text_muted)
    };
    let (btn_tree_bg, btn_tree_fg) = if app.project_view == ProjectView::Tree {
        (palette.highlight_bg, palette.highlight_fg)
    } else {
        (Color::Reset, palette.text_muted)
    };
    let (btn_cont_bg, btn_cont_fg) = if app.project_view == ProjectView::Containers {
        (palette.highlight_bg, palette.highlight_fg)
    } else {
        (Color::Reset, palette.text_muted)
    };

    let btn_hier_w = 17u16;
    let btn_tree_w = 14u16;
    let btn_cont_w = 18u16;
    let btn_add_proj_w = 18u16;
    let btn_add_cont_w = 20u16;
    let btn_start_w = 11u16;
    let btn_stop_w = 10u16;
    let btn_restart_w = 13u16;
    let btn_edit_w = 12u16;
    let btn_del_w = 14u16;

    let target_list = app.project_server_targets();
    let target_display = if app.project_server_target == 0 {
        "LOCAL".to_string()
    } else {
        target_list
            .get(app.project_server_target - 1)
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| "LOCAL".to_string())
    };
    let target_label_text = format!(" [🖥 Target: {} (g)] ", target_display);
    let target_btn_w = (target_label_text.len() + 1) as u16;
    let btn_target_w = target_btn_w;

    let mut cur_x = action_inner.x + 1;
    let y = action_inner.y;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_hier_w,
            height: 1,
        },
        HitboxAction::SelectProjectView(ProjectView::Hierarchy),
    );
    cur_x += btn_hier_w + 1;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_tree_w,
            height: 1,
        },
        HitboxAction::SelectProjectView(ProjectView::Tree),
    );
    cur_x += btn_tree_w + 1;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_cont_w,
            height: 1,
        },
        HitboxAction::SelectProjectView(ProjectView::Containers),
    );
    cur_x += btn_cont_w + 1;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_target_w,
            height: 1,
        },
        HitboxAction::CycleProjectServer,
    );
    cur_x += btn_target_w + 1;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_add_proj_w,
            height: 1,
        },
        HitboxAction::AddProject,
    );
    cur_x += btn_add_proj_w + 1;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_add_cont_w,
            height: 1,
        },
        HitboxAction::AddContainer,
    );
    cur_x += btn_add_cont_w + 2;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_start_w,
            height: 1,
        },
        HitboxAction::ContainerStart(app.selected_container),
    );
    cur_x += btn_start_w + 1;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_stop_w,
            height: 1,
        },
        HitboxAction::ContainerStop(app.selected_container),
    );
    cur_x += btn_stop_w + 1;

    app.register_hitbox(
        Rect {
            x: cur_x,
            y,
            width: btn_restart_w,
            height: 1,
        },
        HitboxAction::ContainerRestart(app.selected_container),
    );
    cur_x += btn_restart_w + 1;

    if app.project_view == ProjectView::Containers {
        app.register_hitbox(
            Rect {
                x: cur_x,
                y,
                width: btn_edit_w,
                height: 1,
            },
            HitboxAction::EditContainer,
        );
        cur_x += btn_edit_w + 1;
        app.register_hitbox(
            Rect {
                x: cur_x,
                y,
                width: btn_del_w,
                height: 1,
            },
            HitboxAction::RemoveContainer,
        );
    } else {
        app.register_hitbox(
            Rect {
                x: cur_x,
                y,
                width: btn_del_w,
                height: 1,
            },
            HitboxAction::RemoveProject,
        );
    }

    let mut bar_spans = vec![
        Span::styled(
            " [📁 Hierarchy] ",
            Style::default()
                .fg(btn_hier_fg)
                .bg(btn_hier_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [🌳 Tree] ",
            Style::default()
                .fg(btn_tree_fg)
                .bg(btn_tree_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            " [🐳 Containers] ",
            Style::default()
                .fg(btn_cont_fg)
                .bg(btn_cont_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            target_label_text.as_str(),
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" │ "),
    ];
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(
        " [+ Add Project] ",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ));
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(
        " [+ Add Container] ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ));
    bar_spans.push(Span::raw(" │ Actions: "));
    bar_spans.push(Span::styled(
        " [▶ Start] ",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ));
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(
        " [■ Stop] ",
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
    ));
    bar_spans.push(Span::raw(" "));
    bar_spans.push(Span::styled(
        " [↺ Restart] ",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ));

    if app.project_view == ProjectView::Containers {
        bar_spans.push(Span::raw(" "));
        bar_spans.push(Span::styled(
            " [✎ Edit] (e) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        bar_spans.push(Span::raw(" "));
        bar_spans.push(Span::styled(
            " [🗑 Delete] (x) ",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        bar_spans.push(Span::raw(" "));
        bar_spans.push(Span::styled(
            " [🗑 Delete] (x) ",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(bar_spans)), action_inner);

    match app.project_view {
        ProjectView::Hierarchy => {
            let split_dir = if chunks[1].width < 90 {
                Direction::Vertical
            } else {
                Direction::Horizontal
            };
            let sub_chunks = Layout::default()
                .direction(split_dir)
                .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(chunks[1]);

            let total_projects = app.latest_snap.tracked_projects.len();
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" 📁 Project Workspaces ({}) ", total_projects))
                .border_style(Style::default().fg(palette.border));
            let inner = block.inner(sub_chunks[0]);

            let visible_rows = inner.height.saturating_sub(1) as usize; // reserve 1 line for header
            if total_projects > 0 && app.selected_project >= total_projects {
                app.selected_project = total_projects - 1;
            }

            let start_idx = if visible_rows == 0 || total_projects <= visible_rows {
                0
            } else {
                let sel = app.selected_project;
                if sel < app.project_scroll {
                    sel
                } else if sel >= app.project_scroll + visible_rows {
                    sel + 1 - visible_rows
                } else {
                    app.project_scroll
                }
            };
            let start_idx = start_idx.min(total_projects.saturating_sub(visible_rows));
            app.project_scroll = start_idx;
            let end_idx = (start_idx + visible_rows).min(total_projects);

            let title_text = if total_projects > visible_rows {
                format!(
                    " 📁 Workspaces ({}) [{}-{}/{}] ",
                    total_projects,
                    start_idx + 1,
                    end_idx,
                    total_projects
                )
            } else {
                format!(" 📁 Project Workspaces ({}) ", total_projects)
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .title(title_text)
                .border_style(Style::default().fg(palette.border));
            f.render_widget(block, sub_chunks[0]);

            let rows: Vec<Row> = if total_projects == 0 {
                Vec::new()
            } else {
                app.latest_snap.tracked_projects[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(offset, p)| {
                        let real_idx = start_idx + offset;
                        let is_sel = real_idx == app.selected_project;
                        let bg = if is_sel {
                            palette.row_selected_bg
                        } else {
                            Color::Reset
                        };
                        let (dirty_badge, dirty_col) = if p.is_dirty {
                            ("● DIRTY", palette.warning)
                        } else {
                            ("● CLEAN", palette.success)
                        };
                        let repos_str = format!("📦 {}", p.repos.len());
                        let cont_str = if p.containers.is_empty() {
                            "-".to_string()
                        } else {
                            format!("🐳 {}", p.containers.len())
                        };
                        let svc_str = if p.services.is_empty() {
                            "-".to_string()
                        } else {
                            format!("⚙ {}", p.services.len())
                        };
                        let db_str = if p.databases.is_empty() {
                            "-".to_string()
                        } else {
                            format!("🗄 {}", p.databases.len())
                        };

                        Row::new(vec![
                            Cell::from(p.name.clone()).style(
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Cell::from(p.category.clone())
                                .style(Style::default().fg(palette.accent)),
                            Cell::from(repos_str).style(Style::default().fg(palette.chart[0])),
                            Cell::from(cont_str).style(Style::default().fg(palette.success)),
                            Cell::from(svc_str).style(Style::default().fg(palette.warning)),
                            Cell::from(db_str).style(Style::default().fg(palette.accent)),
                            Cell::from(dirty_badge)
                                .style(Style::default().fg(dirty_col).add_modifier(Modifier::BOLD)),
                        ])
                        .style(Style::default().bg(bg))
                    })
                    .collect()
            };

            let table = Table::new(
                rows,
                [
                    Constraint::Length(18),
                    Constraint::Length(10),
                    Constraint::Length(7),
                    Constraint::Length(7),
                    Constraint::Length(7),
                    Constraint::Length(7),
                    Constraint::Min(9),
                ],
            )
            .header(
                Row::new(vec![
                    "WORKSPACE",
                    "CAT",
                    "REPOS",
                    "CONT",
                    "SVC",
                    "DBS",
                    "STATUS",
                ])
                .style(
                    Style::default()
                        .fg(palette.warning)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            f.render_widget(table, inner);

            for offset in 0..(end_idx.saturating_sub(start_idx)) {
                let real_idx = start_idx + offset;
                app.register_hitbox(
                    Rect {
                        x: inner.x,
                        y: inner.y + 1 + (offset as u16),
                        width: inner.width,
                        height: 1,
                    },
                    HitboxAction::SelectProject(real_idx),
                );
            }

            // Right panel: Selected workspace resources (Repos, Containers, Services, DBs)
            if !app.latest_snap.tracked_projects.is_empty() {
                let sel_idx = app
                    .selected_project
                    .min(app.latest_snap.tracked_projects.len() - 1);
                let proj = &app.latest_snap.tracked_projects[sel_idx];

                let detail_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" 📦 Resources: {} ({}) ", proj.name, proj.path))
                    .border_style(Style::default().fg(palette.accent));
                let detail_inner = detail_block.inner(sub_chunks[1]);
                f.render_widget(detail_block, sub_chunks[1]);

                let res_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Percentage(28),
                        Constraint::Percentage(28),
                        Constraint::Percentage(22),
                        Constraint::Percentage(22),
                    ])
                    .split(detail_inner);

                // 1. Repos Card
                let repo_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " 📦 Repositories & Code Sources ({}) ",
                        proj.repos.len()
                    ))
                    .border_style(Style::default().fg(palette.border));
                let r_inner = repo_block.inner(res_chunks[0]);
                f.render_widget(repo_block, res_chunks[0]);

                let repo_rows: Vec<Row> = if proj.repos.is_empty() {
                    vec![Row::new(vec![Cell::from(
                        "No git repos detected in workspace",
                    )
                    .style(Style::default().fg(palette.text_muted))])]
                } else {
                    proj.repos
                        .iter()
                        .map(|r| {
                            let (d_icon, d_col) = if r.is_dirty {
                                ("● DIRTY", palette.warning)
                            } else {
                                ("● CLEAN", palette.success)
                            };
                            Row::new(vec![
                                Cell::from(r.name.clone()).style(
                                    Style::default()
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Cell::from(r.branch.clone())
                                    .style(Style::default().fg(palette.chart[0])),
                                Cell::from(d_icon)
                                    .style(Style::default().fg(d_col).add_modifier(Modifier::BOLD)),
                                Cell::from(if r.path.is_empty() {
                                    "root".to_string()
                                } else {
                                    r.path.clone()
                                })
                                .style(Style::default().fg(palette.text_muted)),
                            ])
                        })
                        .collect()
                };
                let r_table = Table::new(
                    repo_rows,
                    [
                        Constraint::Length(22),
                        Constraint::Length(14),
                        Constraint::Length(12),
                        Constraint::Min(15),
                    ],
                )
                .header(
                    Row::new(vec!["REPO", "BRANCH", "STATUS", "PATH"]).style(
                        Style::default()
                            .fg(palette.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
                f.render_widget(r_table, r_inner);

                // 2. Containers Card
                let cont_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " 🐳 Docker / Podman Containers ({}) ",
                        proj.containers.len()
                    ))
                    .border_style(Style::default().fg(palette.border));
                let c_inner = cont_block.inner(res_chunks[1]);
                f.render_widget(cont_block, res_chunks[1]);

                let cont_rows: Vec<Row> = if proj.containers.is_empty() {
                    vec![Row::new(vec![Cell::from(
                        "No containers mapped to this workspace",
                    )
                    .style(Style::default().fg(palette.text_muted))])]
                } else {
                    proj.containers
                        .iter()
                        .map(|c| {
                            let (s_icon, s_col) = match c.state.as_str() {
                                "running" => ("● RUNNING", palette.success),
                                "paused" => ("● PAUSED", palette.warning),
                                "defined" => ("◌ DEFINED", palette.chart[0]),
                                _ => ("✖ EXITED", palette.danger),
                            };
                            Row::new(vec![
                                Cell::from(c.name.clone()).style(
                                    Style::default()
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Cell::from(c.image.clone())
                                    .style(Style::default().fg(palette.chart[0])),
                                Cell::from(s_icon)
                                    .style(Style::default().fg(s_col).add_modifier(Modifier::BOLD)),
                                Cell::from(c.ports.clone())
                                    .style(Style::default().fg(palette.text_muted)),
                            ])
                        })
                        .collect()
                };
                let c_table = Table::new(
                    cont_rows,
                    [
                        Constraint::Length(22),
                        Constraint::Length(22),
                        Constraint::Length(14),
                        Constraint::Min(15),
                    ],
                )
                .header(
                    Row::new(vec!["CONTAINER", "IMAGE", "STATE", "PORTS"]).style(
                        Style::default()
                            .fg(palette.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
                f.render_widget(c_table, c_inner);

                // 3. Services Card
                let svc_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " ⚙ Running Services & Ports ({}) ",
                        proj.services.len()
                    ))
                    .border_style(Style::default().fg(palette.border));
                let s_inner = svc_block.inner(res_chunks[2]);
                f.render_widget(svc_block, res_chunks[2]);

                let svc_rows: Vec<Row> = if proj.services.is_empty() {
                    vec![Row::new(vec![Cell::from(
                        "No services or exposed ports mapped",
                    )
                    .style(Style::default().fg(palette.text_muted))])]
                } else {
                    proj.services
                        .iter()
                        .map(|s| {
                            let (st_icon, st_col) = match s.state.as_str() {
                                "up" | "online" => ("● ONLINE", palette.success),
                                _ => ("✖ DOWN", palette.danger),
                            };
                            Row::new(vec![
                                Cell::from(s.name.clone()).style(
                                    Style::default()
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Cell::from(format!("{}:{}", s.host, s.port))
                                    .style(Style::default().fg(palette.accent)),
                                Cell::from(s.protocol.to_uppercase())
                                    .style(Style::default().fg(palette.chart[0])),
                                Cell::from(st_icon).style(
                                    Style::default().fg(st_col).add_modifier(Modifier::BOLD),
                                ),
                            ])
                        })
                        .collect()
                };
                let s_table = Table::new(
                    svc_rows,
                    [
                        Constraint::Length(22),
                        Constraint::Length(20),
                        Constraint::Length(12),
                        Constraint::Min(12),
                    ],
                )
                .header(
                    Row::new(vec!["SERVICE", "HOST:PORT", "PROTOCOL", "STATE"]).style(
                        Style::default()
                            .fg(palette.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
                f.render_widget(s_table, s_inner);

                // 4. Databases Card
                let db_block = Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        " 🗄 Databases (Postgres / MySQL / Redis / SQLite) ({}) ",
                        proj.databases.len()
                    ))
                    .border_style(Style::default().fg(palette.border));
                let d_inner = db_block.inner(res_chunks[3]);
                f.render_widget(db_block, res_chunks[3]);

                let db_rows: Vec<Row> = if proj.databases.is_empty() {
                    vec![Row::new(vec![Cell::from(
                        "No database instances or configs detected",
                    )
                    .style(Style::default().fg(palette.text_muted))])]
                } else {
                    proj.databases
                        .iter()
                        .map(|d| {
                            let (db_st, db_col) = match d.state.as_str() {
                                "up" | "online" => ("● ONLINE", palette.success),
                                "file" => ("● FILE", palette.chart[0]),
                                _ => ("✖ STOPPED", palette.danger),
                            };
                            let port_str = if d.port > 0 {
                                format!(":{}", d.port)
                            } else {
                                "local".to_string()
                            };
                            Row::new(vec![
                                Cell::from(d.name.clone()).style(
                                    Style::default()
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Cell::from(d.engine.to_uppercase()).style(
                                    Style::default()
                                        .fg(palette.warning)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Cell::from(port_str).style(Style::default().fg(palette.accent)),
                                Cell::from(db_st).style(
                                    Style::default().fg(db_col).add_modifier(Modifier::BOLD),
                                ),
                                Cell::from(d.database.clone())
                                    .style(Style::default().fg(palette.text_muted)),
                            ])
                        })
                        .collect()
                };
                let d_table = Table::new(
                    db_rows,
                    [
                        Constraint::Length(20),
                        Constraint::Length(12),
                        Constraint::Length(10),
                        Constraint::Length(12),
                        Constraint::Min(16),
                    ],
                )
                .header(
                    Row::new(vec!["DATABASE", "ENGINE", "PORT", "STATUS", "DETAILS"]).style(
                        Style::default()
                            .fg(palette.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                );
                f.render_widget(d_table, d_inner);
            } else {
                let empty_block = Block::default()
                    .borders(Borders::ALL)
                    .title(" 📦 Workspace Resources ")
                    .border_style(Style::default().fg(palette.border));
                f.render_widget(Paragraph::new("No workspace projects tracked. Click [+ Add Project] to track a directory.").style(Style::default().fg(palette.text_muted)), empty_block.inner(sub_chunks[1]));
            }
        }
        ProjectView::Tree => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    " 🌳 Workspace Hierarchy Tree ({}) ",
                    app.latest_snap.tracked_projects.len()
                ))
                .border_style(Style::default().fg(palette.border));
            let inner = block.inner(chunks[1]);
            f.render_widget(block, chunks[1]);

            let mut tree_rows: Vec<Row> = Vec::new();
            for (p_idx, p) in app.latest_snap.tracked_projects.iter().enumerate() {
                let is_sel = p_idx == app.selected_project;
                let bg = if is_sel {
                    palette.row_selected_bg
                } else {
                    Color::Reset
                };
                let (dirty_badge, dirty_col) = if p.is_dirty {
                    ("● DIRTY", palette.warning)
                } else {
                    ("● CLEAN", palette.success)
                };

                // Project Header Row
                tree_rows.push(
                    Row::new(vec![
                        Cell::from(format!("▼ 📁 [Project] {} ({})", p.name, p.category)).style(
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Cell::from(p.summary_line()).style(Style::default().fg(palette.chart[0])),
                        Cell::from(dirty_badge)
                            .style(Style::default().fg(dirty_col).add_modifier(Modifier::BOLD)),
                        Cell::from(p.path.clone()).style(Style::default().fg(palette.text_muted)),
                    ])
                    .style(Style::default().bg(bg)),
                );

                // Child Repos
                for r in &p.repos {
                    let (rd_icon, rd_col) = if r.is_dirty {
                        ("● DIRTY", palette.warning)
                    } else {
                        ("● CLEAN", palette.success)
                    };
                    tree_rows.push(Row::new(vec![
                        Cell::from(format!("  ├── 📦 Repo: {}", r.name))
                            .style(Style::default().fg(palette.chart[0])),
                        Cell::from(format!("git:{}", r.branch))
                            .style(Style::default().fg(palette.accent)),
                        Cell::from(rd_icon).style(Style::default().fg(rd_col)),
                        Cell::from(if r.path.is_empty() {
                            "root".to_string()
                        } else {
                            r.path.clone()
                        })
                        .style(Style::default().fg(palette.text_muted)),
                    ]));
                }

                // Child Containers
                for c in &p.containers {
                    let (c_st, c_col) = match c.state.as_str() {
                        "running" => ("● RUNNING", palette.success),
                        "paused" => ("● PAUSED", palette.warning),
                        "defined" => ("◌ DEFINED", palette.chart[0]),
                        _ => ("✖ EXITED", palette.danger),
                    };
                    tree_rows.push(Row::new(vec![
                        Cell::from(format!("  ├── 🐳 Container: {}", c.name))
                            .style(Style::default().fg(palette.success)),
                        Cell::from(c.image.clone()).style(Style::default().fg(palette.text_muted)),
                        Cell::from(c_st)
                            .style(Style::default().fg(c_col).add_modifier(Modifier::BOLD)),
                        Cell::from(c.ports.clone()).style(Style::default().fg(palette.text_muted)),
                    ]));
                }

                // Child Services
                for s in &p.services {
                    let (s_st, s_col) = match s.state.as_str() {
                        "up" | "online" => ("● ONLINE", palette.success),
                        _ => ("✖ DOWN", palette.danger),
                    };
                    tree_rows.push(Row::new(vec![
                        Cell::from(format!("  ├── ⚙ Service: {}", s.name))
                            .style(Style::default().fg(palette.warning)),
                        Cell::from(format!("{}:{}/{}", s.host, s.port, s.protocol))
                            .style(Style::default().fg(palette.accent)),
                        Cell::from(s_st)
                            .style(Style::default().fg(s_col).add_modifier(Modifier::BOLD)),
                        Cell::from("Active process / listener")
                            .style(Style::default().fg(palette.text_muted)),
                    ]));
                }

                // Child Databases
                for d in &p.databases {
                    let (d_st, d_col) = match d.state.as_str() {
                        "up" | "online" => ("● ONLINE", palette.success),
                        "file" => ("● FILE", palette.chart[0]),
                        _ => ("✖ STOPPED", palette.danger),
                    };
                    let port_str = if d.port > 0 {
                        format!(":{}", d.port)
                    } else {
                        "local".to_string()
                    };
                    tree_rows.push(Row::new(vec![
                        Cell::from(format!(
                            "  └── 🗄 DB: {} ({})",
                            d.name,
                            d.engine.to_uppercase()
                        ))
                        .style(Style::default().fg(palette.accent)),
                        Cell::from(port_str).style(Style::default().fg(palette.warning)),
                        Cell::from(d_st)
                            .style(Style::default().fg(d_col).add_modifier(Modifier::BOLD)),
                        Cell::from(d.database.clone())
                            .style(Style::default().fg(palette.text_muted)),
                    ]));
                }
            }

            let tree_table = Table::new(
                tree_rows,
                [
                    Constraint::Percentage(36),
                    Constraint::Percentage(26),
                    Constraint::Percentage(14),
                    Constraint::Percentage(24),
                ],
            )
            .header(
                Row::new(vec![
                    "WORKSPACE & RESOURCE HIERARCHY",
                    "DETAILS / CONFIG",
                    "STATUS",
                    "LOCATION",
                ])
                .style(
                    Style::default()
                        .fg(palette.warning)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            f.render_widget(tree_table, inner);

            for i in 0..app.latest_snap.tracked_projects.len() {
                if (i as u16) < inner.height.saturating_sub(1) {
                    app.register_hitbox(
                        Rect {
                            x: inner.x,
                            y: inner.y + 1 + (i as u16),
                            width: inner.width,
                            height: 1,
                        },
                        HitboxAction::SelectProject(i),
                    );
                }
            }
        }
        ProjectView::Containers => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    " 🐳 Docker & Podman Engine Containers ({}) ",
                    app.latest_snap.docker_containers.len()
                ))
                .border_style(Style::default().fg(palette.border));
            let inner = block.inner(chunks[1]);
            f.render_widget(block, chunks[1]);

            let rows: Vec<Row> = app
                .latest_snap
                .docker_containers
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let is_sel = i == app.selected_container;
                    let bg = if is_sel {
                        palette.row_selected_bg
                    } else {
                        Color::Reset
                    };
                    let (state_icon, state_col) = match c.state.as_str() {
                        "running" => ("● RUNNING", palette.success),
                        "paused" => ("● PAUSED", palette.warning),
                        _ => ("✖ EXITED", palette.danger),
                    };

                    Row::new(vec![
                        Cell::from(c.name.clone()).style(
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Cell::from(c.image.clone()).style(Style::default().fg(palette.chart[0])),
                        Cell::from(state_icon)
                            .style(Style::default().fg(state_col).add_modifier(Modifier::BOLD)),
                        Cell::from(c.status_detail.clone())
                            .style(Style::default().fg(palette.text_muted)),
                    ])
                    .style(Style::default().bg(bg))
                })
                .collect();

            let table = Table::new(
                rows,
                [
                    Constraint::Length(24),
                    Constraint::Length(26),
                    Constraint::Length(16),
                    Constraint::Min(25),
                ],
            )
            .header(
                Row::new(vec!["CONTAINER NAME", "IMAGE", "STATE", "STATUS / PORTS"]).style(
                    Style::default()
                        .fg(palette.warning)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            f.render_widget(table, inner);

            for i in 0..app.latest_snap.docker_containers.len() {
                if (i as u16) < inner.height.saturating_sub(1) {
                    app.register_hitbox(
                        Rect {
                            x: inner.x,
                            y: inner.y + 1 + (i as u16),
                            width: inner.width,
                            height: 1,
                        },
                        HitboxAction::SelectContainer(i),
                    );
                }
            }
        }
    }
}

// ── Tab 3: Uptime (Endpoint Monitors, HTTP/HTTPS, TCP, Cloudflare tunnels) ───

pub fn draw_project_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 46, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 📁 Add Tracked Project — Directory Wizard ")
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 20 || inner.height < 5 {
        return;
    }

    let fields = [
        (
            "Folder Path:",
            app.project_path_buf.clone(),
            "e.g. ~/Product/cashell or /home/...",
        ),
        (
            "Project Name:",
            app.project_name_buf.clone(),
            "Optional (auto-extracted from folder)",
        ),
        (
            "Category:",
            app.project_category_buf.clone(),
            "Product or Project",
        ),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Add a workspace folder to track in the cockpit project overview.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.project_wizard_field == i;
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
                HitboxAction::FocusProjectField(i),
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
        " [✔ Add Project] (Enter) ",
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

    let btn_y = inner.y + 7;
    if btn_y < inner.bottom() {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: 25,
                height: 1,
            },
            HitboxAction::SubmitProject,
        );
        app.register_hitbox(
            Rect {
                x: inner.x + 27,
                y: btn_y,
                width: 18,
                height: 1,
            },
            HitboxAction::CancelProject,
        );
    }
}

pub fn draw_container_wizard(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let popup_area = centered_rect(65, 52, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" 🐳 Add Docker Container — Run Wizard ")
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

    let fields = [
        (
            "Container Name:",
            app.container_name_buf.clone(),
            "e.g. my-nginx, redis-db",
        ),
        (
            "Docker Image:",
            app.container_image_buf.clone(),
            "e.g. nginx:alpine, redis:7",
        ),
        (
            "Port Mapping:",
            app.container_port_buf.clone(),
            "e.g. 8080:80 (optional)",
        ),
        (
            "Environment:",
            app.container_env_buf.clone(),
            "e.g. FOO=bar,PORT=80 (optional)",
        ),
    ];

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Specify container parameters to execute docker/podman run -d.",
        Style::default().fg(palette.text_muted),
    )));
    lines.push(Line::from(""));

    let field_start_y = inner.y + 2;
    for (i, (label, val, placeholder)) in fields.iter().enumerate() {
        let is_focused = app.container_wizard_field == i;
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
                HitboxAction::FocusContainerField(i),
            );
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Tab/Down: Next │ BackTab/Up: Prev │ Enter: Run │ Esc: Cancel",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let btn_save = Span::styled(
        " [✔ Run Container] (Enter) ",
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

    let btn_y = inner.y + 8;
    if btn_y < inner.bottom() {
        app.register_hitbox(
            Rect {
                x: inner.x,
                y: btn_y,
                width: 27,
                height: 1,
            },
            HitboxAction::SubmitContainer,
        );
        app.register_hitbox(
            Rect {
                x: inner.x + 29,
                y: btn_y,
                width: 18,
                height: 1,
            },
            HitboxAction::CancelContainer,
        );
    }
}
