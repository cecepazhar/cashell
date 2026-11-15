//! Outgoing Heartbeat Probing / Dead-Man Switch (U-06).
//!
//! Pings an external heartbeat monitoring service (such as healthchecks.io)
//! via HTTP GET at regular intervals. If the host or CAShell daemon crashes,
//! the external monitor alerts the operator.

use anyhow::Result;
use reqwest::Client;
use std::time::Duration;

/// Send an outgoing heartbeat ping via HTTP GET (U-06).
pub async fn send_heartbeat_ping(url: &str) -> Result<u16> {
    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;

    let resp = client.get(url).send().await?;
    Ok(resp.status().as_u16())
}
