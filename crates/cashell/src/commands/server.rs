//! `cashell server` command group — stats, db, firewall, docker, tui, ai.

use crate::config::Config;
use crate::server::FirewallRunner;
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;

// ── CLI types ─────────────────────────────────────────────────────────────────

#[derive(Parser, Debug, Clone)]
#[command(
    name = "server",
    about = "Server management: stats, db, firewall, docker, ai"
)]
pub struct ServerCommand {
    #[command(subcommand)]
    pub action: ServerAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ServerAction {
    /// Print CPU, RAM, disk, and network stats
    Stats,

    /// SSD / NVMe storage SMART health & CrystalDisk diagnostic
    Disk,

    /// Database operations: install, status, backup, restore
    Db {
        #[command(subcommand)]
        action: DbAction,
    },

    /// Caddy virtual host reverse proxy & automatic SSL management (SV-01)
    Domain {
        #[command(subcommand)]
        action: DomainAction,
    },

    /// Firewall rule via ufw (status, allow, deny, delete) (SV-02)
    Firewall {
        #[command(subcommand)]
        action: FirewallAction,
    },

    /// SSH brute-force auto-ban inspection and unban operations (SV-03)
    Autoban {
        #[command(subcommand)]
        action: AutobanAction,
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

#[derive(Subcommand, Debug, Clone)]
pub enum DomainAction {
    /// Add reverse proxy domain with automated SSL (SV-01)
    Add {
        /// Domain name (e.g. app.example.com)
        domain: String,
        /// Upstream proxy port
        #[arg(short, long)]
        proxy: u16,
    },
    /// Remove reverse proxy domain
    Remove {
        /// Domain name to remove
        domain: String,
    },
    /// List all configured virtual host domains
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum FirewallAction {
    /// Show numbered firewall rules (`ufw status numbered`) (SV-02)
    List,
    /// Allow incoming traffic on port (SV-02)
    Allow {
        /// Port number
        port: u16,
        /// Optional protocol (tcp or udp)
        #[arg(short, long)]
        proto: Option<String>,
    },
    /// Deny incoming traffic on port (SV-02)
    Deny {
        /// Port number
        port: u16,
        /// Optional protocol (tcp or udp)
        #[arg(short, long)]
        proto: Option<String>,
    },
    /// Delete rule by number or port specification (SV-02)
    Delete {
        /// Rule number or specification (e.g. '1' or '80')
        rule: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AutobanAction {
    /// Inspect recent SSH authentication logs and auto-ban brute-force attackers (SV-03)
    Check {
        /// Number of recent log lines to inspect
        #[arg(short, long, default_value_t = 200)]
        lines: usize,
    },
    /// List currently banned IP addresses (SV-03)
    List {
        /// Include historical unbanned records
        #[arg(short, long)]
        all: bool,
    },
    /// Manually unban an IP address (SV-03)
    Unban {
        /// IP address to unban
        ip: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(cmd: &ServerCommand, cfg: &Config) -> Result<()> {
    match &cmd.action {
        ServerAction::Stats => run_stats(),
        ServerAction::Disk => run_disk_health(),
        ServerAction::Db { action } => run_db(action),
        ServerAction::Domain { action } => run_domain(action),
        ServerAction::Firewall { action } => run_firewall(action),
        ServerAction::Autoban { action } => run_autoban(action),
        ServerAction::Docker { action, name } => run_docker(action, name.as_deref()),
        ServerAction::Tui => {
            println!(
                "{}",
                "[TUI coming soon — caorch tui for dashboard]".yellow()
            );
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
    let cpu_avg: f32 =
        sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>() / cpu_count.max(1) as f32;

    // RAM
    let ram_used = sys.used_memory(); // bytes
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

// ── Disk Health ───────────────────────────────────────────────────────────────

fn run_disk_health() -> Result<()> {
    use crate::systat::probe_storage_health;

    let disks = probe_storage_health();

    println!(
        "{}",
        "── CrystalDisk SSD / NVMe SMART Health ─────────────────────".bold()
    );
    if disks.is_empty() {
        println!(
            "  {}",
            "No physical drives detected (SMART permissions unavailable).".yellow()
        );
        return Ok(());
    }

    for d in &disks {
        let health_color = if d.health_status.contains("CRITICAL") {
            d.health_status.red().to_string()
        } else if d.health_status.contains("WARNING") {
            d.health_status.yellow().to_string()
        } else {
            d.health_status.green().to_string()
        };

        let temp_color = if d.temperature_c <= 0.0 {
            "N/A".normal().to_string()
        } else if d.temperature_c >= 75.0 {
            format!("{:.1}°C", d.temperature_c).red().to_string()
        } else if d.temperature_c >= 60.0 {
            format!("{:.1}°C", d.temperature_c).yellow().to_string()
        } else {
            format!("{:.1}°C", d.temperature_c).green().to_string()
        };

        let bar_len = 16usize;
        let filled = ((d.wear_level_pct as usize * bar_len) / 100).min(bar_len);
        let bar = "█".repeat(filled) + &"░".repeat(bar_len - filled);
        let bar_color = if d.wear_level_pct >= 80 {
            bar.red().to_string()
        } else if d.wear_level_pct >= 50 {
            bar.yellow().to_string()
        } else {
            bar.green().to_string()
        };

        println!(
            "\n  {}  [{} · {}]  ⏚ {}",
            d.device.cyan().bold(),
            health_color,
            temp_color,
            d.model
        );
        println!(
            "      Wear: {} [{}] {}% │ TBW: {:.2} TB │ Read: {:.2} TB",
            "●".cyan(),
            bar_color,
            d.wear_level_pct,
            d.written_bytes_tb,
            d.read_bytes_tb
        );
        println!(
            "      Power On: {}h (~{}d) │ Cycles: {} │ Unsafe Shutdowns: {} │ Warnings: {}",
            d.power_on_hours,
            d.power_on_hours / 24,
            d.power_cycles,
            d.unsafe_shutdowns,
            d.critical_warnings
        );
    }

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
        other => bail!(
            "unknown engine '{}'; supported: postgresql, mysql, mariadb",
            other
        ),
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

/// H-08: validate a database name — must match ^[a-zA-Z0-9_]{1,63}$.
/// Prevents filename/shell injection via backup/restore command interpolation.
fn is_valid_db_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn db_backup(name: &str) -> Result<()> {
    use std::path::Path;

    // H-08: validate DB name before it is interpolated into any file path.
    if !is_valid_db_name(name) {
        bail!("invalid database name '{name}' — must match ^[a-zA-Z0-9_]{{1,63}}$");
    }

    let backup_dir = "/var/backups/csp";
    std::fs::create_dir_all(backup_dir)?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let file = format!("{backup_dir}/{name}-{ts}.sql.gz");

    // Detect engine by checking if pg_dump exists; fall back to mysqldump.
    let use_pg =
        Path::new("/usr/bin/pg_dump").exists() || Path::new("/usr/lib/postgresql").exists();

    // H-08: argv-based process chain — pg_dump/mysqldump stdout is piped into
    // `gzip -f` via OS stdio pipes; no shell is involved at all.
    let dump_bin = if use_pg { "pg_dump" } else { "mysqldump" };
    println!(
        "{} {} → {}",
        "▶ Running:".bold(),
        format!("{dump_bin} {name} | gzip -f").yellow(),
        file.yellow()
    );

    let mut dump = std::process::Command::new(dump_bin)
        .arg(name)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn {dump_bin}: {e}"))?;
    let dump_stdout = dump
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture stdout pipe from {dump_bin}"))?;

    // gzip -c compresses its stdin and writes to the stdout we redirect to
    // the backup file; `-f` overwrites an existing backup file.
    let backup_out = std::fs::File::create(&file)
        .map_err(|e| anyhow::anyhow!("failed to create backup file {file}: {e}"))?;
    let mut gzip = std::process::Command::new("gzip")
        .args(["-c", "-f"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::from(backup_out))
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn gzip: {e}"))?;
    let mut gzip_stdin = gzip
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture gzip stdin pipe"))?;

    // Feed dump stdout into gzip stdin; the join handle's receiver is dropped
    // so gzip receives EOF once the copy finishes.
    let feeder = std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::BufReader::new(dump_stdout), &mut gzip_stdin);
    });

    let dump_status = dump.wait()?;
    let _ = feeder.join();
    let gzip_status = gzip.wait()?;

    if !dump_status.success() || !gzip_status.success() {
        std::fs::remove_file(&file).ok();
        bail!(
            "backup command failed ({} exit {:?}, gzip exit {:?})",
            dump_bin,
            dump_status.code(),
            gzip_status.code()
        );
    }

    println!("{}", format!("✓ backup written to {file}").green());
    Ok(())
}

fn db_restore(name: &str, file: &str) -> Result<()> {
    // H-08: same DB name validation as db_backup.
    if !is_valid_db_name(name) {
        bail!("invalid database name '{name}' — must match ^[a-zA-Z0-9_]{{1,63}}$");
    }
    if !std::path::Path::new(file).exists() {
        bail!("file not found: {file}");
    }

    // H-08: argv-based process chain — gunzip stdout piped into psql/mysql
    // stdin; no shell involved.
    let use_pg = std::path::Path::new("/usr/bin/psql").exists();
    let db_bin = if use_pg { "psql" } else { "mysql" };
    println!(
        "{} {}",
        "▶ Running:".bold(),
        format!("zcat {file} | {db_bin} {name}").yellow()
    );

    let mut gunzip = std::process::Command::new("zcat")
        .arg(file)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn zcat: {e}"))?;
    let gunzip_stdout = gunzip
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture zcat stdout pipe"))?;

    let mut db = std::process::Command::new(db_bin)
        .arg(name)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn {db_bin}: {e}"))?;
    let mut db_stdin = db
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to capture {db_bin} stdin pipe"))?;

    let feeder = std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::BufReader::new(gunzip_stdout), &mut db_stdin);
    });

    let gunzip_status = gunzip.wait()?;
    let _ = feeder.join();
    let db_status = db.wait()?;

    if !gunzip_status.success() || !db_status.success() {
        bail!(
            "restore command failed (zcat exit {:?}, {db_bin} exit {:?})",
            gunzip_status.code(),
            db_status.code()
        );
    }
    println!("{}", format!("✓ restored {file} → {name}").green());
    Ok(())
}

// ── Domain / Caddy (SV-01) ───────────────────────────────────────────────────

fn run_domain(action: &DomainAction) -> Result<()> {
    let db = crate::db::Database::open()?;
    let caddy = crate::server::CaddyManager::default();

    match action {
        DomainAction::Add { domain, proxy } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(caddy.add_domain_route(domain, *proxy))?;
            db.upsert_server_domain(domain, *proxy, "active")?;
            println!(
                "{}",
                format!(
                    "✓ Domain {domain} configured → 127.0.0.1:{proxy} (SSL automated by Caddy)"
                )
                .green()
            );
        }
        DomainAction::Remove { domain } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(caddy.remove_domain_route(domain))?;
            let removed = db.remove_server_domain(domain)?;
            if removed {
                println!("{}", format!("✓ Domain {domain} removed").green());
            } else {
                println!(
                    "{}",
                    format!("✓ Domain {domain} removed from Caddy").yellow()
                );
            }
        }
        DomainAction::List => {
            let domains = db.list_server_domains()?;
            if domains.is_empty() {
                println!("No virtual host domains configured.");
            } else {
                println!("{:<30} {:<12} {:<15}", "DOMAIN", "PROXY PORT", "SSL STATUS");
                println!("{}", "-".repeat(60));
                for d in domains {
                    println!("{:<30} {:<12} {:<15}", d.domain, d.proxy_port, d.ssl_status);
                }
            }
        }
    }
    Ok(())
}

// ── Firewall (SV-02) ──────────────────────────────────────────────────────────

fn run_firewall(action: &FirewallAction) -> Result<()> {
    let fw = crate::server::FirewallManager::default();
    match action {
        FirewallAction::List => {
            let status = fw.list_rules()?;
            print!("{status}");
        }
        FirewallAction::Allow { port, proto } => {
            let res = fw.allow_port(*port, proto.as_deref())?;
            println!("{}", format!("✓ {res}").green());
        }
        FirewallAction::Deny { port, proto } => {
            let res = fw.deny_port(*port, proto.as_deref())?;
            println!("{}", format!("✓ {res}").green());
        }
        FirewallAction::Delete { rule } => {
            let res = fw.delete_rule(rule)?;
            println!("{}", format!("✓ {res}").green());
        }
    }
    Ok(())
}

// ── Autoban SSH (SV-03) ───────────────────────────────────────────────────────

fn run_autoban(action: &AutobanAction) -> Result<()> {
    let db = crate::db::Database::open()?;
    let fw = crate::server::SystemFirewallRunner;
    let engine = crate::server::SshAutobanEngine::default();

    match action {
        AutobanAction::Check { lines } => {
            println!(
                "{} inspecting recent {lines} SSH log entries...",
                "▶".cyan()
            );
            let logs = crate::server::SshAutobanEngine::read_recent_logs(*lines)?;
            if logs.is_empty() {
                println!("No readable SSH logs found in journalctl or auth.log.");
            } else {
                let banned = engine.process_log_content(&logs, &db, &fw)?;
                if banned.is_empty() {
                    println!(
                        "{}",
                        "✓ No brute-force threshold violations detected.".green()
                    );
                } else {
                    for ip in &banned {
                        println!("{}", format!("✓ Banned brute-force IP: {ip}").red().bold());
                    }
                }
            }
            // Check for unban expirations
            let now = chrono::Utc::now().timestamp();
            let unbanned = engine.check_and_unban_expired(&db, &fw, now)?;
            for ip in unbanned {
                println!("{}", format!("✓ Expired ban removed: {ip}").yellow());
            }
        }
        AutobanAction::List { all } => {
            let bans = db.list_ssh_bans(!all)?;
            if bans.is_empty() {
                println!("No banned IPs found.");
            } else {
                println!(
                    "{:<18} {:<10} {:<22} {:<22} {:<10}",
                    "IP", "ATTEMPTS", "BANNED AT", "UNBAN AT", "STATUS"
                );
                println!("{}", "-".repeat(86));
                for b in bans {
                    let banned_dt = chrono::DateTime::from_timestamp(b.banned_at, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| b.banned_at.to_string());
                    let unban_dt = chrono::DateTime::from_timestamp(b.unban_at, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| b.unban_at.to_string());
                    println!(
                        "{:<18} {:<10} {:<22} {:<22} {:<10}",
                        b.ip, b.attempts, banned_dt, unban_dt, b.status
                    );
                }
            }
        }
        AutobanAction::Unban { ip } => {
            let res = fw.execute_ufw(&["delete", "deny", "from", ip.trim(), "to", "any"])?;
            db.update_ssh_ban_status(ip.trim(), "unbanned")?;
            println!("{}", format!("✓ Unbanned IP {ip}: {res}").green());
        }
    }
    Ok(())
}

// ── Docker ────────────────────────────────────────────────────────────────────

fn run_docker(action: &str, name: Option<&str>) -> Result<()> {
    let action_lc = action.to_lowercase();
    let args: Vec<&str> = match action_lc.as_str() {
        "ps" => vec!["ps"],
        "start" | "stop" | "restart" | "logs" => {
            let n =
                name.ok_or_else(|| anyhow::anyhow!("action '{action}' requires a container name"))?;
            vec![action_lc.as_str(), n]
        }
        other => {
            bail!("unknown docker action '{other}'; supported: ps, start, stop, restart, logs")
        }
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

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn test_sv01_domain_action_cli_parse() {
        use clap::Parser;
        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(subcommand)]
            action: ServerAction,
        }

        let cli = TestCli::try_parse_from([
            "server",
            "domain",
            "add",
            "app.example.com",
            "--proxy",
            "3000",
        ])
        .unwrap();
        match cli.action {
            ServerAction::Domain {
                action: DomainAction::Add { domain, proxy },
            } => {
                assert_eq!(domain, "app.example.com");
                assert_eq!(proxy, 3000);
            }
            _ => panic!("Expected DomainAction::Add"),
        }
    }

    #[test]
    fn test_sv02_firewall_action_cli_parse() {
        use clap::Parser;
        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(subcommand)]
            action: ServerAction,
        }

        let cli1 =
            TestCli::try_parse_from(["server", "firewall", "allow", "443", "--proto", "tcp"])
                .unwrap();
        match cli1.action {
            ServerAction::Firewall {
                action: FirewallAction::Allow { port, proto },
            } => {
                assert_eq!(port, 443);
                assert_eq!(proto.as_deref(), Some("tcp"));
            }
            _ => panic!("Expected FirewallAction::Allow"),
        }

        let cli2 = TestCli::try_parse_from(["server", "firewall", "list"]).unwrap();
        match cli2.action {
            ServerAction::Firewall {
                action: FirewallAction::List,
            } => {}
            _ => panic!("Expected FirewallAction::List"),
        }
    }

    #[test]
    fn test_sv03_autoban_action_cli_parse() {
        use clap::Parser;
        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(subcommand)]
            action: ServerAction,
        }

        let cli =
            TestCli::try_parse_from(["server", "autoban", "check", "--lines", "100"]).unwrap();
        match cli.action {
            ServerAction::Autoban {
                action: AutobanAction::Check { lines },
            } => {
                assert_eq!(lines, 100);
            }
            _ => panic!("Expected AutobanAction::Check"),
        }
    }
}
