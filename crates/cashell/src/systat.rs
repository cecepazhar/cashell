//! System, GPU, Thermal, Fan & Swap telemetry for cashell status bar & cockpit.
//!
//! Uses `sysinfo` for CPU, RAM, Swap, Disk, CPU temperature, per-core metrics,
//! network throughput, mount points, and processes.
//! GPU telemetry queries `nvidia-smi` or `rocm-smi` (or `/sys/class/drm`).
//! Mesh peer latency queries Tailscale endpoints.

use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct CpuCoreInfo {
    pub core_id: usize,
    pub usage: f32,
    pub frequency_mhz: u64,
}

#[derive(Debug, Clone, Default)]
pub struct MountInfo {
    pub mountpoint: String,
    pub used_gb: u64,
    pub total_gb: u64,
    pub pct: u8,
    pub fstype: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_pct: f32,
    pub mem_mb: u64,
    pub status: String,
    pub role: String,
}

#[derive(Debug, Clone, Default)]
pub struct MeshPeer {
    pub name: String,
    pub endpoint: String,
    pub latency_ms: Option<u64>,
    pub status: String,
}

/// A running Docker/Podman container on the local host.
#[derive(Debug, Clone, Default)]
pub struct DockerContainerInfo {
    pub name: String,
    pub image: String,
    pub state: String, // "running", "exited", "paused", "created"
    pub status_detail: String,
}

/// A tracked project directory from ~/Product and ~/Project.
#[derive(Debug, Clone, Default)]
pub struct TrackedProject {
    pub name: String,
    pub category: String, // "Product" or "Project"
    pub branch: String,
    pub is_dirty: bool,
}

/// An Uptime Kuma style endpoint monitor with heartbeat history.
#[derive(Debug, Clone, Default)]
pub struct UptimeMonitor {
    pub name: String,
    pub target: String,
    pub is_up: bool,
    pub latency_ms: Option<u64>,
    pub status_code: Option<u16>,
    pub uptime_pct: f32,
    /// Last 20 probe results (true = up, false = down), newest last.
    pub history: Vec<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct SysSnapshot {
    pub cpu_pct: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub ram_available_mb: u64,
    pub ram_cached_mb: u64,
    pub swap_used_mb: u64,
    pub swap_total_mb: u64,
    pub disk_used_gb: u64,
    pub disk_total_gb: u64,
    pub cpu_temp_c: f32,
    pub gpu_label: String,
    pub gpu_pct: Option<f32>,
    pub gpu_mem_used_mb: Option<u64>,
    pub gpu_mem_total_mb: Option<u64>,
    pub gpu_temp_c: Option<f32>,
    pub fan_speed: Option<String>,
    pub load_avg: [f64; 3],
    pub cpus: Vec<CpuCoreInfo>,
    pub mounts: Vec<MountInfo>,
    pub net_rx_rate_kbps: f64,
    pub net_tx_rate_kbps: f64,
    pub net_total_rx_gb: f64,
    pub net_total_tx_gb: f64,
    pub active_net_iface: String,
    pub top_processes: Vec<ProcInfo>,
    pub mesh_peers: Vec<MeshPeer>,
    pub docker_containers: Vec<DockerContainerInfo>,
    pub tracked_projects: Vec<TrackedProject>,
    pub uptime_monitors: Vec<UptimeMonitor>,
}

#[derive(Debug, Clone, Copy)]
struct NetRateState {
    last_probe: Instant,
    total_rx_bytes: u64,
    total_tx_bytes: u64,
}

static NET_RATE_TRACKER: Mutex<Option<NetRateState>> = Mutex::new(None);

pub fn snapshot() -> SysSnapshot {
    use sysinfo::{Components, System};

    let mut sys = System::new_all();
    sys.refresh_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();

    let cpu_count = sys.cpus().len();
    let cpu_pct: f32 = sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>()
        / cpu_count.max(1) as f32;

    let load = System::load_average();
    let load_avg = [load.one, load.five, load.fifteen];

    let cpus: Vec<CpuCoreInfo> = sys
        .cpus()
        .iter()
        .enumerate()
        .map(|(i, c)| CpuCoreInfo {
            core_id: i,
            usage: c.cpu_usage(),
            frequency_mhz: c.frequency(),
        })
        .collect();

    let ram_total = sys.total_memory();
    let ram_used = sys.used_memory();
    let ram_available = sys.available_memory();
    let ram_cached = read_cached_mem_mb();

    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();

    let mounts = probe_mounts();
    let (disk_used_gb, disk_total_gb) = mounts
        .iter()
        .fold((0u64, 0u64), |(u, t), m| (u + m.used_gb, t + m.total_gb));

    let cpu_temp_c = Components::new_with_refreshed_list()
        .iter()
        .map(|c| c.temperature())
        .filter(|t| t.is_finite())
        .fold(0.0f32, f32::max);

    let (gpu_label, gpu_pct, gpu_mem_used_mb, gpu_mem_total_mb, gpu_temp_c) = probe_gpu();
    let fan_speed = probe_fan();

    let (
        net_rx_rate_kbps,
        net_tx_rate_kbps,
        net_total_rx_gb,
        net_total_tx_gb,
        active_net_iface,
    ) = probe_networks();

    let top_processes = probe_top_processes(&sys);
    let mesh_peers = probe_mesh();
    let docker_containers = probe_docker_containers();
    let tracked_projects = probe_tracked_projects();
    let uptime_monitors = probe_uptime_monitors();

    SysSnapshot {
        cpu_pct,
        ram_used_mb: ram_used / 1024 / 1024,
        ram_total_mb: ram_total / 1024 / 1024,
        ram_available_mb: ram_available / 1024 / 1024,
        ram_cached_mb: ram_cached,
        swap_used_mb: swap_used / 1024 / 1024,
        swap_total_mb: swap_total / 1024 / 1024,
        disk_used_gb,
        disk_total_gb,
        cpu_temp_c,
        gpu_label,
        gpu_pct,
        gpu_mem_used_mb,
        gpu_mem_total_mb,
        gpu_temp_c,
        fan_speed,
        load_avg,
        cpus,
        mounts,
        net_rx_rate_kbps,
        net_tx_rate_kbps,
        net_total_rx_gb,
        net_total_tx_gb,
        active_net_iface,
        top_processes,
        mesh_peers,
        docker_containers,
        tracked_projects,
        uptime_monitors,
    }
}

// ── Docker Containers Probe ─────────────────────────────────────────────────

pub fn probe_docker_containers() -> Vec<DockerContainerInfo> {
    let out = Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}|{{.Image}}|{{.State}}|{{.Status}}"])
        .output()
        .or_else(|_| {
            Command::new("podman")
                .args(["ps", "-a", "--format", "{{.Names}}|{{.Image}}|{{.State}}|{{.Status}}"])
                .output()
        });

