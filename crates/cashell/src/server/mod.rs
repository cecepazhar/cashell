//! Server management, proxy configuration, and host hardening (Tahap 5).

pub mod autoban;
pub mod caddy;
pub mod firewall;

pub use autoban::{SshAutobanEngine, SshFailureEvent};
pub use caddy::CaddyManager;
pub use firewall::{FirewallManager, FirewallRunner, SystemFirewallRunner, UfwRule};

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
}
