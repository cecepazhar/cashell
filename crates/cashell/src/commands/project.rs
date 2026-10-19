//! Project workspace command — inspect and manage project workspaces,
//! their constituent code repositories, containers, services, and databases.

use crate::config::Config;
use crate::output;
use crate::systat::{probe_tracked_projects, TrackedProject};
use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;

#[derive(Parser, Debug, Clone)]
#[command(name = "project", about = "Project workspaces: list projects and resources (repos, containers, services, DBs)")]
pub struct ProjectCommand {
    #[command(subcommand)]
    pub action: Option<ProjectAction>,

    /// Output in JSON format
    #[arg(short, long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProjectAction {
    /// List all project workspaces with their hierarchical resources (default)
    List {
        /// Show only projects with active containers or services
        #[arg(long)]
        active: bool,
    },
    /// Show detailed resource inspection for a specific project workspace
    Info {
        /// Project workspace name or directory
        name: String,
    },
    /// Track a new project workspace directory
    Add {
        /// Directory path of the workspace
        path: String,
        /// Optional display name
        #[arg(short, long)]
        name: Option<String>,
        /// Category: Product, Project, or Custom
        #[arg(short, long, default_value = "Product")]
        category: String,
    },
    /// Remove a project workspace from tracking
    Remove {
        /// Project workspace name or path
        name: String,
    },
}

pub fn run(cmd: &ProjectCommand, cfg: &Config) -> Result<i32> {
    let action = cmd.action.clone().unwrap_or(ProjectAction::List { active: false });
    match action {
        ProjectAction::List { active } => run_list(cmd.json, active),
        ProjectAction::Info { name } => run_info(&name, cmd.json),
        ProjectAction::Add { path, name, category } => run_add(&path, name.as_deref(), &category, cfg),
        ProjectAction::Remove { name } => run_remove(&name, cfg),
    }
}

fn run_list(is_json: bool, active_only: bool) -> Result<i32> {
    let mut projects = probe_tracked_projects();
    if active_only {
        projects.retain(|p| {
            p.containers.iter().any(|c| c.state == "running")
                || p.services.iter().any(|s| s.state == "up" || s.state == "online")
        });
    }

    if is_json || !output::is_tty() {
        let json_val = serde_json::to_string_pretty(&projects)?;
        println!("{json_val}");
        return Ok(0);
    }

    if projects.is_empty() {
        println!("\n  No project workspaces tracked. Run `cashell project add <path>` to add one.\n");
        return Ok(0);
    }

    println!("\n{}", "  ══════════════════════════════════════════════════════════════════════════════".cyan());
    println!("{}", "  PROJECT WORKSPACES HIERARCHY (Projects → Repos, Containers, Services, DBs)".cyan().bold());
    println!("{}\n", "  ══════════════════════════════════════════════════════════════════════════════".cyan());

    for p in &projects {
        print_project_tree(p);
    }

    println!(
        "  Total: {} workspace project(s) tracked.\n",
        projects.len()
    );

    Ok(0)
}

fn print_project_tree(p: &TrackedProject) {
    let dirty_tag = if p.is_dirty {
        "[DIRTY]".yellow().bold()
    } else {
        "[CLEAN]".green()
    };

    println!(
        "  📁 {} [{}] — {} (branch: {}, {})",
        p.name.bold().white(),
        p.category.cyan(),
        p.summary_line().bright_black(),
        p.branch.yellow(),
        dirty_tag
    );
    println!("     Root: {}", p.path.dimmed());

    // 1. Repositories
    if p.repos.is_empty() {
        println!("     ├── 📦 Repos: (none)");
    } else {
        println!("     ├── 📦 Repositories ({})", p.repos.len());
        for (i, r) in p.repos.iter().enumerate() {
            let is_last_repo = i + 1 == p.repos.len();
            let branch_str = if r.branch.is_empty() { "main".to_string() } else { r.branch.clone() };
            let r_dirty = if r.is_dirty { "● dirty".yellow() } else { "● clean".green() };
            let tree_char = if is_last_repo { "└──" } else { "├──" };
            let path_disp = if r.path.is_empty() { "root" } else { &r.path };
            println!(
                "     │   {} {} [git:{} {}] ({})",
                tree_char,
                r.name.white(),
                branch_str.yellow(),
                r_dirty,
                path_disp.dimmed()
            );
        }
    }

    // 2. Containers
    if p.containers.is_empty() {
        println!("     ├── 🐳 Containers: (none)");
    } else {
        println!("     ├── 🐳 Containers ({})", p.containers.len());
        for (i, c) in p.containers.iter().enumerate() {
            let is_last = i + 1 == p.containers.len();
            let tree_char = if is_last { "└──" } else { "├──" };
            let state_badge = match c.state.as_str() {
                "running" => "● running".green().bold(),
                "paused" => "● paused".yellow(),
                "defined" => "◌ defined".cyan(),
                _ => "✖ exited".red(),
            };
            let ports_disp = if c.ports.is_empty() { String::new() } else { format!(" ports: {}", c.ports) };
            println!(
                "     │   {} {} [{} {}]{}",
                tree_char,
                c.name.white(),
                c.image.bright_black(),
                state_badge,
                ports_disp.cyan()
            );
        }
    }

    // 3. Services
    if p.services.is_empty() {
        println!("     ├── ⚙ Services: (none)");
    } else {
        println!("     ├── ⚙ Services & Ports ({})", p.services.len());
        for (i, s) in p.services.iter().enumerate() {
            let is_last = i + 1 == p.services.len();
            let tree_char = if is_last { "└──" } else { "├──" };
            let st_badge = match s.state.as_str() {
                "up" | "online" => "● ONLINE".green().bold(),
                _ => "✖ DOWN".red(),
            };
            println!(
                "     │   {} {} ({}:{}/{} {})",
                tree_char,
                s.name.white(),
                s.host,
                s.port,
                s.protocol,
                st_badge
            );
        }
    }

    // 4. Databases
    if p.databases.is_empty() {
        println!("     └── 🗄 Databases: (none)\n");
    } else {
        println!("     └── 🗄 Databases ({})", p.databases.len());
        for (i, d) in p.databases.iter().enumerate() {
            let is_last = i + 1 == p.databases.len();
            let tree_char = if is_last { "└──" } else { "├──" };
            let db_st = match d.state.as_str() {
                "up" | "online" => "● ONLINE".green().bold(),
                "file" => "● FILE".cyan(),
                _ => "✖ STOPPED".red(),
            };
            let port_str = if d.port > 0 { format!(":{}", d.port) } else { "local".to_string() };
            println!(
                "         {} {} [{} {} {}] ({})\n",
                tree_char,
                d.name.white(),
                d.engine.to_uppercase().yellow(),
                port_str.cyan(),
                db_st,
                d.database.dimmed()
            );
        }
    }
}