    let mut list = Vec::new();
    if let Ok(o) = out {
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout);
            for line in text.lines() {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 4 {
                    list.push(DockerContainerInfo {
                        name: parts[0].trim().to_string(),
                        image: parts[1].trim().to_string(),
                        state: parts[2].trim().to_lowercase(),
                        status_detail: parts[3].trim().to_string(),
                    });
                }
            }
        }
    }
    list
}

// ── Tracked Projects Discovery ───────────────────────────────────────────────

pub fn probe_tracked_projects() -> Vec<TrackedProject> {
    let mut list = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for (dir_name, cat) in [("Product", "Product"), ("Project", "Project")] {
            let path = home.join(dir_name);
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                        if !name.starts_with('.') && !name.starts_with('_') && name != "target" && name != "node_modules" {
                            let is_git = p.join(".git").exists();
                            if is_git || p.join("Cargo.toml").exists() || p.join("package.json").exists() || p.join("go.mod").exists() {
                                list.push(TrackedProject {
                                    name,
                                    category: cat.to_string(),
                                    branch: "main".to_string(),
                                    is_dirty: false,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Include custom projects from config
    if let Ok(cfg) = crate::config::Config::load(None) {
        for cp in cfg.workspace.custom_projects {
            let expanded = if let Some(stripped) = cp.strip_prefix("~/") {
                if let Some(h) = dirs::home_dir() {
                    h.join(stripped).to_string_lossy().to_string()
                } else {
                    cp.clone()
                }
            } else {
                cp.clone()
            };
            let p = std::path::PathBuf::from(expanded);
            if p.is_dir() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                if !name.is_empty() && !list.iter().any(|item| item.name == name) {
                    list.push(TrackedProject {
                        name,
                        category: "Custom".to_string(),
                        branch: "main".to_string(),
                        is_dirty: false,
                    });
                }
            }
        }
    }

    // Include custom projects from SQLite database
    if let Ok(db) = crate::db::Database::open() {
        if let Ok(db_projs) = db.get_tracked_projects() {
            for proj in db_projs {
                if !list.iter().any(|item| item.name == proj.name) {
                    list.push(TrackedProject {
                        name: proj.name,
                        category: proj.category,
                        branch: proj.branch,
                        is_dirty: proj.is_dirty,
                    });
                }
            }
        }
    }

    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

// ── Uptime Kuma Style Endpoint Monitors ──────────────────────────────────────

static UPTIME_HISTORY_STORE: Mutex<Option<std::collections::HashMap<String, Vec<bool>>>> = Mutex::new(None);

type ProbeResult = (String, String, bool, Option<u64>, Option<u16>);

pub fn probe_uptime_monitors() -> Vec<UptimeMonitor> {
    use rayon::prelude::*;

    let default_targets = [
        ("Landing CATerm", "https://caterm.fathforce.com/"),
        ("FathStore Engine", "https://fathstore.com/"),
        ("CAUI Design System", "https://caui.fathforce.com/"),
        ("9router AI Gateway", "http://127.0.0.1:20128/health"),
        ("Dokploy YPC Host", "http://100.127.232.32:3000"),
        ("Hostinger Gateway", "https://100.76.150.46:8443"),
        ("CAShell Daemon", "http://127.0.0.1:8090/api/health"),
        ("CAOrch VPS Agent", "http://127.0.0.1:8091/api/health"),
    ];

    let mut targets: Vec<(String, String)> = default_targets
        .iter()
        .map(|&(n, u)| (n.to_string(), u.to_string()))
        .collect();

    // Load custom monitors from config
    if let Ok(cfg) = crate::config::Config::load(None) {
        for m in cfg.uptime.custom_monitors {
            if !targets.iter().any(|(_, u)| u == &m.target) {
                targets.push((m.name, m.target));
            }
        }
    }

    // Load custom monitors from SQLite
    if let Ok(db) = crate::db::Database::open() {
        if let Ok(db_monitors) = db.get_uptime_monitors() {
            for (name, url) in db_monitors {
                if !targets.iter().any(|(_, u)| u == &url) {
                    targets.push((name, url));
                }
            }
        }
    }

    let probe_results: Vec<ProbeResult> = targets
        .par_iter()
        .map(|(name, url)| {
            let start = Instant::now();
            if url.starts_with("http://") || url.starts_with("https://") {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_millis(1800))
                    .danger_accept_invalid_certs(true)
                    .build();

                if let Ok(c) = client {
                    if let Ok(res) = c.get(url).send() {
                        let lat = start.elapsed().as_millis() as u64;
                        let code = res.status().as_u16();
                        let is_up = res.status().is_success() || res.status().is_redirection() || code == 404 || code == 401;
                        return (name.to_string(), url.to_string(), is_up, Some(lat), Some(code));
                    }
                }
                (name.to_string(), url.to_string(), false, None, None)
            } else {
                (name.to_string(), url.to_string(), false, None, None)
            }
        })
        .collect();

    // Update historical 20-heartbeat ring in global store
    let mut store_guard = UPTIME_HISTORY_STORE.lock().unwrap();
    let store = store_guard.get_or_insert_with(std::collections::HashMap::new);

    let mut monitors = Vec::new();
    for (name, url, is_up, latency_ms, status_code) in probe_results {
        let hist = store.entry(name.clone()).or_insert_with(|| vec![true; 19]);
        hist.push(is_up);
        if hist.len() > 20 {
            hist.remove(0);
        }

        let up_count = hist.iter().filter(|&&u| u).count();
        let uptime_pct = (up_count as f32 / hist.len() as f32) * 100.0;

        monitors.push(UptimeMonitor {
            name,
            target: url,
            is_up,
            latency_ms,
            status_code,
            uptime_pct,
            history: hist.clone(),
        });
    }

    monitors
}

fn read_cached_mem_mb() -> u64 {
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if line.starts_with("Cached:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        return kb / 1024;
                    }
                }
            }
        }
    }
    0
}

