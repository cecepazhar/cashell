//! Server management, proxy configuration, and host hardening (Tahap 5).

pub mod autoban;
pub mod backup;
pub mod caddy;
pub mod firewall;
pub mod hardening;

pub use autoban::{SshAutobanEngine, SshFailureEvent};
pub use backup::{BackupUploadResult, DatabaseBackupManager};
pub use caddy::CaddyManager;
pub use firewall::{FirewallManager, FirewallRunner, SystemFirewallRunner, UfwRule};
pub use hardening::{
    generate_hardened_systemd_unit, generate_limited_sudoers, verify_daemon_execution_user,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::server::firewall::MockFirewallRunner;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_sv01_caddy_domain_add_and_ssl_reverse_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let admin_url = format!("http://127.0.0.1:{port}");

        let server_task = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.contains("PUT /id/cashell_domain_api_example_com"));
                assert!(req.contains("\"dial\":\"127.0.0.1:3000\""));

                let response =
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        let db = Database::open_in_memory().unwrap();
        let caddy = CaddyManager::new(Some(&admin_url));

        // 1. Add domain reverse proxy
        caddy
            .add_domain_route("api.example.com", 3000)
            .await
            .unwrap();
        db.upsert_server_domain("api.example.com", 3000, "active")
            .unwrap();

        // 2. Verify domain stored in DB
        let domains = db.list_server_domains().unwrap();
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0].domain, "api.example.com");
        assert_eq!(domains[0].proxy_port, 3000);
        assert_eq!(domains[0].ssl_status, "active");

        // 3. Remove domain
        let removed = db.remove_server_domain("api.example.com").unwrap();
        assert!(removed);
        assert!(db.list_server_domains().unwrap().is_empty());

        let _ = server_task.await;
    }

    #[test]
    fn test_sv02_ufw_firewall_numbered_rules_and_argv_safety() {
        let mock_runner = Arc::new(MockFirewallRunner::new());
        let fw = FirewallManager::new(mock_runner.clone());

        // 1. List numbered rules
        let status = fw.list_rules().unwrap();
        let rules = FirewallManager::parse_rules(&status);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].number, 1);
        assert_eq!(rules[0].to, "22/tcp");

        // 2. Allow port with proto
        let allow_res = fw.allow_port(8080, Some("tcp")).unwrap();
        assert!(allow_res.contains("Rule updated"));

        // 3. Deny port
        let deny_res = fw.deny_port(25, None).unwrap();
        assert!(deny_res.contains("Rule updated"));

        // 4. Delete rule by number
        let del_res = fw.delete_rule("1").unwrap();
        assert!(del_res.contains("Rule updated"));

        // Verify argv was constructed safely without shell
        let calls = mock_runner.calls();
        assert_eq!(calls[0], vec!["status", "numbered"]);
        assert_eq!(calls[1], vec!["allow", "8080/tcp"]);
        assert_eq!(calls[2], vec!["deny", "25"]);
        assert_eq!(calls[3], vec!["--force", "delete", "1"]);
    }

    #[test]
    fn test_sv03_autoban_bruteforce_ssh_and_auto_unban() {
        let db = Database::open_in_memory().unwrap();
        let mock_fw = Arc::new(MockFirewallRunner::new());
        let engine = SshAutobanEngine::new(10, 300); // 10 failed attempts, 300s duration

        // Simulate 10 brute-force failed login attempts from IP 198.51.100.99
        let mut logs = String::new();
        for i in 1..=10 {
            logs.push_str(&format!(
                "Nov 15 10:05:{i:02} cax1 sshd[{i}]: Failed password for invalid user admin from 198.51.100.99 port 500{i} ssh2\n"
            ));
        }

        // Process log chunk
        let banned = engine
            .process_log_content(&logs, &db, mock_fw.as_ref())
            .unwrap();
        assert_eq!(banned, vec!["198.51.100.99".to_string()]);

        // Verify SQLite table ssh_bans has active ban
        let active = db.list_ssh_bans(true).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].ip, "198.51.100.99");
        assert_eq!(active[0].attempts, 10);
        assert_eq!(active[0].status, "banned");

        // Verify UFW insert argv call
        let calls = mock_fw.calls();
        assert_eq!(
            calls[0],
            vec!["insert", "1", "deny", "from", "198.51.100.99", "to", "any"]
        );

        // Before expiry: no unban
        let now = active[0].banned_at;
        let early = engine
            .check_and_unban_expired(&db, mock_fw.as_ref(), now)
            .unwrap();
        assert!(early.is_empty());

        // After expiry: auto-unban triggers
        let unbanned = engine
            .check_and_unban_expired(&db, mock_fw.as_ref(), active[0].unban_at + 1)
            .unwrap();
        assert_eq!(unbanned, vec!["198.51.100.99".to_string()]);

        // Verify DB updated
        assert!(db.list_ssh_bans(true).unwrap().is_empty());
        let all = db.list_ssh_bans(false).unwrap();
        assert_eq!(all[0].status, "unbanned");

        // Verify UFW delete argv call
        let final_calls = mock_fw.calls();
        assert_eq!(
            final_calls[1],
            vec!["delete", "deny", "from", "198.51.100.99", "to", "any"]
        );
    }

    #[test]
    fn test_sv04_smartctl_json_parsing_and_no_smart_handling() {
        use crate::systat::parse_smartctl_json;

        // 1. ATA JSON parsing
        let ata_raw = r#"{
            "device": { "name": "/dev/sda" },
            "model_name": "Crucial MX500",
            "smart_status": { "passed": true },
            "temperature": { "current": 36 },
            "power_on_time": { "hours": 5100 },
            "power_cycle_count": 210,
            "ata_smart_attributes": {
                "table": [
                    { "id": 9, "name": "Power_On_Hours", "raw": { "value": 5100 } },
                    { "id": 194, "name": "Temperature_Celsius", "raw": { "value": 36 } },
                    { "id": 231, "name": "SSD_Life_Left", "value": 95 },
                    { "id": 241, "name": "Total_LBAs_Written", "raw": { "value": 2000000000 } },
                    { "id": 242, "name": "Total_LBAs_Read", "raw": { "value": 4000000000 } }
                ]
            }
        }"#;
        let ata_val: serde_json::Value = serde_json::from_str(ata_raw).unwrap();
        let ata_parsed = parse_smartctl_json(&ata_val, "/dev/sda").expect("valid ata parsed");
        assert_eq!(ata_parsed.model, "Crucial MX500");
        assert_eq!(ata_parsed.temperature_c, 36.0);
        assert_eq!(ata_parsed.health_status, "HEALTHY (95%)");

        // 2. NVMe JSON parsing
        let nvme_raw = r#"{
            "device": { "name": "/dev/nvme0n1" },
            "model_name": "Samsung SSD 970 EVO Plus 1TB",
            "smart_status": { "passed": true },
            "nvme_smart_health_information_log": {
                "critical_warning": 0,
                "temperature": 44,
                "percentage_used": 2,
                "data_units_read": 1953125,
                "data_units_written": 781250,
                "power_on_hours": 3200,
                "power_cycles": 450,
                "unsafe_shutdowns": 6
            }
        }"#;
        let nvme_val: serde_json::Value = serde_json::from_str(nvme_raw).unwrap();
        let nvme_parsed =
            parse_smartctl_json(&nvme_val, "/dev/nvme0n1").expect("valid nvme parsed");
        assert_eq!(nvme_parsed.model, "Samsung SSD 970 EVO Plus 1TB");
        assert_eq!(nvme_parsed.temperature_c, 44.0);
        assert_eq!(nvme_parsed.wear_level_pct, 2);

        // 3. Disk without SMART capability or permission denied
        let no_smart_raw = r#"{
            "device": { "name": "/dev/vda" },
            "smart_support": { "available": false, "enabled": false },
            "messages": [
                { "string": "Device lacks SMART capability", "severity": "error" }
            ]
        }"#;
        let no_smart_val: serde_json::Value = serde_json::from_str(no_smart_raw).unwrap();
        let no_smart_parsed = parse_smartctl_json(&no_smart_val, "/dev/vda");
        assert!(
            no_smart_parsed.is_none(),
            "disk without SMART must return None to trigger sysfs fallback"
        );
    }

    #[tokio::test]
    async fn test_sv05_scheduled_db_backup_to_s3_and_restore() -> anyhow::Result<()> {
        let temp_dir = std::env::temp_dir().join(format!(
            "cashell_sv05_test_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&temp_dir)?;

        // Create backup
        let backup_file = DatabaseBackupManager::create_backup("shop_db", &temp_dir)?;
        assert!(backup_file.exists());

        // Setup mock S3/R2 server
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let endpoint = format!("http://127.0.0.1:{port}");

        let server_task = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req_header = String::from_utf8_lossy(&buf[..n]);
                assert!(req_header.contains("PUT /my-prod-backups/backups/shop_db-"));
                assert!(req_header.contains("x-amz-content-sha256"));

                let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        // Upload to S3
        let upload_res = DatabaseBackupManager::upload_backup_to_s3(
            &backup_file,
            "my-prod-backups",
            &endpoint,
            "AKIA_USER",
            "SECRET_KEY",
        )
        .await?;
        assert_eq!(upload_res.bucket, "my-prod-backups");
        assert!(upload_res.object_key.starts_with("backups/shop_db-"));

        let _ = server_task.await;

        // Restore backup
        let restore_res = DatabaseBackupManager::restore_backup("shop_db", &backup_file);
        assert!(restore_res.is_ok());

        // Generate cron schedule
        let cron = DatabaseBackupManager::generate_cron_schedule(
            "shop_db",
            "0 3 * * *",
            Some("my-prod-backups"),
        )?;
        assert_eq!(
            cron,
            "0 3 * * * /usr/local/bin/cashell server db backup shop_db --bucket my-prod-backups >/dev/null 2>&1"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
        Ok(())
    }

    #[test]
    fn test_sv06_daemon_permissions_and_sudoers_hardening() {
        // Systemd unit checks
        let unit = generate_hardened_systemd_unit("cashell", "cashell", "/usr/local/bin/cashell");
        assert!(unit.contains("User=cashell"));
        assert!(unit.contains("Group=cashell"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("ProtectHome=read-only"));

        // Sudoers rules checks
        let sudoers = generate_limited_sudoers("cashell");
        assert!(sudoers.contains("cashell ALL=(ALL) NOPASSWD:"));
        assert!(sudoers.contains("/usr/sbin/ufw"));
        assert!(sudoers.contains("/usr/bin/smartctl"));
        assert!(sudoers.contains("/bin/systemctl restart *"));

        // Runtime root check
        let res = verify_daemon_execution_user();
        #[cfg(unix)]
        {
            let euid = unsafe { libc::geteuid() };
            if euid == 0 {
                assert!(res.is_err());
            } else {
                assert!(res.is_ok());
            }
        }
    }
}
