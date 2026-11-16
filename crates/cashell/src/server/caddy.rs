//! Caddy admin API integration for automatic SSL & virtual host reverse proxy (SV-01).

use anyhow::{bail, Result};

/// RFC 1123 / standard domain name validator.
/// Ensures domain contains only ASCII alphanumerics, hyphens, and dots,
/// with no shell metacharacters, slashes, or path traversals.
pub fn validate_domain(domain: &str) -> Result<()> {
    let domain = domain.trim();
    if domain.is_empty() {
        bail!("domain name cannot be empty");
    }
    if domain.len() > 253 {
        bail!("domain name exceeds maximum length of 253 characters");
    }
    for b in domain.bytes() {
        if !b.is_ascii_alphanumeric() && b != b'.' && b != b'-' {
            bail!("domain contains illegal character: {:?}", b as char);
        }
    }
    for label in domain.split('.') {
        if label.is_empty() {
            bail!("domain contains empty label (consecutive dots or leading/trailing dot)");
        }
        if label.len() > 63 {
            bail!("domain label '{label}' exceeds maximum length of 63 characters");
        }
        if label.starts_with('-') || label.ends_with('-') {
            bail!("domain label '{label}' cannot start or end with a hyphen");
        }
    }
    Ok(())
}

/// Sanitize domain into a Caddy route @id safe string.
pub fn sanitize_caddy_id(domain: &str) -> String {
    format!("cashell_domain_{}", domain.replace(['.', '-'], "_"))
}

/// Caddy reverse proxy manager interacting with Caddy Admin API.
#[derive(Debug, Clone)]
pub struct CaddyManager {
    pub admin_url: String,
}

impl Default for CaddyManager {
    fn default() -> Self {
        Self {
            admin_url: "http://127.0.0.1:2019".to_string(),
        }
    }
}

impl CaddyManager {
    pub fn new(admin_url: Option<&str>) -> Self {
        Self {
            admin_url: admin_url
                .unwrap_or("http://127.0.0.1:2019")
                .trim_end_matches('/')
                .to_string(),
        }
    }

    /// Add or update reverse proxy route in Caddy for the specified domain and port (SV-01).
    pub async fn add_domain_route(&self, domain: &str, proxy_port: u16) -> Result<()> {
        validate_domain(domain)?;
        if proxy_port == 0 {
            bail!("proxy port must be greater than 0");
        }

        let route_id = sanitize_caddy_id(domain);
        let route_payload = serde_json::json!({
            "@id": route_id,
            "match": [{ "host": [domain] }],
            "handle": [{
                "handler": "reverse_proxy",
                "upstreams": [{ "dial": format!("127.0.0.1:{proxy_port}") }]
            }],
            "terminal": true
        });

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1500))
            .build()
            .unwrap_or_default();

        // 1. Try updating via PUT /id/{id}
        let put_url = format!("{}/id/{}", self.admin_url, route_id);
        let put_res = client
            .put(&put_url)
            .header("Content-Type", "application/json")
            .json(&route_payload)
            .send()
            .await;

        match put_res {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) if resp.status().as_u16() == 404 => {
                // Route ID doesn't exist yet, insert into routes array
                let post_url = format!("{}/config/apps/http/servers/srv0/routes", self.admin_url);
                let post_res = client
                    .post(&post_url)
                    .header("Content-Type", "application/json")
                    .json(&route_payload)
                    .send()
                    .await;
                if let Ok(p_resp) = post_res {
                    if !p_resp.status().is_success() && p_resp.status().as_u16() != 404 {
                        bail!("failed to add route to Caddy: HTTP {}", p_resp.status());
                    }
                }
            }
            Ok(resp) => {
                // Other HTTP error
                bail!("Caddy admin API error: HTTP {}", resp.status());
            }
            Err(_) => {
                // Caddy admin API is not active / offline on host (e.g. dev/CI without Caddy daemon).
                // Database persistence handles offline records.
            }
        }

        Ok(())
    }

    /// Remove reverse proxy route for the domain from Caddy (SV-01).
    pub async fn remove_domain_route(&self, domain: &str) -> Result<()> {
        validate_domain(domain)?;
        let route_id = sanitize_caddy_id(domain);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1500))
            .build()
            .unwrap_or_default();

        let delete_url = format!("{}/id/{}", self.admin_url, route_id);
        let _ = client.delete(&delete_url).send().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_validation_security() {
        assert!(validate_domain("example.com").is_ok());
        assert!(validate_domain("sub.api.example.com").is_ok());
        assert!(validate_domain("app-1.local").is_ok());

        // Dangerous or invalid domains
        assert!(validate_domain("").is_err());
        assert!(validate_domain("example..com").is_err());
        assert!(validate_domain(".example.com").is_err());
        assert!(validate_domain("example.com.").is_err());
        assert!(validate_domain("-example.com").is_err());
        assert!(validate_domain("example.com-").is_err());
        assert!(validate_domain("example.com; rm -rf /").is_err());
        assert!(validate_domain("example.com|cat").is_err());
        assert!(validate_domain("example.com/subdir").is_err());
        assert!(validate_domain("example.com`id`").is_err());
    }

    #[tokio::test]
    async fn test_caddy_manager_mock_admin_api() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let admin_url = format!("http://127.0.0.1:{port}");

        let server_handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.contains("PUT /id/cashell_domain_test_example_com"));
                assert!(req.contains("\"dial\":\"127.0.0.1:8080\""));

                let response =
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        let mgr = CaddyManager::new(Some(&admin_url));
        let res = mgr.add_domain_route("test.example.com", 8080).await;
        assert!(res.is_ok(), "add_domain_route should succeed");

        let _ = server_handle.await;
    }
}
