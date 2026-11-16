//! SSH brute-force detection, UFW auto-banning, and automatic unban expiration (SV-03).

use anyhow::Result;
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;

use crate::db::Database;
use crate::server::firewall::FirewallRunner;

/// A detected failed SSH authentication attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshFailureEvent {
    pub ip: String,
    pub user: Option<String>,
}

/// Parse SSH failure events from sshd/auth.log text.
pub fn parse_ssh_failed_events(logs: &str) -> Vec<SshFailureEvent> {
    let mut events = Vec::new();

    for line in logs.lines() {
        let trimmed = line.trim();

        // 1. "Failed password for [invalid user ]<user> from <ip> port <port>"
        if let Some(pos) = trimmed.find("Failed password for ") {
            let after = &trimmed[pos + "Failed password for ".len()..];
            let after = after.trim_start_matches("invalid user ");
            let parts: Vec<&str> = after.split_whitespace().collect();
            // parts: [<user>, "from", <ip>, "port", <port>, ...]
            if parts.len() >= 3 && parts[1] == "from" {
                let user = parts[0];
                let ip_candidate = parts[2];
                if IpAddr::from_str(ip_candidate).is_ok() {
                    events.push(SshFailureEvent {
                        ip: ip_candidate.to_string(),
                        user: Some(user.to_string()),
                    });
                    continue;
                }
            }
        }

        // 2. "Invalid user <user> from <ip>"
        if let Some(pos) = trimmed.find("Invalid user ") {
            let after = &trimmed[pos + "Invalid user ".len()..];
            let parts: Vec<&str> = after.split_whitespace().collect();
            // parts: [<user>, "from", <ip>, ...]
            if parts.len() >= 3 && parts[1] == "from" {
                let user = parts[0];
                let ip_candidate = parts[2];
                if IpAddr::from_str(ip_candidate).is_ok() {
                    events.push(SshFailureEvent {
                        ip: ip_candidate.to_string(),
                        user: Some(user.to_string()),
                    });
                    continue;
                }
            }
        }

        // 3. "Connection closed by authenticating user <user> <ip> port <port> [preauth]"
        if let Some(pos) = trimmed.find("authenticating user ") {
            let after = &trimmed[pos + "authenticating user ".len()..];
            let parts: Vec<&str> = after.split_whitespace().collect();
            if parts.len() >= 2 {
                let user = parts[0];
                let ip_candidate = parts[1];
                if IpAddr::from_str(ip_candidate).is_ok() {
                    events.push(SshFailureEvent {
                        ip: ip_candidate.to_string(),
                        user: Some(user.to_string()),
                    });
                    continue;
                }
            }
        }
    }

    events
}

/// SSH auto-ban engine configuration and coordinator (SV-03).
#[derive(Debug, Clone)]
pub struct SshAutobanEngine {
    pub threshold: u32,
    pub ban_duration_secs: i64,
}

impl Default for SshAutobanEngine {
    fn default() -> Self {
        Self {
            threshold: 10,
            ban_duration_secs: 3600, // 1 hour
        }
    }
}

impl SshAutobanEngine {
    pub fn new(threshold: u32, ban_duration_secs: i64) -> Self {
        Self {
            threshold,
            ban_duration_secs,
        }
    }

