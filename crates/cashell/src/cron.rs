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
        let output = std::process::Command::new("crontab")
            .arg("-l")
            .output();

        match output {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                Ok(Self::parse_crontab(&text))
            }
            _ => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
