//! Daemon privilege separation & limited sudoers hardening (SV-06).
//!
//! Enforces running the 24/7 background monitoring daemon as an unprivileged
//! system user (`cashell`) rather than root, providing strict systemd service
//! sandboxing and tightly scoped sudoers whitelist rules.

use anyhow::{bail, Result};

/// Generate hardened systemd service unit specification for unprivileged daemon (SV-06).
pub fn generate_hardened_systemd_unit(user: &str, group: &str, bin_path: &str) -> String {
    format!(
        r#"[Unit]
Description=CAShell 24/7 Fleet & Health Monitoring Daemon
After=network.target

[Service]
Type=simple
User={user}
Group={group}
ExecStart={bin_path} daemon run
Restart=always
RestartSec=10
Environment=PATH=/usr/local/bin:/usr/bin:/bin

# Security Sandbox Hardening (SV-06)
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/var/lib/cashell /var/log/cashell
CapabilityBoundingSet=
AmbientCapabilities=

[Install]
WantedBy=multi-user.target
"#
    )
}

/// Generate tightly scoped sudoers rules for the unprivileged daemon user (SV-06).
///
/// Grants passwordless execution ONLY for specific required operations:
/// - UFW firewall rule modification (`/usr/sbin/ufw`)
/// - Storage diagnostics (`/usr/bin/smartctl`)
/// - Service remediation (`/bin/systemctl restart *`)
pub fn generate_limited_sudoers(user: &str) -> String {
    format!(
        r#"# CAShell daemon restricted privileges (SV-06)
# Generated automatically by `cashell daemon install-service --system`
# The daemon runs as an unprivileged user and may only execute these specific binaries:
{user} ALL=(ALL) NOPASSWD: /usr/sbin/ufw, /usr/bin/smartctl, /bin/systemctl restart *
"#
    )
}

/// Verify that the daemon process is NOT running as root (UID 0) (SV-06).
pub fn verify_daemon_execution_user() -> Result<()> {
    #[cfg(unix)]
    {
        // Allow override in test environments if specifically enabled
        if std::env::var("CASHELL_ALLOW_ROOT_TEST")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            return Ok(());
        }

        let euid = unsafe { libc::geteuid() };
        if euid == 0 {
            bail!(
                "Daemon security violation (SV-06): Running directly as root is disallowed. \
                 The daemon must run as unprivileged user 'cashell' with limited sudoers privileges."
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sv06_daemon_permissions_and_sudoers_hardening() {
        // 1. Verify hardened systemd unit file content
        let unit = generate_hardened_systemd_unit("cashell", "cashell", "/usr/local/bin/cashell");
        assert!(unit.contains("User=cashell"));
        assert!(unit.contains("Group=cashell"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("ProtectHome=read-only"));
        assert!(unit.contains("ReadWritePaths=/var/lib/cashell /var/log/cashell"));

        // 2. Verify limited sudoers rules
        let sudoers = generate_limited_sudoers("cashell");
        assert!(sudoers.contains("cashell ALL=(ALL) NOPASSWD:"));
        assert!(sudoers.contains("/usr/sbin/ufw"));
        assert!(sudoers.contains("/usr/bin/smartctl"));
        assert!(sudoers.contains("/bin/systemctl restart *"));
        assert!(
            !sudoers.contains("ALL=(ALL) NOPASSWD: ALL"),
            "must never grant broad NOPASSWD ALL"
        );

        // 3. Test root execution check
        // Under non-root user (e.g. standard developer environment UID > 0), verify succeeds:
        let res = verify_daemon_execution_user();
        #[cfg(unix)]
        {
            let euid = unsafe { libc::geteuid() };
            if euid == 0 {
                assert!(
                    res.is_err(),
                    "must fail when running as root without override"
                );
            } else {
                assert!(res.is_ok(), "must succeed when running as non-root user");
            }
        }
    }
}
