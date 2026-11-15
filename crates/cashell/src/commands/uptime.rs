//! `cashell uptime` command — probe and display live uptime status of all CA Ecosystem endpoints.

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::config::Config;
use crate::db::Database;
use crate::uptime::{get_all_monitors_summary, render_status_page};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "uptime",
    about = "Probe and display live uptime status of all CA Ecosystem endpoints and fleet nodes"
)]
pub struct UptimeCommand {
    /// Filter by group (e.g. 'CA Ecosystem', 'Fleet Nodes', 'Localhost')
    #[arg(short, long)]
    pub group: Option<String>,

    /// Output full status and metrics as JSON (U-07)
    #[arg(long)]
    pub json: bool,

    /// Show uptime history and percentages over duration (e.g. '7d', '30d', '90d') (U-04)
    #[arg(long)]
    pub history: Option<String>,

    /// Subcommand for uptime operations
    #[command(subcommand)]
    pub action: Option<UptimeSubcommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum UptimeSubcommand {
    /// Generate a standalone static HTML status page without external JS (U-05)
    StatusPage {
        /// Target path for output HTML file (default: status.html)
        #[arg(long, default_value = "./status.html")]
        out: String,
        /// Title for the status page
        #[arg(long, default_value = "CAShell System Status")]
        title: String,
    },
    /// Run an on-demand probe of all or specific endpoints (U-01)
    Check {
        /// Optional name of specific monitor to probe
        monitor: Option<String>,
    },
    /// List all configured monitors
    List,
}

pub fn run(cmd: &UptimeCommand) -> Result<()> {
    let cfg = Config::load(None).unwrap_or_default();
    let db = Database::open()?;

    // Handle subcommands
    if let Some(ref action) = cmd.action {
        match action {
            UptimeSubcommand::StatusPage { out, title } => {
                return run_status_page(&cfg, &db, out, title);
            }
            UptimeSubcommand::Check { monitor } => {
                return run_check(&cfg, &db, monitor.as_deref());
            }
            UptimeSubcommand::List => {
                return run_list(&cfg, &db);
            }
        }
    }

    // JSON output (U-07)
    if cmd.json {
        let mut summaries = get_all_monitors_summary(&cfg, &db)?;
        if let Some(ref grp) = cmd.group {
            let filter_lower = grp.to_lowercase();
            summaries.retain(|m| m.group.to_lowercase().contains(&filter_lower));
        }
        println!("{}", serde_json::to_string_pretty(&summaries)?);
        return Ok(());
    }

    // History mode (U-04)
    if let Some(ref duration_str) = cmd.history {
        let days = parse_history_days(duration_str);
        return run_history(&cfg, &db, days, cmd.group.as_deref());
    }

    // Default table output
    run_default_table(&cfg, &db, cmd.group.as_deref())
}

fn parse_history_days(s: &str) -> u32 {
    let clean = s.trim().trim_end_matches('d').trim_end_matches('D');
    clean.parse::<u32>().unwrap_or(30).max(1)
}

fn run_status_page(cfg: &Config, db: &Database, out_path: &str, title: &str) -> Result<()> {
    let summaries = get_all_monitors_summary(cfg, db)?;
    let incidents = db.list_incidents(10)?;
    let html = render_status_page(title, &summaries, &incidents);

    std::fs::write(out_path, html)?;
    println!(
        "{} Static status page generated at: {}",
        "✔".green().bold(),
        out_path
    );
    Ok(())
}

fn run_check(cfg: &Config, db: &Database, monitor_filter: Option<&str>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let monitors = crate::uptime::load_configured_monitors(cfg, db);
    let filtered: Vec<_> = monitors
        .into_iter()
        .filter(|m| {
            if let Some(filter) = monitor_filter {
                m.name.to_lowercase().contains(&filter.to_lowercase())
            } else {
                true
            }
        })
        .collect();

    if filtered.is_empty() {
        println!("{}", "No matching monitors found to probe.".yellow());
        return Ok(());
    }

    println!(
        "{} Probing {} monitor(s)...",
        "🔍".cyan().bold(),
        filtered.len()
    );

    rt.block_on(async {
        for m in &filtered {
            let res = crate::uptime::probe_endpoint(m).await;
            let status_badge = if res.is_up {
                "✔ UP".green().bold()
            } else {
                "✖ DOWN".red().bold()
            };
            let latency = res
                .latency_ms
                .map(|l| format!("{l}ms"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  {:<24} {:<10} {:<8} {}",
                m.name.bold(),
                status_badge,
                latency,
                res.error.as_deref().unwrap_or("")
            );

            let _ = db.record_uptime_check(
                &m.name,
                res.is_up,
                res.latency_ms,
                res.status_code,
                res.error.as_deref(),
                res.cert_expires_at.as_deref(),
            );
        }
    });

    Ok(())
}

fn run_list(cfg: &Config, db: &Database) -> Result<()> {
    let monitors = crate::uptime::load_configured_monitors(cfg, db);
    println!(
        "  {:<16} {:<24} {:<40} {:<8} {:<8}",
        "GROUP".bold().dimmed(),
        "NAME".bold(),
        "TARGET".bold(),
        "TYPE".bold(),
        "PUBLIC".bold(),
    );
    println!("  {}", "─".repeat(100).dimmed());
    for m in monitors {
        let pub_str = if m.public {
            "yes".green()
        } else {
            "no".dimmed()
        };
        println!(
            "  {:<16} {:<24} {:<40} {:<8} {:<8}",
            m.group.dimmed(),
            m.name.bold(),
            m.target,
            m.monitor_type.as_str(),
            pub_str
        );
    }
    Ok(())
}

fn run_history(cfg: &Config, db: &Database, days: u32, group_filter: Option<&str>) -> Result<()> {
    println!(
        "{}",
        format!("📊 Endpoint Uptime History (past {days} days)")
            .cyan()
            .bold()
    );
    println!();
    println!(
        "  {:<16} {:<24} {:<42} {:<12} {:<10}",
        "GROUP".bold().dimmed(),
        "NAME".bold(),
        "TARGET".bold(),
        "STATUS".bold(),
        format!("{days}d UPTIME").bold()
    );
    println!("  {}", "─".repeat(110).dimmed());

    let mut summaries = get_all_monitors_summary(cfg, db)?;
    if let Some(grp) = group_filter {
        let filter_lower = grp.to_lowercase();
        summaries.retain(|m| m.group.to_lowercase().contains(&filter_lower));
    }

    for m in &summaries {
        let pct = db.get_uptime_percentage(&m.name, days).unwrap_or(100.0);
        let status_str = match m.state {
            crate::uptime::MonitorState::Up => "● UP".green().bold(),
            crate::uptime::MonitorState::Degraded => "⚠ DEGRADED".yellow().bold(),
            crate::uptime::MonitorState::Down => "✖ DOWN".red().bold(),
        };

        let pct_str = if pct >= 99.0 {
            format!("{pct:.1}%").green()
        } else if pct >= 95.0 {
            format!("{pct:.1}%").yellow()
        } else {
            format!("{pct:.1}%").red()
        };

        println!(
            "  {:<16} {:<24} {:<42} {:<12} {:<10}",
            m.group.dimmed(),
            m.name.bold(),
            if m.target.len() > 40 {
                format!("{}...", &m.target[..37])
            } else {
                m.target.clone()
            },
            status_str,
            pct_str
        );
    }
    println!("  {}", "─".repeat(110).dimmed());
    Ok(())
}

fn run_default_table(_cfg: &Config, db: &Database, group_filter: Option<&str>) -> Result<()> {
    println!(
        "{}",
        "🔍 Probing CA Ecosystem & Fleet uptime endpoints..."
            .cyan()
            .bold()
    );

    let mut monitors = crate::systat::probe_uptime_monitors();

    if let Some(grp) = group_filter {
        let filter_lower = grp.to_lowercase();
        monitors.retain(|m| m.group.to_lowercase().contains(&filter_lower));
    }

    if monitors.is_empty() {
        println!("{}", "No monitors match the specified filter.".yellow());
        return Ok(());
    }

    monitors.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));