fn probe_mounts() -> Vec<MountInfo> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    let mut list = Vec::new();
    for d in disks.iter() {
        let total = d.total_space();
        if total == 0 {
            continue;
        }
        let mp = d.mount_point().to_string_lossy().to_string();
        if mp.starts_with("/dev")
            || mp.starts_with("/sys")
            || mp.starts_with("/proc")
            || mp.starts_with("/run/credentials")
        {
            continue;
        }
        let avail = d.available_space();
        let used = total.saturating_sub(avail);
        let total_gb = total / 1024 / 1024 / 1024;
        let used_gb = used / 1024 / 1024 / 1024;
        let pct = if total > 0 {
            ((used as f64 / total as f64) * 100.0).round() as u8
        } else {
            0
        };
        let fstype = d.file_system().to_string_lossy().to_string();
        list.push(MountInfo {
            mountpoint: mp,
            used_gb,
            total_gb,
            pct,
            fstype,
        });
    }

    list.sort_by(|a, b| {
        let rank = |mp: &str| -> u8 {
            if mp == "/" {
                0
            } else if mp.contains("DATA") {
                1
            } else if mp == "/home" {
                2
            } else {
                3
            }
        };
        rank(&a.mountpoint)
            .cmp(&rank(&b.mountpoint))
            .then_with(|| a.mountpoint.cmp(&b.mountpoint))
    });

    list
}

