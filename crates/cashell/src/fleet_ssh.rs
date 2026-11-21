//! Manual fleet servers: SSH connection arguments, remote telemetry probe and
//! the parser for its output.
//!
//! Servers are registered by the user (`[daemon.nodes]`); nothing is
//! discovered automatically. Each node has an auth method:
//! `key` (identity file or the ssh-agent), `password` (stored encrypted in
//! `secrets.enc`, handed to `ssh` through `SSH_ASKPASS`) or `tailscale`
//! (plain ssh over the node's Tailscale address).

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config::{NodeAuth, NodeConfig};

/// Seconds a successful/failed remote probe is reused before asking again.
const PROBE_TTL: Duration = Duration::from_secs(20);

/// Marker line used to split the remote command output into sections.
const SEP: &str = "---CASHELL---";

/// Single shell line run on the remote host. Read-only, POSIX `sh`.
pub const REMOTE_PROBE_CMD: &str =
    "cat /proc/loadavg; echo ---CASHELL---; nproc; echo ---CASHELL---; \
     grep -E '^(MemTotal|MemAvailable):' /proc/meminfo; echo ---CASHELL---; df -P / | tail -1";

/// Result of one remote probe.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeOutcome {
    /// SSH worked and the numbers were parsed.
    Metrics(RemoteMetrics),
    /// Host answers but login failed (wrong key/password, no access).
    AuthFailed,
    /// Host did not answer in time.
    Unreachable,
    /// First connection: the host key is not in `known_hosts` yet.
    HostKeyUnknown,
    /// The host key differs from the one in `known_hosts` (possible MITM).
    HostKeyChanged,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemoteMetrics {
    pub cpu_pct: f32,
    pub ram_pct: u8,
    pub disk_pct: u8,
}

/// Parse the output of [`REMOTE_PROBE_CMD`]. Returns `None` on any malformed
/// section: callers must show `--`, never invented numbers.
pub fn parse_metrics(out: &str) -> Option<RemoteMetrics> {
    let mut parts = out.split(SEP);
    let load = parts
        .next()?
        .split_whitespace()
        .next()?
        .parse::<f32>()
        .ok()?;
    let cores = parts.next()?.trim().parse::<f32>().ok()?;
    let mem = parts.next()?;
    let disk = parts.next()?;

    let mut total: Option<u64> = None;
    let mut avail: Option<u64> = None;
    for line in mem.lines() {
        let mut it = line.split_whitespace();
        match (it.next(), it.next().and_then(|v| v.parse::<u64>().ok())) {
            (Some("MemTotal:"), Some(v)) => total = Some(v),
            (Some("MemAvailable:"), Some(v)) => avail = Some(v),
            _ => {}
        }
    }
    let total = total.filter(|t| *t > 0)?;
    let avail = avail?.min(total);
    let ram_pct = ((total - avail) * 100 / total) as u8;

    // `df -P`: Filesystem 1024-blocks Used Available Capacity Mounted-on
    let cap = disk
        .split_whitespace()
        .find(|f| f.ends_with('%'))?
        .trim_end_matches('%')
        .parse::<u8>()
        .ok()?;

    if cores <= 0.0 {
        return None;
    }
    let cpu_pct = (load / cores * 100.0).clamp(0.0, 100.0);
    Some(RemoteMetrics {
        cpu_pct,
        ram_pct,
        disk_pct: cap.min(100),
    })
}

/// Path of the tiny helper `ssh` calls to obtain a password. It only echoes
/// the environment variable `CASHELL_SSH_PW`, so the password is never written
/// to disk and never appears on a command line.
fn askpass_path() -> std::path::PathBuf {
    crate::paths::config_dir().join("askpass.sh")
}

