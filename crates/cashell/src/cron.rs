//! Cron and Scheduled Job Management for cashell cockpit.
//!
//! Inspects user crontab entries, systemd timers, and local scheduled tasks.

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronEntry {
    pub schedule: String,
    pub command: String,
    pub description: String,
    pub is_active: bool,
    pub source: CronSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CronSource {
    Crontab,
    SystemdTimer,
    Database,
}

impl CronEntry {
    /// Parse crontab text output into CronEntry list.
    pub fn parse_crontab(output: &str) -> Vec<Self> {
        let mut entries = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 6 {
                let schedule = parts[0..5].join(" ");
                let command = parts[5..].join(" ");
                entries.push(Self {
                    schedule,
                    command,
                    description: "User crontab".to_string(),
                    is_active: true,
                    source: CronSource::Crontab,
                });
            }
        }
        entries
    }

    /// Read system user crontab via `crontab -l`.
    pub fn read_user_crontab() -> Result<Vec<Self>> {
        let output = std::process::Command::new("crontab").arg("-l").output();

        match output {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                Ok(Self::parse_crontab(&text))
            }
            _ => Ok(Vec::new()),
        }
    }
}

// ── CAShell-managed block in the user crontab ──────────────────────────────

pub const BLOCK_START: &str = "# >>> CAShell managed jobs — edit in CAShell (CRON tab) >>>";
pub const BLOCK_END: &str = "# <<< CAShell managed jobs <<<";

/// A job CAShell writes into the crontab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedJob {
    pub id: i64,
    pub schedule: String,
    pub command: String,
    pub description: String,
}

/// Accept `@reboot|@hourly|@daily|@weekly|@monthly|@yearly|@annually` or five
/// fields made of digits, names (jan, mon …), `*`, `,`, `-` and `/`.
pub fn validate_schedule(s: &str) -> Result<()> {
    let s = s.trim();
    if s.contains(['\n', '\r', '%']) {
        anyhow::bail!("schedule may not contain newlines or %");
    }
    if s.starts_with('@') {
        return match s {
            "@reboot" | "@hourly" | "@daily" | "@weekly" | "@monthly" | "@yearly" | "@annually" => {
                Ok(())
            }
            _ => anyhow::bail!("unknown shortcut '{s}' (use @hourly, @daily, @weekly, …)"),
        };
    }
    let fields: Vec<&str> = s.split_whitespace().collect();
    if fields.len() != 5 {
        anyhow::bail!("schedule needs 5 fields: minute hour day month weekday (e.g. 0 2 * * *)");
    }
    let ok = |f: &str| {
        !f.is_empty()
            && f.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '*' | ',' | '-' | '/'))
    };
    if let Some(bad) = fields.iter().find(|f| !ok(f)) {
        anyhow::bail!("invalid schedule field '{bad}'");
    }
    // numeric ranges for plain numbers
    let limits = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
    for (f, (lo, hi)) in fields.iter().zip(limits) {
        for n in f
            .split([',', '-', '/'])
            .filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        {
            let v: u32 = n.parse().unwrap_or(u32::MAX);
            if f.contains('/') && f.split('/').nth(1) == Some(n) {
                continue; // step value
            }
            if v < lo || v > hi {
                anyhow::bail!("'{n}' is out of range {lo}-{hi} in field '{f}'");
            }
        }
    }
    Ok(())
}

/// Commands go into one crontab line: no newlines.
pub fn validate_command(c: &str) -> Result<()> {
    if c.trim().is_empty() {
        anyhow::bail!("command is empty");
    }
    if c.contains(['\n', '\r']) {
        anyhow::bail!("command may not contain newlines");
    }
    Ok(())
}

/// Where managed jobs append "<rfc3339> <job id> <exit code>" after each run.
pub fn run_log_path() -> std::path::PathBuf {
    crate::paths::data_dir().join("cron-runs.log")
}

/// Crontab line for one job: run it, then record the exit code.
/// `%` is special in crontab (newline), so it is escaped.
pub fn job_line(job: &ManagedJob, log: &str) -> String {
    let line = format!(
        "{} ( {} ) >/dev/null 2>&1; echo \"$(date +%Y-%m-%dT%H:%M:%S) {} $?\" >> '{}'",
        job.schedule.trim(),
        job.command.trim(),
        job.id,
        log
    );
    line.replace('%', "\\%")
}