    /// Read recent sshd logs from journalctl or auth.log via direct argv.
    pub fn read_recent_logs(lines: usize) -> Result<String> {
        let lines_str = lines.to_string();
        let journal_res = std::process::Command::new("journalctl")
            .args(["-u", "ssh", "-u", "sshd", "-n", &lines_str, "--no-pager"])
            .output();

        if let Ok(out) = journal_res {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).to_string();
                if !s.trim().is_empty() {
                    return Ok(s);
                }
            }
        }

        // Fallback to /var/log/auth.log if readable
        for path in ["/var/log/auth.log", "/var/log/secure"] {
            if std::path::Path::new(path).exists() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    let recent_lines: Vec<&str> = content.lines().rev().take(lines).collect();
                    return Ok(recent_lines
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n"));
                }
            }
        }

        Ok(String::new())
    }

    /// Process log entries, count failures, and ban offending IPs exceeding threshold (SV-03).
    pub fn process_log_content(
        &self,
        logs: &str,
        db: &Database,
        fw: &dyn FirewallRunner,
    ) -> Result<Vec<String>> {
        let events = parse_ssh_failed_events(logs);
        let mut counts: HashMap<String, u32> = HashMap::new();
        for event in events {
            *counts.entry(event.ip).or_insert(0) += 1;
        }

        let mut banned_ips = Vec::new();
        let existing_active_bans: HashMap<String, _> = db
            .list_ssh_bans(true)?
            .into_iter()
            .map(|b| (b.ip.clone(), b))
            .collect();

        for (ip, count) in counts {
            if count >= self.threshold && !existing_active_bans.contains_key(&ip) {
                // Execute UFW drop rule
                let _ = fw.execute_ufw(&["insert", "1", "deny", "from", &ip, "to", "any"])?;
                // Persist ban record in SQLite
                db.record_ssh_ban(&ip, count, self.ban_duration_secs)?;
                banned_ips.push(ip);
            }
        }

        Ok(banned_ips)
    }

    /// Check and automatically unban IPs whose ban duration has expired (SV-03).
    pub fn check_and_unban_expired(
        &self,
        db: &Database,
        fw: &dyn FirewallRunner,
        now: i64,
    ) -> Result<Vec<String>> {
        let expired = db.get_expired_ssh_bans(now)?;
        let mut unbanned = Vec::new();

        for ban in expired {
            // Delete UFW rule
            let _ = fw.execute_ufw(&["delete", "deny", "from", &ban.ip, "to", "any"]);
            // Mark as unbanned in DB
            db.update_ssh_ban_status(&ban.ip, "unbanned")?;
            unbanned.push(ban.ip);
        }

        Ok(unbanned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::firewall::MockFirewallRunner;
    use std::sync::Arc;

    #[test]
    fn test_parse_ssh_failed_events() {
        let sample_logs = r#"
Nov 15 10:00:01 cax1 sshd[1234]: Failed password for invalid user admin from 198.51.100.42 port 49212 ssh2
Nov 15 10:00:02 cax1 sshd[1235]: Failed password for root from 198.51.100.42 port 49213 ssh2
Nov 15 10:00:03 cax1 sshd[1236]: Invalid user test from 198.51.100.43 port 51234
Nov 15 10:00:04 cax1 sshd[1237]: Connection closed by authenticating user guest 203.0.113.5 port 55123 [preauth]
Nov 15 10:00:05 cax1 sshd[1238]: Accepted publickey for deploy from 192.0.2.1 port 2222 ssh2
"#;
        let events = parse_ssh_failed_events(sample_logs);
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].ip, "198.51.100.42");
        assert_eq!(events[0].user.as_deref(), Some("admin"));
        assert_eq!(events[1].ip, "198.51.100.42");
        assert_eq!(events[1].user.as_deref(), Some("root"));
        assert_eq!(events[2].ip, "198.51.100.43");
        assert_eq!(events[3].ip, "203.0.113.5");
    }

    #[test]
    fn test_sv03_auto_ban_and_unban_lifecycle() -> Result<()> {
        let db = Database::open_in_memory()?;
        let mock_fw = Arc::new(MockFirewallRunner::new());
        let engine = SshAutobanEngine::new(10, 60); // 10 attempts threshold, 60s ban duration

        // Simulate 10 failed login attempts from test IP 198.51.100.77
        let mut simulated_logs = String::new();
        for i in 1..=10 {
            simulated_logs.push_str(&format!(
                "Nov 15 10:01:{i:02} cax1 sshd[{i}]: Failed password for invalid user hacker from 198.51.100.77 port 4000{i} ssh2\n"
            ));
        }

        // Process logs -> Should ban 198.51.100.77
        let banned = engine.process_log_content(&simulated_logs, &db, mock_fw.as_ref())?;
        assert_eq!(banned, vec!["198.51.100.77".to_string()]);

        // Verify active bans in database
        let active_bans = db.list_ssh_bans(true)?;
        assert_eq!(active_bans.len(), 1);
        assert_eq!(active_bans[0].ip, "198.51.100.77");
        assert_eq!(active_bans[0].attempts, 10);
        assert_eq!(active_bans[0].status, "banned");

        // Verify UFW insert command was called
        let calls = mock_fw.calls();
        assert!(calls
            .iter()
            .any(|c| c == &["insert", "1", "deny", "from", "198.51.100.77", "to", "any"]));

        // Attempting to unban immediately (now < unban_at) -> No unbans
        let now = active_bans[0].banned_at;
        let early_unbans = engine.check_and_unban_expired(&db, mock_fw.as_ref(), now)?;
        assert!(early_unbans.is_empty());

        // Fast forward time past expiry (now + 65 seconds)
        let expired_time = active_bans[0].unban_at + 5;
        let unbanned = engine.check_and_unban_expired(&db, mock_fw.as_ref(), expired_time)?;
        assert_eq!(unbanned, vec!["198.51.100.77".to_string()]);

        // Verify status in DB changed to unbanned
        let remaining_active = db.list_ssh_bans(true)?;
        assert!(remaining_active.is_empty());
        let all_bans = db.list_ssh_bans(false)?;
        assert_eq!(all_bans[0].status, "unbanned");

        // Verify UFW delete command was called
        let final_calls = mock_fw.calls();
        assert!(final_calls
            .iter()
            .any(|c| c == &["delete", "deny", "from", "198.51.100.77", "to", "any"]));

        Ok(())
    }
}
