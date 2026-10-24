//! `cashell analytics` command — view sovereign web analytics & traffic summary across CA ecosystem sites.

use anyhow::Result;
use clap::Parser;
use colored::Colorize;

#[derive(Parser, Debug, Clone)]
#[command(name = "analytics", about = "View sovereign web analytics & traffic summary across CA ecosystem sites")]
pub struct AnalyticsCommand {
    /// Filter by website ID (e.g. 'docs', 'blog', 'caterm', 'fathforce')
    #[arg(short, long)]
    pub site: Option<String>,
    /// Limit number of top pages / referrers to show (default: 10)
    #[arg(short, long, default_value = "10")]
    pub limit: usize,
}

pub fn run(cmd: &AnalyticsCommand) -> Result<()> {
    let db = crate::db::Database::open()?;
    let _ = db.import_default_ca_websites();

    let summary = db.get_analytics_summary(cmd.site.as_deref())?;

    let filter_label = match &cmd.site {
        Some(s) => format!(" (Filtered by site: {})", s.cyan()),
        None => " (All Registered CA Sites)".to_string(),
    };

    println!("\n{}", format!("📊 Sovereign Web Analytics{}", filter_label).bold());
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
        prods.sort_by(|a, b| b.pageviews.cmp(&a.pageviews).then_with(|| a.name.cmp(&b.name)));

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
            println!("  • {:<50} {} referrals", ref_url.yellow(), count.to_string().bold());
        }
    }

    // Browsers & OS breakdown
    if !summary.browser_breakdown.is_empty() || !summary.os_breakdown.is_empty() {
        println!("\n  {}", "Environment Stats:".bold().underline());
        if !summary.browser_breakdown.is_empty() {
            let b_str: Vec<String> = summary.browser_breakdown.iter().take(5)
                .map(|b| format!("{}: {:.1}%", b.label, b.pct))
                .collect();
            println!("  Browsers : {}", b_str.join("  |  ").dimmed());
        }
        if !summary.os_breakdown.is_empty() {
            let os_str: Vec<String> = summary.os_breakdown.iter().take(5)
                .map(|o| format!("{}: {:.1}%", o.label, o.pct))
                .collect();
            println!("  OS       : {}", os_str.join("  |  ").dimmed());
        }
    }

    println!("\n  {} Embed script in your HTML: {}", "Tip:".cyan().bold(), r#"<script defer src="https://cashell.fathforce.com/tracker.js" data-site="<id>"></script>"#.dimmed());
    println!();

    Ok(())
}
