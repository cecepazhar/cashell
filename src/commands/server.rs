//! `cashell server` command group — stats, db, firewall, docker, tui, ai.

use crate::config::Config;
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;

// ── CLI types ─────────────────────────────────────────────────────────────────

#[derive(Parser, Debug, Clone)]
#[command(name = "server", about = "Server management: stats, db, firewall, docker, ai")]
pub struct ServerCommand {
    #[command(subcommand)]
    pub action: ServerAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ServerAction {
    /// Print CPU, RAM, disk, and network stats
    Stats,

    /// Database operations: install, status, backup, restore
    Db {
        #[command(subcommand)]
        action: DbAction,
    },

    /// Firewall rule via ufw (allow/deny <port>)
    Firewall {
        /// Action: allow or deny
        action: String,
        /// Port number
        port: u16,
    },

    /// Docker container management
    Docker {
        /// Action: ps | start | stop | restart | logs
        action: String,
        /// Container name (required for start/stop/restart/logs)
        name: Option<String>,
    },

    /// Launch server TUI dashboard
    Tui,

    /// Ask AI about server operations
    Ai {
        /// Prompt (all remaining words)
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum DbAction {
    /// Install a database engine via apt-get
    Install {
        /// Engine: postgresql | mysql
        engine: String,
    },
    /// Show active status of postgresql and mysql
    Status,
    /// Dump a database to /var/backups/csp/<name>-<ts>.sql.gz
    Backup {
        /// Database name
        name: String,
    },
    /// Restore a database from a dump file
    Restore {
        /// Database name
        name: String,
        /// Path to the dump file
        file: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(cmd: &ServerCommand, cfg: &Config) -> Result<()> {
    match &cmd.action {
        ServerAction::Stats => run_stats(),
        ServerAction::Db { action } => run_db(action),
        ServerAction::Firewall { action, port } => run_firewall(action, *port),
        ServerAction::Docker { action, name } => run_docker(action, name.as_deref()),
        ServerAction::Tui => {
            println!("{}", "[TUI coming soon — caorch tui for dashboard]".yellow());
            Ok(())
        }
        ServerAction::Ai { prompt } => run_ai(prompt, cfg),
    }
}

// ── Stats ─────────────────────────────────────────────────────────────────────

fn run_stats() -> Result<()> {
    use sysinfo::System;

    let mut sys = System::new_all();
    sys.refresh_all();

    // CPU
    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    let cpu_count = sys.cpus().len();
    let cpu_avg: f32 = sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>()
        / cpu_count.max(1) as f32;

    // RAM
    let ram_used = sys.used_memory();   // bytes
    let ram_total = sys.total_memory();
    let ram_pct = (ram_used * 100).checked_div(ram_total).unwrap_or(0);

    // Disk
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    let (disk_used, disk_total) = disks.iter().fold((0u64, 0u64), |(u, t), d| {
        (
            u + (d.total_space() - d.available_space()),
            t + d.total_space(),
        )
    });
    let disk_pct = (disk_used * 100).checked_div(disk_total).unwrap_or(0);

    // Network
    use sysinfo::Networks;
    let mut nets = Networks::new_with_refreshed_list();
    std::thread::sleep(std::time::Duration::from_millis(500));
    nets.refresh();
    let (rx_total, tx_total) = nets.iter().fold((0u64, 0u64), |(r, t), (_, n)| {
        (r + n.total_received(), t + n.total_transmitted())
    });

    println!("{}", "── Server Stats ─────────────────────────────".bold());
    println!(
        "  {}  {:.1}%  ({} cores)",
        "CPU:".cyan().bold(),
        cpu_avg,
        cpu_count
    );
    println!(
        "  {}  {} / {}  ({}%)",
        "RAM:".cyan().bold(),
        fmt_bytes(ram_used),
        fmt_bytes(ram_total),
        ram_pct
    );
    println!(
        "  {}  {} / {}  ({}%)",
        "DSK:".cyan().bold(),
        fmt_bytes(disk_used),
        fmt_bytes(disk_total),
        disk_pct
    );
    println!(
        "  {}  ↓ {}  ↑ {}  (since boot)",
        "NET:".cyan().bold(),
        fmt_bytes(rx_total),
        fmt_bytes(tx_total)
    );
    Ok(())
}

fn fmt_bytes(b: u64) -> String {
    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    const KIB: u64 = 1 << 10;
    if b >= GIB {
        format!("{:.2} GiB", b as f64 / GIB as f64)
    } else if b >= MIB {
        format!("{:.1} MiB", b as f64 / MIB as f64)
    } else if b >= KIB {
        format!("{:.0} KiB", b as f64 / KIB as f64)
    } else {
        format!("{} B", b)
    }
}

// ── Db ────────────────────────────────────────────────────────────────────────

fn run_db(action: &DbAction) -> Result<()> {
    match action {
        DbAction::Install { engine } => db_install(engine),
        DbAction::Status => db_status(),
        DbAction::Backup { name } => db_backup(name),
        DbAction::Restore { name, file } => db_restore(name, file),
    }
}

fn db_install(engine: &str) -> Result<()> {
    let pkg = match engine.to_lowercase().as_str() {
        "postgresql" | "postgres" | "pg" => "postgresql",
        "mysql" => "mysql-server",
        "mariadb" => "mariadb-server",
        other => bail!("unknown engine '{}'; supported: postgresql, mysql, mariadb", other),
    };
    let cmd = format!("apt-get install -y {pkg}");
    println!("{} {}", "▶ Running:".bold(), cmd.yellow());
    let status = std::process::Command::new("apt-get")
        .args(["install", "-y", pkg])
        .status()?;
    if !status.success() {
        bail!("apt-get install failed (exit {:?})", status.code());
    }
    println!("{}", format!("✓ {pkg} installed").green());
    Ok(())
}

fn db_status() -> Result<()> {
    for svc in ["postgresql", "mysql"] {
        let out = std::process::Command::new("systemctl")
            .args(["is-active", svc])
            .output();
        let state = match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(_) => "unknown".into(),
        };
        let colored_state = if state == "active" {
            state.green().to_string()
        } else {
            state.red().to_string()
        };
        println!("  {} {}", format!("{svc}:").cyan().bold(), colored_state);
    }
    Ok(())
}

fn db_backup(name: &str) -> Result<()> {
    use std::path::Path;
    let backup_dir = "/var/backups/csp";
    std::fs::create_dir_all(backup_dir)?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let file = format!("{backup_dir}/{name}-{ts}.sql.gz");

    // Detect engine by checking if pg_dump exists; fall back to mysqldump.
    let use_pg = Path::new("/usr/bin/pg_dump")
        .exists()
        || Path::new("/usr/lib/postgresql").exists();

    let cmd_str = if use_pg {
        format!("pg_dump {name} | gzip > {file}")
    } else {
        format!("mysqldump {name} | gzip > {file}")
    };
    println!("{} {}", "▶ Running:".bold(), cmd_str.yellow());

    let status = std::process::Command::new("sh")
        .args(["-c", &cmd_str])
        .status()?;
    if !status.success() {
        bail!("backup command failed (exit {:?})", status.code());
    }
    println!("{}", format!("✓ backup written to {file}").green());
    Ok(())
}

fn db_restore(name: &str, file: &str) -> Result<()> {
    if !std::path::Path::new(file).exists() {
        bail!("file not found: {file}");
    }
    let use_pg = std::path::Path::new("/usr/bin/psql").exists();
    let cmd_str = if use_pg {
        format!("zcat {file} | psql {name}")
    } else {
        format!("zcat {file} | mysql {name}")
    };
    println!("{} {}", "▶ Running:".bold(), cmd_str.yellow());
    let status = std::process::Command::new("sh")
        .args(["-c", &cmd_str])
        .status()?;
    if !status.success() {
        bail!("restore command failed (exit {:?})", status.code());
    }
    println!("{}", format!("✓ restored {file} → {name}").green());
    Ok(())
}

// ── Firewall ──────────────────────────────────────────────────────────────────

fn run_firewall(action: &str, port: u16) -> Result<()> {
    let action_lc = action.to_lowercase();
    if action_lc != "allow" && action_lc != "deny" {
        bail!("action must be 'allow' or 'deny', got '{action}'");
    }
    let port_str = port.to_string();
    println!(
        "{} ufw {} {}",
        "▶ Running:".bold(),
        action_lc.yellow(),
        port_str.yellow()
    );
    let status = std::process::Command::new("ufw")
        .args([action_lc.as_str(), &port_str])
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("{}", format!("✓ ufw {action_lc} {port}").green());
            Ok(())
        }
        Ok(s) => bail!("ufw exited with {:?}", s.code()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("ufw not found — install with: sudo apt-get install ufw");
        }
        Err(e) => bail!("failed to run ufw: {e}"),
    }
}

// ── Docker ────────────────────────────────────────────────────────────────────

fn run_docker(action: &str, name: Option<&str>) -> Result<()> {
    let action_lc = action.to_lowercase();
    let args: Vec<&str> = match action_lc.as_str() {
        "ps" => vec!["ps"],
        "start" | "stop" | "restart" | "logs" => {
            let n = name.ok_or_else(|| {
                anyhow::anyhow!("action '{action}' requires a container name")
            })?;
            vec![action_lc.as_str(), n]
        }
        other => bail!(
            "unknown docker action '{other}'; supported: ps, start, stop, restart, logs"
        ),
    };

    // Build args vec owned to avoid lifetime issues
    let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    println!(
        "{} docker {}",
        "▶ Running:".bold(),
        args_owned.join(" ").yellow()
    );

    let out = std::process::Command::new("docker")
        .args(&args_owned)
        .output();

    match out {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "{}",
                "✗ docker not found — install Docker: https://docs.docker.com/engine/install/"
                    .red()
                    .bold()
            );
            Ok(())
        }
        Err(e) => bail!("failed to run docker: {e}"),
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stdout.is_empty() {
                print!("{stdout}");
            }
            if !stderr.is_empty() {
                eprint!("{stderr}");
            }
            if !o.status.success() {
                bail!("docker exited with {:?}", o.status.code());
            }
            Ok(())
        }
    }
}

// ── AI ────────────────────────────────────────────────────────────────────────

fn run_ai(prompt: &[String], cfg: &Config) -> Result<()> {
    let user_prompt = format!("[Server Ops] {}", prompt.join(" "));
    let answer = crate::ai::query(cfg, None, &user_prompt)?;
    // query() streams on TTY and buffers in pipe; only print when buffered.
    if !crate::output::is_tty() {
        println!("{answer}");
    }
    Ok(())
}
