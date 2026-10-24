//! `cashell uptime` command — probe and display live uptime status of all CA Ecosystem endpoints.

use anyhow::Result;
use clap::Parser;
use colored::Colorize;

#[derive(Parser, Debug, Clone)]
#[command(name = "uptime", about = "Probe and display live uptime status of all CA Ecosystem endpoints and fleet nodes")]
pub struct UptimeCommand {
    /// Filter by group (e.g. 'CA Ecosystem', 'Fleet Nodes', 'Localhost')
    #[arg(short, long)]
    pub group: Option<String>,
}

pub fn run(cmd: &UptimeCommand) -> Result<()> {
    println!("{}", "🔍 Probing CA Ecosystem & Fleet uptime endpoints...".cyan().bold());

    let mut monitors = crate::systat::probe_uptime_monitors();

    if let Some(ref grp) = cmd.group {
        let filter_lower = grp.to_lowercase();
        monitors.retain(|m| m.group.to_lowercase().contains(&filter_lower));
    }

    if monitors.is_empty() {
        println!("{}", "No monitors match the specified filter.".yellow());
        return Ok(());
    }

    // Sort by group then name
    monitors.sort_by(|a, b| {
        a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name))
    });

    println!();
    println!(
        "  {:<16} {:<24} {:<42} {:<12} {:<8}",
        "GROUP".bold().dimmed(),
        "NAME".bold(),
        "TARGET".bold(),
        "STATUS".bold(),
        "LATENCY".bold()
    );
    println!("  {}", "─".repeat(106).dimmed());

    let mut up_count = 0;
    let total = monitors.len();

    for m in &monitors {
        let status_str = if m.is_up {
            up_count += 1;
            let code = m.status_code.map(|c| c.to_string()).unwrap_or_else(|| "200".to_string());
            format!("● UP ({})", code).green().bold()
        } else {
            let code = m.status_code.map(|c| c.to_string()).unwrap_or_else(|| "ERR".to_string());
            format!("✖ DOWN ({})", code).red().bold()
        };

        let latency_str = match m.latency_ms {
            Some(ms) if ms < 150 => format!("{}ms", ms).green(),
            Some(ms) if ms < 500 => format!("{}ms", ms).yellow(),
            Some(ms) => format!("{}ms", ms).red(),
            None => "-".dimmed(),
        };

        println!(
            "  {:<16} {:<24} {:<42} {:<12} {:<8}",
            m.group.dimmed(),
            m.name.bold(),
            if m.target.len() > 40 {
                format!("{}...", &m.target[..37])
            } else {
                m.target.clone()
            },
            status_str,
            latency_str
        );
    }

    println!("  {}", "─".repeat(106).dimmed());
    let pct = (up_count as f32 / total as f32) * 100.0;
    let summary_color = if pct >= 99.0 {
        format!("{}/{} Operational ({:.1}%)", up_count, total, pct).green().bold()
    } else if pct >= 75.0 {
        format!("{}/{} Operational ({:.1}%)", up_count, total, pct).yellow().bold()
    } else {
        format!("{}/{} Operational ({:.1}%)", up_count, total, pct).red().bold()
    };

    println!("  Summary: {}\n", summary_color);

    Ok(())
}