    println!();
    println!(
        "  {:<16} {:<24} {:<42} {:<12} {:<8} {:<10}",
        "GROUP".bold().dimmed(),
        "NAME".bold(),
        "TARGET".bold(),
        "STATUS".bold(),
        "LATENCY".bold(),
        "30d UPTIME".bold()
    );
    println!("  {}", "─".repeat(118).dimmed());

    let mut up_count = 0;
    let total = monitors.len();

    for m in &monitors {
        let status_str = if m.is_up {
            up_count += 1;
            let code = m
                .status_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "200".to_string());
            format!("● UP ({code})").green().bold()
        } else {
            let code = m
                .status_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "ERR".to_string());
            format!("✖ DOWN ({code})").red().bold()
        };

        let latency_str = match m.latency_ms {
            Some(ms) if ms < 150 => format!("{ms}ms").green(),
            Some(ms) if ms < 500 => format!("{ms}ms").yellow(),
            Some(ms) => format!("{ms}ms").red(),
            None => "-".dimmed(),
        };

        let pct = db.get_uptime_percentage(&m.name, 30).unwrap_or(100.0);
        let pct_str = if pct >= 99.0 {
            format!("{pct:.1}%").green()
        } else if pct >= 95.0 {
            format!("{pct:.1}%").yellow()
        } else {
            format!("{pct:.1}%").red()
        };

        println!(
            "  {:<16} {:<24} {:<42} {:<12} {:<8} {:<10}",
            m.group.dimmed(),
            m.name.bold(),
            if m.target.len() > 40 {
                format!("{}...", &m.target[..37])
            } else {
                m.target.clone()
            },
            status_str,
            latency_str,
            pct_str
        );
    }

    println!("  {}", "─".repeat(118).dimmed());
    let pct = (up_count as f32 / total as f32) * 100.0;
    let summary_color = if pct >= 99.0 {
        format!("{up_count}/{total} Operational ({pct:.1}%)")
            .green()
            .bold()
    } else if pct >= 75.0 {
        format!("{up_count}/{total} Operational ({pct:.1}%)")
            .yellow()
            .bold()
    } else {
        format!("{up_count}/{total} Operational ({pct:.1}%)")
            .red()
            .bold()
    };
    println!("  Summary: {summary_color}");
    println!();
    Ok(())
}