/// Replace (or add) the CAShell block in `existing`, leaving every other
/// line untouched. An empty `jobs` removes the block.
pub fn render_crontab(existing: &str, jobs: &[ManagedJob], log: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    for line in existing.lines() {
        if line.trim() == BLOCK_START {
            inside = true;
            continue;
        }
        if line.trim() == BLOCK_END {
            inside = false;
            continue;
        }
        if !inside {
            out.push(line.to_string());
        }
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    if !jobs.is_empty() {
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push(BLOCK_START.to_string());
        for j in jobs {
            let desc: String = j.description.chars().filter(|c| *c != '\n').collect();
            out.push(format!("# cashell-job:{} {}", j.id, desc));
            out.push(job_line(j, log));
        }
        out.push(BLOCK_END.to_string());
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Raw `crontab -l` text ("" when the user has none).
pub fn read_raw_crontab() -> Result<String> {
    let o = crate::host_guard::output(std::process::Command::new("crontab").arg("-l"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        // "no crontab for <user>" is the normal empty case
        Ok(String::new())
    }
}

/// Crontab entries that CAShell does not manage.
pub fn other_entries(raw: &str) -> Vec<CronEntry> {
    let outside = render_crontab(raw, &[], "");
    CronEntry::parse_crontab(&outside)
}

/// Write `jobs` into the user crontab (via `crontab -`).
pub fn install(jobs: &[ManagedJob]) -> Result<()> {
    use std::io::Write;
    let log = run_log_path();
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let log_s = log.display().to_string();
    if log_s.contains('\'') {
        anyhow::bail!("data directory path contains a quote: {log_s}");
    }
    let new = render_crontab(&read_raw_crontab()?, jobs, &log_s);
    if !crate::host_guard::allowed() {
        anyhow::bail!("crontab writes are disabled in tests");
    }
    let mut child = std::process::Command::new("crontab")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(new.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!(
            "crontab rejected the update: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Last run per job id from the run log: (timestamp, exit code).
pub fn last_runs() -> std::collections::HashMap<i64, (String, i32)> {
    let mut map = std::collections::HashMap::new();
    if let Ok(text) = std::fs::read_to_string(run_log_path()) {
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if let (Some(ts), Some(id), Some(code)) = (it.next(), it.next(), it.next()) {
                if let (Ok(id), Ok(code)) = (id.parse::<i64>(), code.parse::<i32>()) {
                    map.insert(id, (ts.to_string(), code));
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: i64) -> ManagedJob {
        ManagedJob {
            id,
            schedule: "*/5 * * * *".into(),
            command: "date +%F >> /tmp/x".into(),
            description: "demo".into(),
        }
    }

    #[test]
    fn test_validate_schedule() {
        for ok in [
            "0 2 * * *",
            "*/15 * * * *",
            "0 9-17 * * mon-fri",
            "@daily",
            "5,35 * 1 1 0",
        ] {
            assert!(validate_schedule(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "* * * *",
            "61 * * * *",
            "0 25 * * *",
            "@sometimes",
            "0 2 * * * ; rm",
            "0 2 * * *\n* * * * * x",
        ] {
            assert!(validate_schedule(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn test_render_keeps_user_lines_and_replaces_block() {
        let user = "MAILTO=me\n0 1 * * * /home/me/backup.sh\n";
        let first = render_crontab(user, &[job(1)], "/l/cron.log");
        assert!(first.starts_with("MAILTO=me\n0 1 * * * /home/me/backup.sh\n"));
        assert!(first.contains(BLOCK_START) && first.contains("# cashell-job:1 demo"));
        assert!(
            first.contains("date +\\%F"),
            "percent must be escaped: {first}"
        );
        let second = render_crontab(&first, &[job(2)], "/l/cron.log");
        assert!(!second.contains("cashell-job:1") && second.contains("cashell-job:2"));
        assert_eq!(second.matches(BLOCK_START).count(), 1);
        let cleared = render_crontab(&second, &[], "/l/cron.log");
        assert_eq!(cleared, user);
        assert_eq!(other_entries(&second).len(), 1);
    }

    #[test]
    fn test_parse_crontab() {
        let sample = "
# Comment line
0 2 * * * /usr/local/bin/backup.sh
*/15 * * * * curl -s http://localhost:8080/health
# Another comment
";
        let entries = CronEntry::parse_crontab(sample);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].schedule, "0 2 * * *");
        assert_eq!(entries[0].command, "/usr/local/bin/backup.sh");
        assert_eq!(entries[1].schedule, "*/15 * * * *");
    }
}