fn probe_networks() -> (f64, f64, f64, f64, String) {
    use sysinfo::Networks;

    let nets = Networks::new_with_refreshed_list();
    let mut total_rx = 0u64;
    let mut total_tx = 0u64;
    let mut active_iface = String::from("wlp2s0");
    let mut max_traffic = 0u64;

    for (name, data) in nets.iter() {
        let rx = data.total_received();
        let tx = data.total_transmitted();
        if name != "lo" {
            total_rx += rx;
            total_tx += tx;
            let iface_traffic = rx + tx;
            if iface_traffic >= max_traffic {
                max_traffic = iface_traffic;
                active_iface = name.clone();
            }
        }
    }

    let now = Instant::now();
    let (rx_rate_kbps, tx_rate_kbps) = if let Ok(mut lock) = NET_RATE_TRACKER.lock() {
        if let Some(prev) = *lock {
            let elapsed_sec = now.duration_since(prev.last_probe).as_secs_f64().max(0.1);
            let d_rx = total_rx.saturating_sub(prev.total_rx_bytes);
            let d_tx = total_tx.saturating_sub(prev.total_tx_bytes);
            let rx_kbps = (d_rx as f64 / 1024.0) / elapsed_sec;
            let tx_kbps = (d_tx as f64 / 1024.0) / elapsed_sec;
            *lock = Some(NetRateState {
                last_probe: now,
                total_rx_bytes: total_rx,
                total_tx_bytes: total_tx,
            });
            (rx_kbps, tx_kbps)
        } else {
            *lock = Some(NetRateState {
                last_probe: now,
                total_rx_bytes: total_rx,
                total_tx_bytes: total_tx,
            });
            (0.0, 0.0)
        }
    } else {
        (0.0, 0.0)
    };

    let total_rx_gb = total_rx as f64 / (1024.0 * 1024.0 * 1024.0);
    let total_tx_gb = total_tx as f64 / (1024.0 * 1024.0 * 1024.0);

    (
        rx_rate_kbps,
        tx_rate_kbps,
        total_rx_gb,
        total_tx_gb,
        active_iface,
    )
}

fn probe_peer(name: &str, targets: &[&str], display_endpoint: &str) -> MeshPeer {
    let start = Instant::now();
    let mut online = false;
    for target in targets {
        if let Ok(addr) = target.parse::<SocketAddr>() {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok() {
                online = true;
                break;
            }
        }
    }
    if online {
        let lat = start.elapsed().as_millis().max(1) as u64;
        MeshPeer {
            name: name.to_string(),
            endpoint: display_endpoint.to_string(),
            latency_ms: Some(lat),
            status: "ONLINE".to_string(),
        }
    } else {
        MeshPeer {
            name: name.to_string(),
            endpoint: display_endpoint.to_string(),
            latency_ms: None,
            status: "OFFLINE".to_string(),
        }
    }
}

