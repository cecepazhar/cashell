//! `cashell analytics` command — view sovereign web analytics & traffic summary across CA ecosystem sites.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "analytics",
    about = "View sovereign web analytics & traffic summary across CA ecosystem sites"
)]
pub struct AnalyticsCommand {
    #[command(subcommand)]
    pub action: Option<AnalyticsAction>,

    /// Filter by website ID (e.g. 'docs', 'blog', 'caterm', 'fathforce')
    #[arg(short, long)]
    pub site: Option<String>,

    /// Limit number of top pages / referrers to show (default: 10)
    #[arg(short, long, default_value = "10")]
    pub limit: usize,

    /// Output full analytics summary as JSON (A-10)
    #[arg(long)]
    pub json: bool,

    /// Direct export format: 'csv' or 'json' (A-08)
    #[arg(long)]
    pub format: Option<String>,

    /// Direct export output file destination (A-08)
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AnalyticsAction {
    /// Export analytics raw data to CSV or JSON (A-08)
    Export {
        /// Format: 'csv' or 'json' (default: 'csv')
        #[arg(short, long, default_value = "csv")]
        format: String,

        /// Output destination file path
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Output summary data (default view)
    Summary,
}

pub fn run(cmd: &AnalyticsCommand) -> Result<()> {
    // Check if export subcommand or direct export flags given (A-08)
    if let Some(AnalyticsAction::Export { format, out }) = &cmd.action {
        return do_export(format, out);
    }
    if let (Some(format), Some(out)) = (&cmd.format, &cmd.out) {
        return do_export(format, out);
    }

    let db = crate::db::Database::open()?;
    let summary = db.get_analytics_summary(cmd.site.as_deref())?;

    // A-10: JSON output matching Cockpit TUI data model
    if cmd.json {
        let json_val = serde_json::to_string_pretty(&serde_json::json!({
            "pageviews": summary.pageviews,
            "unique_visitors": summary.unique_visitors,
            "active_visitors": summary.active_visitors,
            "bounce_rate": summary.bounce_rate,
            "avg_session_secs": summary.avg_session_secs,
            "top_pages": summary.top_pages,
            "top_referrers": summary.top_referrers,
            "device_breakdown": summary.device_breakdown,
            "browser_breakdown": summary.browser_breakdown,
            "os_breakdown": summary.os_breakdown,
            "country_breakdown": summary.country_breakdown,
            "daily_traffic": summary.daily_traffic,
            "product_analytics": summary.product_analytics,
            "scroll_funnel": summary.scroll_funnel,
            "top_cta_events": summary.top_cta_events,
            "rage_clicks": summary.rage_clicks,
            "avg_dwell_time_secs": summary.avg_dwell_time_secs,
        }))?;
        println!("{json_val}");
        return Ok(());
    }

    let filter_label = match &cmd.site {
        Some(s) => format!(" (Filtered by site: {})", s.cyan()),
        None => " (All Registered CA Sites)".to_string(),
    };

    println!(
        "\n{}",
        format!("📊 Sovereign Web Analytics{}", filter_label).bold()
    );
    println!("  {}", "─".repeat(80).dimmed());

    // Overview KPIs
    println!(
        "  {} {:<12}  {} {:<12}  {} {:<12}  {} {:.1}%",
        "Total Views:".bold(),
        summary.pageviews.to_string().cyan().bold(),
        "Unique Visitors:".bold(),
        summary.unique_visitors.to_string().green().bold(),
        "Active Now:".bold(),
        summary.active_visitors.to_string().yellow().bold(),
        "Bounce Rate:".bold(),
        summary.bounce_rate
    );
    println!("  {}", "─".repeat(80).dimmed());

    // Product Traffic breakdown
    if !summary.product_analytics.is_empty() {
        println!("\n  {}", "Product Breakdown:".bold().underline());
        println!(
            "  {:<16} {:<30} {:<12} {:<12}",
            "ID".bold().dimmed(),
            "SITE NAME".bold(),
            "PAGEVIEWS".bold(),
            "VISITORS".bold()
        );
        println!("  {}", "─".repeat(70).dimmed());

        let mut prods = summary.product_analytics.clone();
        prods.sort_by(|a, b| {
            b.pageviews
                .cmp(&a.pageviews)
                .then_with(|| a.name.cmp(&b.name))
        });

        for p in prods {
            let pv_str = if p.pageviews > 0 {
                p.pageviews.to_string().green().bold()
            } else {
                "0".dimmed()
            };
            let vis_str = if p.visitors > 0 {
                p.visitors.to_string().cyan()
            } else {
                "0".dimmed()
            };

            println!(
                "  {:<16} {:<30} {:<12} {:<12}",
                p.product_id.dimmed(),
                p.name.bold(),
                pv_str,
                vis_str
            );
        }
    }

    // Top Pages
    if !summary.top_pages.is_empty() {
        println!("\n  {}", "Top Visited Paths:".bold().underline());
        for (page, count) in summary.top_pages.iter().take(cmd.limit) {
            println!("  • {:<50} {} views", page.cyan(), count.to_string().bold());
        }
    }

    // Top Referrers
    if !summary.top_referrers.is_empty() {
        println!("\n  {}", "Top Referrers:".bold().underline());
        for (ref_url, count) in summary.top_referrers.iter().take(cmd.limit) {
            println!(
                "  • {:<50} {} referrals",
                ref_url.yellow(),
                count.to_string().bold()
            );
        }
    }

    // Browsers & OS breakdown
    if !summary.browser_breakdown.is_empty() || !summary.os_breakdown.is_empty() {
        println!("\n  {}", "Environment Stats:".bold().underline());
        if !summary.browser_breakdown.is_empty() {
            let b_str: Vec<String> = summary
                .browser_breakdown
                .iter()
                .take(5)
                .map(|b| format!("{}: {:.1}%", b.label, b.pct))
                .collect();
            println!("  Browsers : {}", b_str.join("  |  ").dimmed());
        }
        if !summary.os_breakdown.is_empty() {
            let os_str: Vec<String> = summary
                .os_breakdown
                .iter()
                .take(5)
                .map(|o| format!("{}: {:.1}%", o.label, o.pct))
                .collect();
            println!("  OS       : {}", os_str.join("  |  ").dimmed());
        }
    }

    println!("\n  {} Embed script in your HTML: {}", "Tip:".cyan().bold(), r#"<script defer src="https://cashell.fathforce.com/tracker.js" data-site="<id>"></script>"#.dimmed());
    println!();

    Ok(())
}

fn do_export(format: &str, out: &std::path::Path) -> Result<()> {
    let db = crate::db::Database::open()?;
    let out_str = out.to_string_lossy();
    let fmt_lower = format.to_lowercase();
    match fmt_lower.as_str() {
        "csv" => {
            let count = db
                .export_analytics_csv(&out_str)
                .with_context(|| format!("Failed to export CSV to {out_str}"))?;
            println!(
                "✅ Successfully exported {} analytics events to CSV: {}",
                count,
                out_str.green()
            );
        }
        "json" => {
            let count = db
                .export_analytics_json(&out_str)
                .with_context(|| format!("Failed to export JSON to {out_str}"))?;
            println!(
                "✅ Successfully exported {} analytics events to JSON: {}",
                count,
                out_str.green()
            );
        }
        other => {
            bail!("Unsupported export format '{other}'. Supported formats: 'csv', 'json'.");
        }
    }
    Ok(())
}
