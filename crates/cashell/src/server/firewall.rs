//! UFW firewall management via direct argv execution (SV-02).
//!
//! Hardening requirement (H-08): All UFW commands are invoked via direct
//! process argv chaining without any shell interpolation.

use anyhow::{bail, Context, Result};
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// Abstraction for invoking UFW commands, enabling deterministic mocking in tests.
pub trait FirewallRunner: Send + Sync {
    fn execute_ufw(&self, args: &[&str]) -> Result<String>;
}

/// System implementation executing `/usr/sbin/ufw` or `ufw` via direct OS argv.
#[derive(Debug, Default, Clone)]
pub struct SystemFirewallRunner;

impl FirewallRunner for SystemFirewallRunner {
    fn execute_ufw(&self, args: &[&str]) -> Result<String> {
        let output = std::process::Command::new("ufw")
            .args(args)
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    anyhow::anyhow!("ufw not found — install with: sudo apt-get install ufw")
                } else {
                    anyhow::anyhow!("failed to execute ufw: {e}")
                }
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            bail!("ufw command failed (exit code {code}): {stderr} {stdout}");
        }

        Ok(stdout)
    }
}

/// In-memory mock firewall runner for tests.
#[derive(Debug, Default, Clone)]
pub struct MockFirewallRunner {
    pub executed_commands: Arc<Mutex<Vec<Vec<String>>>>,
    pub simulated_status: Arc<Mutex<String>>,
}

impl MockFirewallRunner {
    pub fn new() -> Self {
        let default_status = "Status: active\n\n     To                         Action      From\n     --                         ------      ----\n[ 1] 22/tcp                     ALLOW IN    Anywhere\n[ 2] 80/tcp                     ALLOW IN    Anywhere\n[ 3] 443/tcp                    ALLOW IN    Anywhere\n";
        Self {
            executed_commands: Arc::new(Mutex::new(Vec::new())),
            simulated_status: Arc::new(Mutex::new(default_status.to_string())),
        }
    }

    pub fn calls(&self) -> Vec<Vec<String>> {
        self.executed_commands.lock().unwrap().clone()
    }
}

impl FirewallRunner for MockFirewallRunner {
    fn execute_ufw(&self, args: &[&str]) -> Result<String> {
        let cmd: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        self.executed_commands.lock().unwrap().push(cmd);

        if args.len() >= 2 && args[0] == "status" && args[1] == "numbered" {
            return Ok(self.simulated_status.lock().unwrap().clone());
        }

        Ok(format!("Rule updated: {:?}", args))
    }
}

/// Parsed UFW firewall rule representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UfwRule {
    pub number: usize,
    pub to: String,
    pub action: String,
    pub from: String,
}

/// Validate protocol string (must be tcp or udp).
pub fn validate_protocol(proto: &str) -> Result<()> {
    match proto.to_lowercase().as_str() {
        "tcp" | "udp" => Ok(()),
        other => bail!("unsupported protocol '{other}', expected 'tcp' or 'udp'"),
    }
}

/// Validate IP address string (IPv4 or IPv6).
pub fn validate_ip_address(ip: &str) -> Result<IpAddr> {
    IpAddr::from_str(ip.trim()).with_context(|| format!("invalid IP address '{ip}'"))
}

/// High-level Firewall Manager (SV-02).
pub struct FirewallManager {
    runner: Arc<dyn FirewallRunner>,
}

impl Default for FirewallManager {
    fn default() -> Self {
        Self {
            runner: Arc::new(SystemFirewallRunner),
        }
    }
}

impl FirewallManager {
    pub fn new(runner: Arc<dyn FirewallRunner>) -> Self {
        Self { runner }
    }

    /// List active UFW rules with numbers (SV-02).
    pub fn list_rules(&self) -> Result<String> {
        self.runner.execute_ufw(&["status", "numbered"])
    }

