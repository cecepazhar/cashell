//! Server daemon runtime, lifecycle management, and systemd service generation.

use anyhow::Result;
use colored::Colorize;
use rand::RngCore;
use std::net::Ipv4Addr;

use super::prober::{execute_remediation, probe_all, send_telegram_alert, ProbeStatus};
use super::routes::build_router;
use crate::config::Config;

/// True if the bind address refers to a loopback interface.
pub fn is_loopback(bind: &str) -> bool {
    match bind.parse::<Ipv4Addr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => bind == "::1" || bind == "localhost",
    }
}

pub fn is_remote_node(cfg: &Config) -> bool {
    cfg.daemon.nodes.values().any(|n| {
        n.active && !n.ip.is_empty() && n.ip != "127.0.0.1" && n.ip != "localhost" && n.ip != "::1"
    })
}

/// L-08: verify the local license against GCC when it is stale (> 7 days).
pub fn periodic_license_verify(cfg: &Config) {
    let Ok(Some(lic)) = crate::license_local::load_license() else {
        if is_remote_node(cfg) {
            eprintln!("⚠ Free tier: 1 remote fleet node(s) configured, but no active license.\nRemote fleet nodes are a Pro feature — run 'cashell license activate <KEY>'.");
        }
        return;
    };

    if !crate::license_local::should_verify_periodically(&lic) {
        return;
    }

    if cfg.license.gcc_base_url.is_empty() {
        return;
    }

    match crate::license_local::verify_license(&lic, &cfg.license.gcc_base_url) {
        Ok(()) => {
            eprintln!("ℹ License verified against GCC.");
            let ent = crate::entitlements::current();
            if !ent.is_pro {
                eprintln!("⚠ License no longer Pro (revoked/expired) — Pro features off.");
            }
        }
        Err(e) => eprintln!("⚠ License verify failed: {e} (offline grace in effect)"),
    }
}