fn ensure_askpass() -> Option<std::path::PathBuf> {
    let p = askpass_path();
    let body = "#!/bin/sh\nprintf '%s\\n' \"$CASHELL_SSH_PW\"\n";
    if std::fs::read_to_string(&p).ok().as_deref() != Some(body) {
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::write(&p, body).ok()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(p)
}

/// Secret-store key for a node's password.
pub fn password_secret_key(node: &str) -> String {
    format!("node_password:{node}")
}

fn load_password(node: &str) -> Option<String> {
    crate::secrets::SecretStore::load_default()
        .ok()
        .and_then(|s| s.get_secret(&password_secret_key(node)).map(str::to_string))
}

/// Persist (or clear, when empty) a node's password in `secrets.enc`.
pub fn store_password(node: &str, password: &str) -> anyhow::Result<()> {
    let mut store = crate::secrets::SecretStore::load_default().unwrap_or_default();
    if password.is_empty() {
        store.remove_secret(&password_secret_key(node));
    } else {
        store.set_secret(&password_secret_key(node), password);
    }
    store.save_default()?;
    Ok(())
}

/// Build the `ssh` command for `node`. `remote_cmd = None` opens an
/// interactive session (inherits stdio); `Some(cmd)` runs it non-interactively.
pub fn ssh_command(name: &str, node: &NodeConfig, remote_cmd: Option<&str>) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("ConnectTimeout=4");
    // Unknown hosts must be confirmed by the user first (see `host_key_state`).
    cmd.arg("-o").arg("StrictHostKeyChecking=yes");
    cmd.arg("-p").arg(node.ssh_port().to_string());

    match node.auth {
        NodeAuth::Password => {
            cmd.arg("-o")
                .arg("PreferredAuthentications=password,keyboard-interactive");
            cmd.arg("-o").arg("PubkeyAuthentication=no");
            if remote_cmd.is_some() {
                cmd.arg("-o").arg("NumberOfPasswordPrompts=1");
            }
            if let (Some(script), Some(pw)) = (ensure_askpass(), load_password(name)) {
                cmd.env("SSH_ASKPASS", script)
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env("CASHELL_SSH_PW", pw);
                cmd.stdin(Stdio::null());
            }
        }
        NodeAuth::Key | NodeAuth::Tailscale => {
            if remote_cmd.is_some() {
                cmd.arg("-o").arg("BatchMode=yes");
            }
            if node.auth == NodeAuth::Key && !node.key_path.trim().is_empty() {
                cmd.arg("-i").arg(node.key_path.trim());
                cmd.arg("-o").arg("IdentitiesOnly=yes");
            }
        }
    }

    cmd.arg(format!("{}@{}", node.user, node.ip));
    if let Some(r) = remote_cmd {
        cmd.arg(r);
    }
    cmd
}

/// Run the telemetry probe over SSH (blocking, bounded by `ConnectTimeout`
/// plus a hard 8 s wall clock).
pub fn probe_ssh(name: &str, node: &NodeConfig) -> ProbeOutcome {
    if !crate::host_guard::allowed() {
        return ProbeOutcome::Unreachable;
    }
    let mut cmd = ssh_command(name, node, Some(REMOTE_PROBE_CMD));
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let Ok(child) = cmd.spawn() else {
        return ProbeOutcome::Unreachable;
    };
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let output = match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(o)) => o,
        _ => {
            // Hard timeout: do not leave a hung ssh behind.
            let _ = Command::new("kill").arg(pid.to_string()).status();
            return ProbeOutcome::Unreachable;
        }
    };
    if output.status.success() {
        return match parse_metrics(&String::from_utf8_lossy(&output.stdout)) {
            Some(m) => ProbeOutcome::Metrics(m),
            // Logged in but not a Linux /proc host: online, no numbers.
            None => ProbeOutcome::Unreachable,
        };
    }
    let err = String::from_utf8_lossy(&output.stderr).to_lowercase();
    if err.contains("remote host identification has changed") {
        ProbeOutcome::HostKeyChanged
    } else if err.contains("host key verification failed") {
        ProbeOutcome::HostKeyUnknown
    } else if err.contains("permission denied") || err.contains("authentication") {
        ProbeOutcome::AuthFailed
    } else {
        ProbeOutcome::Unreachable
    }
}

type Cache = Mutex<HashMap<String, (Instant, ProbeOutcome)>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cached variant of [`probe_ssh`]: a node is asked at most once per
/// [`PROBE_TTL`], so refreshing the dashboard does not spawn ssh each tick.
pub fn probe_ssh_cached(name: &str, node: &NodeConfig) -> ProbeOutcome {
    let key = format!("{name}|{}|{}|{:?}", node.ip, node.ssh_port(), node.auth);
    if let Ok(map) = cache().lock() {
        if let Some((at, out)) = map.get(&key) {
            if at.elapsed() < PROBE_TTL {
                return out.clone();
            }
        }
    }
    let out = probe_ssh(name, node);
    if let Ok(mut map) = cache().lock() {
        map.insert(key, (Instant::now(), out.clone()));
    }
    out
}

