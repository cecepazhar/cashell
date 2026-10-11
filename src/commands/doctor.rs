//! System diagnostic command — `cashell doctor` (REQ-P2-01).
//!
//! Checks Tailscale mesh reachability, the local port matrix, storage health,
//! and the developer toolchain. Never panics; absence is reported as Warn.

use crate::cli::DoctorCommand;
use crate::output;
use anyhow::Result;
use colored::Colorize;
use serde::Serialize;
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

const VPS_ADDR: &str = "100.76.150.46";
const PORTS: [u16; 8] = [8081, 8082, 8083, 8084, 8085, 8086, 8087, 8088];

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "level", content = "detail", rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn(String),
    Fail(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub component: String,
    pub status: Status,
}

impl CheckResult {
    fn ok(component: &str) -> Self {
        Self {
            component: component.to_string(),
            status: Status::Pass,
        }
    }
    fn warn(component: &str, detail: impl Into<String>) -> Self {
        Self {
            component: component.to_string(),
            status: Status::Warn(detail.into()),
        }
    }
    fn fail(component: &str, detail: impl Into<String>) -> Self {
        Self {
            component: component.to_string(),
            status: Status::Fail(detail.into()),
        }
    }
    fn detail(&self) -> &str {
        match &self.status {
            Status::Pass => "",
            Status::Warn(d) | Status::Fail(d) => d,
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<CheckResult>,
    pub healthy: bool,
    pub warnings: usize,
    pub failures: usize,
}

impl DoctorReport {
    fn from_checks(checks: Vec<CheckResult>) -> Self {
        let warnings = checks
            .iter()
            .filter(|c| matches!(c.status, Status::Warn(_)))
            .count();
        let failures = checks
            .iter()
            .filter(|c| matches!(c.status, Status::Fail(_)))
            .count();
        let healthy = warnings == 0 && failures == 0;
        Self {
            checks,
            healthy,
            warnings,
            failures,
        }
    }

    pub fn exit_code(&self) -> i32 {
        if self.failures > 0 {
            2
        } else if self.warnings > 0 {
            1
        } else {
            0
        }
    }
}

/// TCP probe with a bounded timeout. Returns Some(rtt) if connected.
fn probe(addr: SocketAddr, timeout: Duration) -> Option<Duration> {
    let start = Instant::now();
    TcpStream::connect_timeout(&addr, timeout).ok()?;
    Some(start.elapsed())
}

fn check_tailscale() -> CheckResult {
    let has_socket = std::path::Path::new("/var/run/tailscale/tailscaled.sock").exists();
    let status_ok = Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !status_ok {
        let detail = if has_socket {
            "socket present but `tailscale status` failed"
        } else {
            "tailscale daemon not detected — mesh checks skipped"
        };
        return CheckResult::warn("Tailscale", detail);
    }

    // Mesh reachable: measure latency to the VPS.
    let addr: SocketAddr = match format!("{VPS_ADDR}:443").parse() {
        Ok(a) => a,
        Err(_) => return CheckResult::warn("Tailscale", "invalid VPS address"),
    };
    match probe(addr, Duration::from_millis(500)) {
        Some(rtt) => CheckResult::ok(&format!(
            "Tailscale — VPS {VPS_ADDR} reachable in {}ms",
            rtt.as_millis()
        )),
        None => CheckResult::warn(
            "Tailscale",
            format!("daemon up but VPS {VPS_ADDR} unreachable (500ms timeout)"),
        ),
    }
}

fn check_ports() -> CheckResult {
    let open: Vec<u16> = PORTS
        .iter()
        .copied()
        .filter(|&p| {
            probe(
                SocketAddr::from(([127, 0, 0, 1], p)),
                Duration::from_millis(50),
            )
            .is_some()
        })
        .collect();

    if open.is_empty() {
        CheckResult::warn("Ports 8081-8088", "no local listeners detected")
    } else {
        let list = open
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        CheckResult::ok(&format!("Ports 8081-8088 — listening: {list}"))
    }
}

/// Parse `df -P` / `df -Pi` capacity percentage for a mount point.
fn df_percent(path: &str, inodes: bool) -> Option<f64> {
    let mut cmd = Command::new("df");
    cmd.arg(if inodes { "-Pi" } else { "-P" }).arg(path);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?; // skip header
    let pct = line.split_whitespace().find(|f| f.ends_with('%'))?;
    pct.trim_end_matches('%').parse().ok()
}

fn check_storage() -> Vec<CheckResult> {
    let mut results = Vec::new();
    for mount in ["/", "/run/media/cecepazhar/DATA"] {
        let blocks = df_percent(mount, false);
        let inodes = df_percent(mount, true);
        match (blocks, inodes) {
            (Some(b), Some(i)) => {
                if mount == "/" && b > 95.0 {
                    results.push(CheckResult::fail(
                        &format!("Storage {mount}"),
                        format!("{b:.0}% blocks used — critical"),
                    ));
                } else if b > 85.0 || i > 85.0 {
                    results.push(CheckResult::warn(
                        &format!("Storage {mount}"),
                        format!("{b:.0}% blocks, {i:.0}% inodes — above 85%"),
                    ));
                } else {
                    results.push(CheckResult::ok(&format!(
                        "Storage {mount} — {b:.0}% blocks, {i:.0}% inodes"
                    )));
                }
            }
            _ => results.push(CheckResult::warn(
                &format!("Storage {mount}"),
                "unable to read filesystem capacity",
            )),
        }
    }
    results
}

fn check_toolchain() -> Vec<CheckResult> {
    ["cargo", "rustc", "go", "templ", "pnpm"]
        .iter()
        .map(|tool| {
            let out = Command::new(tool).arg("--version").output();
            match out {
                Ok(o) if o.status.success() => {
                    let v = String::from_utf8_lossy(&o.stdout);
                    let line = v.lines().next().unwrap_or("unknown").trim();
                    CheckResult::ok(&format!("{tool} — {line}"))
                }
                _ => CheckResult::warn(tool, format!("{tool} not found in PATH")),
            }
        })
        .collect()
}

fn print_table(report: &DoctorReport) {
    println!("\n  {:<8} {:<40} DETAIL", "STATUS", "COMPONENT");
    println!("  {}", "─".repeat(78));
    for c in &report.checks {
        let (mark, colored) = match &c.status {
            Status::Pass => ("✔", "PASS".green().to_string()),
            Status::Warn(_) => ("▲", "WARN".yellow().to_string()),
            Status::Fail(_) => ("✖", "FAIL".red().to_string()),
        };
        println!(
            "  {:<8} {:<40} {}",
            format!("{mark} {colored}"),
            c.component,
            c.detail()
        );
    }
    println!(
        "\n  {} healthy, {} warnings, {} failures\n",
        if report.healthy {
            "all".green().to_string()
        } else {
            "not all".to_string()
        },
        report.warnings.to_string().yellow(),
        report.failures.to_string().red()
    );
}

pub fn run(cmd: &DoctorCommand) -> Result<i32> {
    let mut checks = vec![check_tailscale(), check_ports()];
    checks.extend(check_storage());
    checks.extend(check_toolchain());

    let report = DoctorReport::from_checks(checks);
    let is_json = cmd.json || !output::is_tty();

    if is_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_table(&report);
    }

    Ok(report.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_mapping() {
        assert_eq!(
            DoctorReport::from_checks(vec![CheckResult::ok("x")]).exit_code(),
            0
        );
        assert_eq!(
            DoctorReport::from_checks(vec![CheckResult::warn("x", "w")]).exit_code(),
            1
        );
        assert_eq!(
            DoctorReport::from_checks(vec![
                CheckResult::warn("x", "w"),
                CheckResult::fail("y", "f")
            ])
            .exit_code(),
            2
        );
    }

    #[test]
    fn live_probe_never_panics() {
        // Port 1 is reserved and never listening; result must be None, not a panic.
        let _ = probe(
            SocketAddr::from(([127, 0, 0, 1], 1)),
            Duration::from_millis(50),
        );
    }

    #[test]
    fn storage_checks_do_not_panic() {
        let results = check_storage();
        assert_eq!(results.len(), 2);
    }
}