pub fn probe_mesh() -> Vec<MeshPeer> {
    use rayon::prelude::*;
    let peers: [(&str, &[&str], &str); 3] = [
        ("Hostinger VPS", &["100.76.150.46:22", "100.76.150.46:80"], "100.76.150.46"),
        ("9router Gateway", &["127.0.0.1:20128"], "localhost:20128"),
        ("YPC Dokploy", &["100.127.232.32:22", "100.127.232.32:80"], "100.127.232.32"),
    ];

    peers
        .into_par_iter()
        .map(|(name, targets, disp)| probe_peer(name, targets, disp))
        .collect()
}

pub fn assign_role(name: &str, cmd: &[std::ffi::OsString]) -> String {
    let name_lower = name.to_lowercase();
    let cmd_str = cmd
        .iter()
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    if cmd_str.contains("hermes") || name_lower.contains("hermes") {
        "AI Agent Runtime".to_string()
    } else if cmd_str.contains("9router") || name_lower.contains("9router") {
        "AI Proxy Gateway".to_string()
    } else if cmd_str.contains("cloudflared") || name_lower.contains("cloudflared") {
        "Edge Tunnel".to_string()
    } else if cmd_str.contains("caterm") || name_lower.contains("caterm") {
        "Web Terminal Console".to_string()
    } else if cmd_str.contains("fathstore") || name_lower.contains("fathstore") {
        "E-Commerce Engine".to_string()
    } else if cmd_str.contains("gcc") || name_lower.contains("gcc") {
        "Agent Dispatcher".to_string()
    } else if cmd_str.contains("qa-agent") || name_lower.contains("qa-agent") {
        "QA Test Agent".to_string()
    } else if cmd_str.contains("dev-agent") || name_lower.contains("dev-agent") {
        "Coding Dev Agent".to_string()
    } else if cmd_str.contains("biz-agent") || name_lower.contains("biz-agent") {
        "Biz Agent".to_string()
    } else if cmd_str.contains("content-agent") || name_lower.contains("content-agent") {
        "Content Agent".to_string()
    } else if cmd_str.contains("telegram") || name_lower.contains("telegram") {
        "Telegram Worker".to_string()
    } else if cmd_str.contains("zed") || name_lower.contains("zed") {
        "Code Editor".to_string()
    } else if name_lower == "node" {
        "Node.js Service".to_string()
    } else if name_lower.starts_with("python") {
        "Python Worker".to_string()
    } else if name_lower.contains("docker") || name_lower.contains("containerd") {
        "Container Daemon".to_string()
    } else {
        "System Service".to_string()
    }
}

fn probe_top_processes(sys: &sysinfo::System) -> Vec<ProcInfo> {
    let mut procs: Vec<ProcInfo> = sys
        .processes()
        .iter()
        .map(|(pid, proc)| {
            let name = proc.name().to_string_lossy().to_string();
            let cpu_pct = proc.cpu_usage();
            let mem_mb = proc.memory() / 1024 / 1024;
            let status = format!("{:?}", proc.status());
            let role = assign_role(&name, proc.cmd());
            ProcInfo {
                pid: pid.as_u32(),
                name,
                cpu_pct,
                mem_mb,
                status,
                role,
            }
        })
        .collect();

    procs.sort_by(|a, b| {
        b.cpu_pct
            .partial_cmp(&a.cpu_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.mem_mb.cmp(&a.mem_mb))
    });

    procs.truncate(20);
    procs
}

/// Probe NVIDIA or AMD ROCm GPU via CLI query
fn probe_gpu() -> (String, Option<f32>, Option<u64>, Option<u64>, Option<f32>) {
    // 1. Try nvidia-smi
    if let Ok(out) = Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,memory.used,memory.total,temperature.gpu,name",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().next() {
                let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
                if parts.len() >= 5 {
                    let util = parts[0].parse::<f32>().ok();
                    let mem_used = parts[1].parse::<u64>().ok();
                    let mem_total = parts[2].parse::<u64>().ok();
                    let temp = parts[3].parse::<f32>().ok();
                    let name = parts[4].to_string();
                    return (name, util, mem_used, mem_total, temp);
                }
            }
        }
    }

    // 2. Try rocm-smi
    if let Ok(out) = Command::new("rocm-smi")
        .args(["--showuse", "--showmemuse", "--showtemp", "--csv"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            for line in s.lines() {
                if line.contains("GPU") || line.contains("card") {
                    return ("AMD ROCm".to_string(), None, None, None, None);
                }
            }
        }
    }

    ("No GPU".to_string(), None, None, None, None)
}