// ── Host key trust (TOFU) ───────────────────────────────────────────────────

/// What CAShell knows about a server's SSH host key.
#[derive(Debug, Clone, PartialEq)]
pub enum HostKeyState {
    /// Already in `~/.ssh/known_hosts`.
    Known,
    /// Not known yet: show `fingerprints` to the user and ask.
    Unknown {
        fingerprints: Vec<String>,
        scan: String,
    },
    /// Could not read the host key (host down / ssh-keyscan missing).
    Unavailable,
}

fn known_hosts_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".ssh").join("known_hosts"))
}

/// `known_hosts` lookup name: bare host on port 22, `[host]:port` otherwise.
pub fn known_hosts_name(node: &NodeConfig) -> String {
    if node.ssh_port() == 22 {
        node.ip.clone()
    } else {
        format!("[{}]:{}", node.ip, node.ssh_port())
    }
}

/// Fingerprint lines (`256 SHA256:... host (ED25519)`) from `ssh-keygen -lf -`
/// output; empty on malformed input.
pub fn parse_fingerprints(out: &str) -> Vec<String> {
    out.lines()
        .filter(|l| l.contains("SHA256:"))
        .map(|l| l.trim().to_string())
        .collect()
}

/// Check `known_hosts`; when the host is new, fetch its key with `ssh-keyscan`
/// and compute fingerprints for the confirmation dialog.
pub fn host_key_state(node: &NodeConfig) -> HostKeyState {
    if !crate::host_guard::allowed() {
        return HostKeyState::Unavailable;
    }
    let name = known_hosts_name(node);
    let known = Command::new("ssh-keygen")
        .args(["-F", &name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if known {
        return HostKeyState::Known;
    }
    let scan = Command::new("ssh-keyscan")
        .args(["-T", "4", "-p", &node.ssh_port().to_string(), &node.ip])
        .stderr(Stdio::null())
        .output();
    let Ok(scan) = scan else {
        return HostKeyState::Unavailable;
    };
    let scan_text = String::from_utf8_lossy(&scan.stdout).to_string();
    if scan_text.trim().is_empty() {
        return HostKeyState::Unavailable;
    }
    let fp = Command::new("ssh-keygen")
        .args(["-lf", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take()?.write_all(scan_text.as_bytes()).ok()?;
            c.wait_with_output().ok()
        })
        .map(|o| parse_fingerprints(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    if fp.is_empty() {
        return HostKeyState::Unavailable;
    }
    HostKeyState::Unknown {
        fingerprints: fp,
        scan: scan_text,
    }
}

/// Append the scanned keys to `known_hosts` after the user confirmed.
pub fn trust_host(node: &NodeConfig, scan: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let path = known_hosts_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    let name = known_hosts_name(node);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    for line in scan
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        // ssh-keyscan prints `<ip>[:port] keytype key`; rewrite the host part.
        let rest = line.split_once(' ').map(|x| x.1).unwrap_or("");
        if !rest.is_empty() {
            writeln!(f, "{name} {rest}")?;
        }
    }
    Ok(())
}

// ── Tailscale import (opt-in, never automatic) ──────────────────────────────

/// A peer reported by `tailscale status --json`.
#[derive(Debug, Clone, PartialEq)]
pub struct TailscalePeer {
    pub name: String,
    pub ip: String,
    pub online: bool,
}

/// Parse peers from `tailscale status --json`, sorted by name.
pub fn parse_tailscale_peers(json: &serde_json::Value) -> Vec<TailscalePeer> {
    let mut out: Vec<TailscalePeer> = json
        .get("Peer")
        .and_then(|p| p.as_object())
        .map(|peers| {
            peers
                .values()
                .filter_map(|p| {
                    let name = p.get("HostName")?.as_str()?.trim().to_string();
                    let ip = p
                        .get("TailscaleIPs")?
                        .as_array()?
                        .iter()
                        .filter_map(|v| v.as_str())
                        .find(|ip| !ip.contains(':'))?
                        .to_string();
                    if name.is_empty() {
                        return None;
                    }
                    Some(TailscalePeer {
                        name,
                        ip,
                        online: p.get("Online").and_then(|o| o.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// Ask the local `tailscale` for its peers. `None` when tailscale is absent.
pub fn list_tailscale_peers() -> Option<Vec<TailscalePeer>> {
    if !crate::host_guard::allowed() {
        return None;
    }
    let out = Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .ok()?;
    let json = serde_json::from_slice::<serde_json::Value>(&out.stdout).ok()?;
    Some(parse_tailscale_peers(&json))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0.52 0.40 0.35 1/320 12345\n---CASHELL---\n4\n---CASHELL---\n\
        MemTotal:        8000000 kB\nMemAvailable:    2000000 kB\n---CASHELL---\n\
        /dev/sda1 50000000 20000000 30000000 41% /\n";

    #[test]
    fn test_parse_metrics_valid() {
        let m = parse_metrics(SAMPLE).expect("parses");
        assert_eq!(m.ram_pct, 75);
        assert_eq!(m.disk_pct, 41);
        assert!((m.cpu_pct - 13.0).abs() < 0.5);
    }

    #[test]
    fn test_parse_metrics_rejects_garbage_and_caps_cpu() {
        assert!(parse_metrics("").is_none());
        assert!(parse_metrics("bash: nproc: not found").is_none());
        let hot = SAMPLE.replace("0.52 0.40", "40.0 40.0");
        assert_eq!(parse_metrics(&hot).map(|m| m.cpu_pct), Some(100.0));
        let no_total = SAMPLE.replace("MemTotal:        8000000", "MemTotal:        0");
        assert!(parse_metrics(&no_total).is_none());
    }

    #[test]
    fn test_ssh_command_per_auth_method() {
        let mut node = NodeConfig {
            ip: "10.0.0.5".into(),
            user: "deploy".into(),
            services: vec![],
            active: true,
            auth: NodeAuth::Key,
            key_path: "/home/u/.ssh/id_ed25519".into(),
        };
        let dbg = |c: &Command| format!("{c:?}");
        let key = dbg(&ssh_command("n", &node, Some("uptime")));
        assert!(key.contains("BatchMode=yes") && key.contains("-i"));
        assert!(key.contains("deploy@10.0.0.5"));

        node.auth = NodeAuth::Tailscale;
        let ts = dbg(&ssh_command("n", &node, Some("uptime")));
        assert!(!ts.contains("\"-i\""));

        // Interactive sessions must not force BatchMode (user can type).
        let inter = dbg(&ssh_command("n", &node, None));
        assert!(!inter.contains("BatchMode"));
    }

    #[test]
    fn test_probe_never_spawns_ssh_in_tests() {
        let node = NodeConfig {
            ip: "10.255.255.1".into(),
            user: "x".into(),
            services: vec![],
            active: true,
            auth: NodeAuth::Key,
            key_path: String::new(),
        };
        assert_eq!(probe_ssh("n", &node), ProbeOutcome::Unreachable);
    }
    #[test]
    fn test_parse_fingerprints_and_known_hosts_name() {
        let out = "256 SHA256:abc123 203.0.113.5 (ED25519)\n3072 SHA256:def456 203.0.113.5 (RSA)\n";
        let fp = parse_fingerprints(out);
        assert_eq!(fp.len(), 2);
        assert!(fp[0].contains("SHA256:abc123"));
        assert!(parse_fingerprints("garbage").is_empty());

        let mut node = NodeConfig {
            ip: "203.0.113.5".into(),
            user: "u".into(),
            services: vec![],
            active: true,
            auth: NodeAuth::Key,
            key_path: String::new(),
        };
        assert_eq!(known_hosts_name(&node), "203.0.113.5");
        node.services = vec![crate::config::ServiceConfig {
            name: "ssh".into(),
            port: 2222,
            protocol: "tcp".into(),
        }];
        assert_eq!(known_hosts_name(&node), "[203.0.113.5]:2222");
    }

    #[test]
    fn test_parse_tailscale_peers_picks_ipv4_and_sorts() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"Peer":{
              "k1":{"HostName":"zeta","TailscaleIPs":["fd7a::1","100.64.0.2"],"Online":true},
              "k2":{"HostName":"alpha","TailscaleIPs":["100.64.0.9"],"Online":false},
              "k3":{"HostName":"","TailscaleIPs":["100.64.0.3"]},
              "k4":{"HostName":"noip","TailscaleIPs":["fd7a::2"]}
            }}"#,
        )
        .expect("json");
        let peers = parse_tailscale_peers(&json);
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].name, "alpha");
        assert!(!peers[0].online);
        assert_eq!(peers[1].ip, "100.64.0.2");
        assert!(parse_tailscale_peers(&serde_json::json!({})).is_empty());
    }
}