fn run_info(name: &str, is_json: bool) -> Result<i32> {
    let projects = probe_tracked_projects();
    let name_lower = name.to_lowercase();
    let found = projects.iter().find(|p| {
        p.name.eq_ignore_ascii_case(name)
            || p.path.to_lowercase().ends_with(&name_lower)
            || p.name.to_lowercase().contains(&name_lower)
    });

    match found {
        Some(p) => {
            if is_json || !output::is_tty() {
                println!("{}", serde_json::to_string_pretty(p)?);
            } else {
                println!("\n  Project Workspace Details:");
                print_project_tree(p);
            }
            Ok(0)
        }
        None => {
            eprintln!("Error: Project workspace '{name}' not found");
            Ok(1)
        }
    }
}

fn run_add(path: &str, name: Option<&str>, category: &str, cfg: &Config) -> Result<i32> {
    let mut updated_cfg = cfg.clone();
    let expanded = if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            h.join(stripped).to_string_lossy().to_string()
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };

    let p = std::path::Path::new(&expanded);
    if !p.is_dir() {
        eprintln!("Error: Path '{}' is not a valid directory", expanded);
        return Ok(1);
    }

    let display_name = name
        .map(|s| s.to_string())
        .unwrap_or_else(|| p.file_name().and_then(|n| n.to_str()).unwrap_or("project").to_string());

    if !updated_cfg.workspace.custom_projects.iter().any(|cp| cp == &expanded) {
        updated_cfg.workspace.custom_projects.push(expanded.clone());
        let _ = updated_cfg.save(&Config::default_path());
    }

    if let Ok(db) = crate::db::Database::open() {
        let _ = db.add_tracked_project(&expanded, &display_name, category, "main", false);
    }

    println!("✔ Project workspace '{}' ({}) added to tracked workspaces", display_name, category);
    Ok(0)
}

fn run_remove(name: &str, cfg: &Config) -> Result<i32> {
    let mut updated_cfg = cfg.clone();
    let name_lower = name.to_lowercase();
    let mut removed = false;

    if let Some(pos) = updated_cfg.workspace.custom_projects.iter().position(|p| {
        p.eq_ignore_ascii_case(name) || p.to_lowercase().ends_with(&name_lower)
    }) {
        let removed_path = updated_cfg.workspace.custom_projects.remove(pos);
        let _ = updated_cfg.save(&Config::default_path());
        if let Ok(db) = crate::db::Database::open() {
            let _ = db.delete_tracked_project(&removed_path);
        }
        removed = true;
    }

    if let Ok(db) = crate::db::Database::open() {
        if let Ok(projs) = db.get_tracked_projects() {
            for p in projs {
                if p.name.eq_ignore_ascii_case(name) || p.path.eq_ignore_ascii_case(name) {
                    let _ = db.delete_tracked_project(&p.path);
                    removed = true;
                }
            }
        }
    }

    if removed {
        println!("✔ Removed project workspace '{}' from tracking", name);
        Ok(0)
    } else {
        eprintln!("Warning: Project workspace '{}' was not in custom tracked list", name);
        Ok(1)
    }
}