/// Probe fan speed via hwmon sysfs (Linux)
fn probe_fan() -> Option<String> {
    if let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let path = entry.path();
            for i in 1..=4 {
                let fan_file = path.join(format!("fan{i}_input"));
                if let Ok(content) = std::fs::read_to_string(fan_file) {
                    if let Ok(rpm) = content.trim().parse::<u32>() {
                        if rpm > 0 {
                            return Some(format!("{rpm} RPM"));
                        }
                    }
                }
            }
        }
    }
    None
}

impl SysSnapshot {
    pub fn to_status_line(&self) -> String {
        status_bar_text(self)
    }
}

/// Format a concise telemetry bar for TUI rendering
pub fn status_bar_text(s: &SysSnapshot) -> String {
    let ram_pct = (s.ram_used_mb * 100).checked_div(s.ram_total_mb).unwrap_or(0);
    let dsk_pct = (s.disk_used_gb * 100).checked_div(s.disk_total_gb).unwrap_or(0);
    let swap_str = if s.swap_total_mb > 0 {
        format!(" SWP: {}/{}MB", s.swap_used_mb, s.swap_total_mb)
    } else {
        String::new()
    };
    let cpu_temp_str = if s.cpu_temp_c > 0.0 {
        format!(" {:.0}°C", s.cpu_temp_c)
    } else {
        String::new()
    };
    let gpu_str = if let Some(pct) = s.gpu_pct {
        format!(" GPU: {:.0}%", pct)
    } else if s.gpu_label != "No GPU" {
        format!(" GPU: {}", s.gpu_label)
    } else {
        " GPU: --".to_string()
    };
    let fan_str = if let Some(ref fan) = s.fan_speed {
        format!(" Fan: {fan}")
    } else {
        String::new()
    };

    format!(
        "CPU: {:.0}%{} | RAM: {}MB ({}%){} | DSK: {}/{}GB ({}%) | {}{}",
        s.cpu_pct,
        cpu_temp_str,
        s.ram_used_mb,
        ram_pct,
        swap_str,
        s.disk_used_gb,
        s.disk_total_gb,
        dsk_pct,
        gpu_str,
        fan_str,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assign_role_classification() {
        assert_eq!(
            assign_role("python3", &[std::ffi::OsString::from("python3"), std::ffi::OsString::from("hermes_cli.main")]),
            "AI Agent Runtime"
        );
        assert_eq!(
            assign_role("node", &[std::ffi::OsString::from("node"), std::ffi::OsString::from("/path/9router/cli.js")]),
            "AI Proxy Gateway"
        );
        assert_eq!(
            assign_role("cloudflared", &[std::ffi::OsString::from("cloudflared"), std::ffi::OsString::from("tunnel")]),
            "Edge Tunnel"
        );
        assert_eq!(
            assign_role("caterm-app", &[std::ffi::OsString::from("caterm-app")]),
            "Web Terminal Console"
        );
        assert_eq!(
            assign_role("systemd", &[std::ffi::OsString::from("/usr/lib/systemd/systemd")]),
            "System Service"
        );
    }

    #[test]
    fn test_snapshot_defaults() {
        let snap = SysSnapshot::default();
        assert_eq!(snap.cpu_pct, 0.0);
        assert_eq!(snap.ram_used_mb, 0);
        assert!(snap.cpus.is_empty());
        assert!(snap.mounts.is_empty());
        assert!(snap.top_processes.is_empty());
        assert!(snap.mesh_peers.is_empty());
        assert!(snap.docker_containers.is_empty());
        assert!(snap.tracked_projects.is_empty());
        assert!(snap.uptime_monitors.is_empty());
    }

    #[test]
    fn test_status_bar_text_with_data() {
        let snap = SysSnapshot {
            cpu_pct: 42.0,
            ram_used_mb: 8192,
            ram_total_mb: 16384,
            disk_used_gb: 120,
            disk_total_gb: 500,
            cpu_temp_c: 55.0,
            ..Default::default()
        };
        let text = status_bar_text(&snap);
        assert!(text.contains("CPU: 42% 55°C"));
        assert!(text.contains("RAM: 8192MB (50%)"));
        assert!(text.contains("DSK: 120/500GB (24%)"));
    }
}