    /// Parse numbered rules from UFW output text.
    pub fn parse_rules(output: &str) -> Vec<UfwRule> {
        let mut rules = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.contains(']') {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 4 {
                    // e.g. "[", "1]", "22/tcp", "ALLOW", "IN", "Anywhere"
                    let num_str = parts[0]
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .trim();
                    let num = if num_str.is_empty() && parts[1].ends_with(']') {
                        parts[1].trim_end_matches(']').parse::<usize>().unwrap_or(0)
                    } else {
                        num_str.parse::<usize>().unwrap_or(0)
                    };

                    let to = if num_str.is_empty() {
                        parts[2]
                    } else {
                        parts[1]
                    };
                    let action = if num_str.is_empty() {
                        parts[3]
                    } else {
                        parts[2]
                    };
                    let from = if parts.len() > 4 {
                        parts[parts.len() - 1]
                    } else {
                        "Anywhere"
                    };

                    rules.push(UfwRule {
                        number: num,
                        to: to.to_string(),
                        action: action.to_string(),
                        from: from.to_string(),
                    });
                }
            }
        }
        rules
    }

    /// Allow port via UFW (SV-02).
    pub fn allow_port(&self, port: u16, proto: Option<&str>) -> Result<String> {
        if port == 0 {
            bail!("port must be between 1 and 65535");
        }
        if let Some(p) = proto {
            validate_protocol(p)?;
            let spec = format!("{port}/{}", p.to_lowercase());
            self.runner.execute_ufw(&["allow", &spec])
        } else {
            let port_str = port.to_string();
            self.runner.execute_ufw(&["allow", &port_str])
        }
    }

    /// Deny port via UFW (SV-02).
    pub fn deny_port(&self, port: u16, proto: Option<&str>) -> Result<String> {
        if port == 0 {
            bail!("port must be between 1 and 65535");
        }
        if let Some(p) = proto {
            validate_protocol(p)?;
            let spec = format!("{port}/{}", p.to_lowercase());
            self.runner.execute_ufw(&["deny", &spec])
        } else {
            let port_str = port.to_string();
            self.runner.execute_ufw(&["deny", &port_str])
        }
    }

    /// Delete rule via UFW (by rule number or port specification) (SV-02).
    pub fn delete_rule(&self, rule_spec: &str) -> Result<String> {
        let spec = rule_spec.trim();
        if spec.is_empty() {
            bail!("rule specification cannot be empty");
        }

        // If integer, delete by rule number: `ufw --force delete <num>`
        if let Ok(num) = spec.parse::<usize>() {
            let num_str = num.to_string();
            return self.runner.execute_ufw(&["--force", "delete", &num_str]);
        }

        // Otherwise delete by spec: e.g. "allow 80" or "80/tcp"
        let parts: Vec<&str> = spec.split_whitespace().collect();
        if parts.len() == 1 {
            // Check if port or port/proto
            return self
                .runner
                .execute_ufw(&["--force", "delete", "allow", parts[0]]);
        } else if parts.len() == 2 {
            let action = parts[0].to_lowercase();
            if action != "allow" && action != "deny" {
                bail!("action must be 'allow' or 'deny'");
            }
            return self
                .runner
                .execute_ufw(&["--force", "delete", &action, parts[1]]);
        }

        bail!("unrecognized delete rule specification '{spec}'")
    }

    /// Drop all traffic from target IP address at top priority (SV-03).
    pub fn ban_ip(&self, ip: &str) -> Result<String> {
        let _ = validate_ip_address(ip)?;
        self.runner
            .execute_ufw(&["insert", "1", "deny", "from", ip.trim(), "to", "any"])
    }

    /// Remove ban rule for target IP address (SV-03).
    pub fn unban_ip(&self, ip: &str) -> Result<String> {
        let _ = validate_ip_address(ip)?;
        self.runner
            .execute_ufw(&["delete", "deny", "from", ip.trim(), "to", "any"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_firewall_argv_safety_and_rules() {
        let mock = Arc::new(MockFirewallRunner::new());
        let fw = FirewallManager::new(mock.clone());

        // 1. List rules
        let status = fw.list_rules().unwrap();
        assert!(status.contains("ALLOW IN"));
        let parsed = FirewallManager::parse_rules(&status);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].to, "22/tcp");

        // 2. Allow port with proto
        let res1 = fw.allow_port(8080, Some("tcp")).unwrap();
        assert!(res1.contains("Rule updated"));

        // 3. Deny port
        let res2 = fw.deny_port(25, None).unwrap();
        assert!(res2.contains("Rule updated"));

        // 4. Delete rule by number
        let res3 = fw.delete_rule("2").unwrap();
        assert!(res3.contains("Rule updated"));

        // 5. Ban & Unban IP
        let ban_res = fw.ban_ip("192.0.2.1").unwrap();
        assert!(ban_res.contains("Rule updated"));
        let unban_res = fw.unban_ip("192.0.2.1").unwrap();
        assert!(unban_res.contains("Rule updated"));

        // Verify all calls were argv-based
        let calls = mock.calls();
        assert_eq!(calls[1], vec!["allow", "8080/tcp"]);
        assert_eq!(calls[2], vec!["deny", "25"]);
        assert_eq!(calls[3], vec!["--force", "delete", "2"]);
        assert_eq!(
            calls[4],
            vec!["insert", "1", "deny", "from", "192.0.2.1", "to", "any"]
        );
        assert_eq!(
            calls[5],
            vec!["delete", "deny", "from", "192.0.2.1", "to", "any"]
        );

        // Verify invalid inputs rejected before process call
        assert!(fw.allow_port(0, None).is_err());
        assert!(fw.allow_port(80, Some("invalid_proto")).is_err());
        assert!(fw.ban_ip("not-an-ip; rm -rf /").is_err());
    }
}
