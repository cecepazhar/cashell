//! Product command — list/query/browse CA products.

use crate::{config, output, tui};
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Subcommand, Debug)]
pub enum ProductAction {
    /// List all CA products (plain table / JSON)
    List,
    /// Show details for a specific product (by slug)
    Info { slug: String },
    /// Interactive TUI browser (ratatui)
    Browse,
}

#[derive(Parser, Debug)]
pub struct ProductCommand {
    #[command(subcommand)]
    pub action: ProductAction,
}

#[derive(Debug, Clone)]
pub struct ProductMeta {
    pub slug: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub price_range: &'static str,
    pub status: &'static str,
}

/// All CA products — used by list/info commands and the TUI browser.
pub fn catalog() -> &'static [ProductMeta] {
    &[
        ProductMeta {
            slug: "caterm",
            name: "CATerm",
            description: "AI-powered terminal & knowledge management platform",
            price_range: "$0 / $10/mo Pro",
            status: "active",
        },
        ProductMeta {
            slug: "caui",
            name: "CAUI",
            description: "Component library — Svelte 5 + Tailwind CSS v4 design system",
            price_range: "Open Source (MIT)",
            status: "active",
        },
        ProductMeta {
            slug: "caframework",
            name: "CAFramework",
            description: "Cross-platform desktop framework (Rust + Tauri)",
            price_range: "Open Source",
            status: "active",
        },
        ProductMeta {
            slug: "camark",
            name: "CAMark",
            description: "Zero-knowledge markdown vault with FTS5 search",
            price_range: "$0 / $5/mo Pro",
            status: "active",
        },
        ProductMeta {
            slug: "castudio",
            name: "CAStudio",
            description: "AI creative suite — image, audio, video generation",
            price_range: "Beta",
            status: "beta",
        },
        ProductMeta {
            slug: "cacash",
            name: "CACash",
            description: "Peer-to-peer cashless micro-payment system",
            price_range: "Open Source",
            status: "active",
        },
    ]
}

pub fn run(cmd: &ProductCommand) -> Result<()> {
    match &cmd.action {
        ProductAction::List => cmd_list(),
        ProductAction::Info { slug } => cmd_info(slug),
        ProductAction::Browse => tui::run_product_browser(),
    }
}

fn cmd_list() -> Result<()> {
    // Find default catalog preference
    let _cfg = config::Config::load(None)?;
    let items = catalog();

    if output::is_tty() {
        // Table mode
        println!(
            "\n  {:<16} {:<20} {:<12} {}",
            "SLUG", "NAME", "STATUS", "PRICE"
        );
        println!("  {}", "─".repeat(62));
        for p in items {
            println!(
                "  {:<16} {:<20} {:<12} {}",
                p.slug, p.name, p.status, p.price_range
            );
        }
        println!("\n  Total: {} product(s)\n", items.len());
    } else {
        // JSON mode
        let rows: Vec<serde_json::Value> = items
            .iter()
            .map(|p| {
                serde_json::json!({
                    "slug": p.slug,
                    "name": p.name,
                    "description": p.description,
                    "price_range": p.price_range,
                    "status": p.status
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    }
    Ok(())
}

fn cmd_info(slug: &str) -> Result<()> {
    match catalog().iter().find(|p| p.slug == slug) {
        Some(p) => {
            output::kv("slug", p.slug);
            output::kv("name", p.name);
            output::kv("description", p.description);
            output::kv("price_range", p.price_range);
            output::kv("status", p.status);
        }
        None => anyhow::bail!("Product not found: {slug}"),
    }
    Ok(())
}