pub fn run_daemon(cfg: &Config, port_opt: Option<u16>, interval_opt: Option<u64>) -> Result<()> {
    periodic_license_verify(cfg);

    let port = port_opt.unwrap_or(cfg.daemon.port);
    let interval = interval_opt.unwrap_or(cfg.daemon.interval_secs);
    let threshold = cfg.daemon.retry_threshold;
    let bind = cfg.daemon.bind.clone();

    if !is_loopback(&bind) && cfg.daemon.token.is_empty() {
        eprintln!(
            "Refusing to start: daemon.bind '{}' is not a loopback address but daemon.token is empty. \nSet a non-empty daemon.token in config, or use daemon.bind = \"127.0.0.1\".",
            bind
        );
        std::process::exit(2);
    }

    println!(
        "{} starting on {}:{} (interval={}s, threshold={})",
        "CAShell daemon".green().bold(),
        bind,
        port,
        interval,
        threshold
    );

    let pid_path = crate::paths::daemon_pid_path();
    if let Some(parent) = pid_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&pid_path, std::process::id().to_string()) {
        eprintln!(
            "{} failed writing PID file {}: {e}",
            "⚠".yellow(),
            pid_path.display()
        );
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let app = build_router(cfg).into_make_service_with_connect_info::<std::net::SocketAddr>();
        let addr = format!("{bind}:{port}");
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => {
                println!(
                    "{} HTTP API listening on http://{}:{port}/api/health",
                    "✔".green(),
                    bind
                );
                Some(l)
            }
            Err(e) => {
                eprintln!("{} Failed to bind HTTP API to {}: {e}", "✖".red(), addr);
                None
            }
        };

        if let Some(l) = listener {
            tokio::spawn(async move {
                if let Err(e) = axum::serve(l, app).await {
                    eprintln!("HTTP API server error: {e}");
                }
            });
        }

        // T-01: Spawn Telegram long polling worker if bot token is configured
        let tg_token = crate::secrets::SecretStore::load_default()
            .ok()
            .and_then(|s| s.telegram_bot_token)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| cfg.daemon.telegram_bot_token.clone());

        if !tg_token.trim().is_empty() {
            if let Ok(db) = crate::db::get_db() {
                let db_arc = std::sync::Arc::new(std::sync::Mutex::new(db));
                let docker = std::sync::Arc::new(crate::actions::SystemDocker);
                let chat_id = if !cfg.daemon.telegram_chat_id.trim().is_empty() {
                    Some(cfg.daemon.telegram_chat_id.clone())
                } else {
                    None
                };
                let poller = crate::telegram::TelegramPoller::new(
                    tg_token,
                    db_arc,
                    docker,
                    chat_id,
                );
                tokio::spawn(async move {
                    poller.run().await;
                });
            }
        }

        let mut fails: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
        println!("{} Service probe loop initialized.", "✔".green());

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("\nShutdown signal received. CAShell daemon stopping.");
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(interval)) => {
                    let results = probe_all(&cfg.daemon.nodes).await;
                    for (node_name, svc_name, r) in results {
                        let key = format!("{node_name}.{svc_name}");
                        if r.status == ProbeStatus::Up {
                            if fails.remove(&key).is_some() {
                                println!("{} {key} recovered (UP)", "✔".green());
                                let rec_msg = format!("Service {key} recovered (UP)");
                                if let Ok(db) = crate::db::get_db() {
                                    let _ = db.log_event("INFO", "daemon", &rec_msg);
                                }
                            }
                        } else {
                            let count = fails.entry(key.clone()).or_insert(0);
                            *count += 1;
                            println!("{} {key} down ({count} consecutive failures)", "⚠".yellow());
                            if *count == threshold {
                                let msg = format!("❌ {key} DOWN after {count} consecutive failures");

                                if let Err(e) = send_telegram_alert(
                                    &cfg.daemon.telegram_bot_token,
                                    &cfg.daemon.telegram_chat_id,
                                    &msg,
                                ).await {
                                    eprintln!("  Telegram alert error: {e}");
                                } else if !cfg.daemon.telegram_bot_token.is_empty() && !cfg.daemon.telegram_chat_id.is_empty() {
                                    println!("  Telegram alert sent");
                                }

                                if let Ok(db) = crate::db::get_db() {
                                    if let Err(e) = db.log_event("ERROR", "daemon", &msg) {
                                        eprintln!("  DB log error: {e}");
                                    }
                                }

                                if let Some(cmd) = cfg.daemon.remediation.get(&key) {
                                    if let Some(node) = cfg.daemon.nodes.get(&node_name) {
                                        println!("  Executing remediation for {key}: {cmd}");
                                        match execute_remediation(&cfg.daemon.ssh_key_path, node, &node_name, cmd).await {
                                            Ok(out) => {
                                                println!("  Remediated {key}: {}", out.trim());
                                                let rem_log = format!("Remediated {key} with `{cmd}`: {}", out.trim());
                                                if let Ok(db) = crate::db::get_db() {
                                                    let _ = db.log_event("INFO", "daemon", &rem_log);
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("  Remediation for {key} failed: {e}");
                                                let rem_fail = format!("Remediation for {key} failed: {e}");
                                                if let Ok(db) = crate::db::get_db() {
                                                    let _ = db.log_event("ERROR", "daemon", &rem_fail);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    Ok(())
}

pub fn service_unit_path(user: bool) -> std::path::PathBuf {
    if user {
        crate::paths::user_systemd_unit()
    } else {
        crate::paths::system_systemd_unit()
    }
}

pub fn generate_service_unit(user: bool) -> String {
    if user {
        r#"[Unit]
Description=CAShell 24/7 Fleet & Health Monitoring Daemon
After=network.target tailscaled.service

[Service]
Type=simple
ExecStart=%h/.local/bin/cashell daemon run
Restart=always
RestartSec=10
Environment=PATH=%h/.local/bin:/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=default.target
"#
        .to_string()
    } else {
        r#"[Unit]
Description=CAShell 24/7 Fleet & Health Monitoring Daemon
After=network.target tailscaled.service

[Service]
Type=simple
ExecStart=/usr/local/bin/cashell daemon run
Restart=always
RestartSec=10
Environment=PATH=/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=multi-user.target
"#
        .to_string()
    }
}

pub fn run_install_service(user: bool) -> Result<()> {
    let cfg_path = crate::paths::config_path();
    let mut cfg = crate::config::Config::load(None)?;
    if cfg.daemon.token.is_empty() {
        let mut buf = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut buf);
        let token = crate::pro::b64_url_encode_public(&buf);
        cfg.daemon.token = token;
        let content = toml::to_string_pretty(&cfg)?;
        std::fs::write(&cfg_path, content.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&cfg_path, std::fs::Permissions::from_mode(0o600))?;
        }
        println!(
            "{} Generated daemon token and saved to {} (0600).",
            "✔".green(),
            cfg_path.display()
        );
    }

    let unit_path = service_unit_path(user);
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = generate_service_unit(user);
    std::fs::write(&unit_path, &content)?;
    println!(
        "{} Wrote systemd service unit to: {}",
        "✔".green(),
        unit_path.display()
    );

    if user {
        let username = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        println!("Enabling lingering for user '{}'...", username);
        let _ = std::process::Command::new("loginctl")
            .args(["enable-linger", &username])
            .status();

        println!("Reloading systemd user daemon and enabling service...");
        let reload = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        if let Err(e) = reload {
            eprintln!("Warning: failed to reload user daemon: {e}");
        }

        let enable = std::process::Command::new("systemctl")
            .args(["--user", "enable", "--now", "cashell-daemon.service"])
            .status();
        match enable {
            Ok(s) if s.success() => {
                println!(
                    "{} cashell-daemon.service enabled and started.",
                    "✔".green()
                );
            }
            Ok(s) => eprintln!("systemctl returned exit code: {:?}", s.code()),
            Err(e) => eprintln!("Failed to start service: {e}"),
        }

        println!("\nService management commands:");
        println!("  systemctl --user status cashell-daemon.service");
        println!("  journalctl --user -u cashell-daemon.service -f");
    } else {
        println!("Reloading systemd and enabling system service...");
        let _ = std::process::Command::new("systemctl")
            .args(["daemon-reload"])
            .status();
        let _ = std::process::Command::new("systemctl")
            .args(["enable", "--now", "cashell-daemon.service"])
            .status();

        println!("\nService management commands:");
        println!("  sudo systemctl status cashell-daemon.service");
        println!("  sudo journalctl -u cashell-daemon.service -f");
    }
    Ok(())
}

pub fn run_stop_service(user: bool) -> Result<()> {
    let args_stop = if user {
        vec!["--user", "stop", "cashell-daemon.service"]
    } else {
        vec!["stop", "cashell-daemon.service"]
    };
    let args_disable = if user {
        vec!["--user", "disable", "cashell-daemon.service"]
    } else {
        vec!["disable", "cashell-daemon.service"]
    };

    println!("Stopping cashell-daemon.service...");
    let _ = std::process::Command::new("systemctl")
        .args(&args_stop)
        .status();

    println!("Disabling cashell-daemon.service...");
    let _ = std::process::Command::new("systemctl")
        .args(&args_disable)
        .status();

    println!(
        "{} cashell-daemon.service stopped and disabled.",
        "✔".green()
    );
    Ok(())
}
