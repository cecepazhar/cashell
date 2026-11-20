//! Per-application process grouping for the cockpit "Applications" table.
//!
//! Processes are grouped the same way KDE System Monitor does it: by the
//! systemd cgroup that the desktop session launched them in
//! (`app-<desktop-id>@<rand>.service` / `app-<desktop-id>-<n>.scope`,
//! `app-flatpak-<id>-<n>.scope`). Everything else in the user session becomes
//! "Background Services" and everything outside it "System Services".
//!
//! All values are read from the live system (`sysinfo`, `/proc`); nothing is
//! synthesised. Values the kernel won't expose to an unprivileged user (I/O
//! or GPU of other users' processes) are reported as unavailable.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use sysinfo::{Pid, System, Users};

pub const BACKGROUND_GROUP: &str = "Background Services";
pub const SYSTEM_GROUP: &str = "System Services";

/// One process inside an application group.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppProcess {
    pub pid: u32,
    pub name: String,
    /// CPU share of the whole machine (0..=100).
    pub cpu_pct: f32,
    pub mem_mb: u64,
    pub swap_mb: u64,
    pub gpu_pct: Option<f32>,
    pub read_kbps: f64,
    pub write_kbps: f64,
    /// TCP download / upload rate (KB/s); `None` when sockets aren't readable.
    pub down_kbps: Option<f64>,
    pub up_kbps: Option<f64>,
    pub user: String,
    pub is_stopped: bool,
}

/// Aggregated application row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppInfo {
    /// Stable grouping key (desktop id, or the group constant).
    pub key: String,
    /// Human name (desktop file `Name=`, else the key).
    pub name: String,
    pub cpu_pct: f32,
    pub mem_mb: u64,
    pub swap_mb: u64,
    /// `None` when no process in the group exposes DRM usage.
    pub gpu_pct: Option<f32>,
    pub read_kbps: f64,
    pub write_kbps: f64,
    pub down_kbps: Option<f64>,
    pub up_kbps: Option<f64>,
    /// Owner; "multiple" when processes belong to different users.
    pub user: String,
    /// True when every process in the group is stopped (SIGSTOP).
    pub is_stopped: bool,
    /// Whether whole-group actions (End/Kill/Pause) are allowed.
    /// False for the Background/System buckets, which mix unrelated daemons.
    pub group_actions: bool,
    /// Processes, heaviest CPU first.
    pub processes: Vec<AppProcess>,
}

impl AppInfo {
    pub fn pids(&self) -> Vec<u32> {
        self.processes.iter().map(|p| p.pid).collect()
    }
}

/// Where a process belongs according to its cgroup path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GroupKey {
    App(String),
    Background,
    System,
}

/// Undo systemd unit-name escaping (`\x2d` -> `-`).
fn systemd_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\x") {
        out.push_str(rest.get(..i).unwrap_or(""));
        let hex = rest.get(i + 2..i + 4).unwrap_or("");
        match u8::from_str_radix(hex, 16) {
            Ok(b) if hex.len() == 2 => {
                out.push(b as char);
                rest = rest.get(i + 4..).unwrap_or("");
            }
            _ => {
                out.push_str("\\x");
                rest = rest.get(i + 2..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// Classify a `/proc/<pid>/cgroup` file (cgroup v2 `0::<path>` line).
pub fn parse_cgroup_group(content: &str) -> GroupKey {
    let path = content
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .unwrap_or("")
        .trim();
    if !path.starts_with("/user.slice/") {
        return GroupKey::System;
    }
    let unit = path.rsplit('/').next().unwrap_or("");
    let Some(rest) = unit.strip_prefix("app-") else {
        return GroupKey::Background;
    };
    let rest = rest
        .strip_suffix(".scope")
        .or_else(|| rest.strip_suffix(".service"))
        .unwrap_or(rest);
    // `<id>@<random>` (service) or `<id>-<digits>` (scope).
    let id = match rest.split_once('@') {
        Some((id, _)) => id,
        None => match rest.rsplit_once('-') {
            Some((id, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => id,
            _ => rest,
        },
    };
    let id = id.strip_prefix("flatpak-").unwrap_or(id);
    let id = systemd_unescape(id);
    if id.is_empty() {
        GroupKey::Background
    } else {
        GroupKey::App(id)
    }
}

/// Read `Name=` from the `[Desktop Entry]` section of a .desktop file.
pub fn parse_desktop_name(content: &str) -> Option<String> {
    let mut in_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_entry {
            if let Some(v) = line.strip_prefix("Name=") {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn desktop_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![
        "/usr/share/applications".into(),
        "/usr/local/share/applications".into(),
        "/var/lib/flatpak/exports/share/applications".into(),
    ];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/applications"));
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }
    dirs
}

fn lookup_desktop_name(id: &str) -> Option<String> {
    desktop_dirs().iter().find_map(|d| {
        std::fs::read_to_string(d.join(format!("{id}.desktop")))
            .ok()
            .and_then(|c| parse_desktop_name(&c))
    })
}

/// `VmSwap:` from `/proc/<pid>/status`, in kB.
pub fn parse_vm_swap_kb(status: &str) -> u64 {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmSwap:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// From one DRM fdinfo file: `(client_id, busiest engine busy-ns)`.
/// Returns `None` for non-DRM fds.
pub fn parse_drm_fdinfo(content: &str) -> Option<(String, u64)> {
    let mut client = None;
    let mut busiest: Option<u64> = None;
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("drm-client-id:") {
            client = Some(v.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("drm-engine-") {
            // drm-engine-render:  123456 ns
            if let Some((_, val)) = rest.split_once(':') {
                if let Some(ns) = val.split_whitespace().next().and_then(|n| n.parse().ok()) {
                    busiest = Some(busiest.map_or(ns, |b: u64| b.max(ns)));
                }
            }
        }
    }
    Some((client?, busiest?))
}

/// What one pass over `/proc/<pid>/fd` found.
#[derive(Default)]
struct FdScan {
    /// Socket inodes (`socket:[<inode>]`).
    sockets: Vec<u64>,
    /// Busiest-engine GPU time (ns), deduplicated by DRM client.
    gpu_ns: Option<u64>,
}

/// Scan a process's fds once for sockets and DRM clients. `None` when the
/// fd table isn't readable (another user's process).
fn scan_fds(pid: u32) -> Option<FdScan> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut scan = FdScan::default();
    let mut seen_clients = HashSet::new();
    for e in entries.flatten() {
        let Ok(target) = std::fs::read_link(e.path()) else {
            continue;
        };
        let t = target.to_string_lossy();
        if let Some(inode) = t
            .strip_prefix("socket:[")
            .and_then(|r| r.strip_suffix(']'))
            .and_then(|n| n.parse().ok())
        {
            scan.sockets.push(inode);
        } else if t.starts_with("/dev/dri/") {
            let info = format!("/proc/{pid}/fdinfo/{}", e.file_name().to_string_lossy());
            if let Some((client, ns)) = std::fs::read_to_string(info)
                .ok()
                .and_then(|c| parse_drm_fdinfo(&c))
            {
                if seen_clients.insert(client) {
                    scan.gpu_ns = Some(scan.gpu_ns.unwrap_or(0).saturating_add(ns));
                }
            }
        }
    }
    Some(scan)
}

// ── TCP byte counters via NETLINK_SOCK_DIAG (same source as `ss -ti`) ───────

const SOCK_DIAG_BY_FAMILY: u16 = 20;
const INET_DIAG_INFO: u16 = 2;
/// Offsets inside `struct tcp_info` (linux/tcp.h, stable since 4.1/4.2).
const TCPI_BYTES_ACKED_OFF: usize = 120;
const TCPI_BYTES_RECEIVED_OFF: usize = 128;

fn read_u64_ne(buf: &[u8], off: usize) -> Option<u64> {
    let b: [u8; 8] = buf.get(off..off + 8)?.try_into().ok()?;
    Some(u64::from_ne_bytes(b))
}
fn read_u32_ne(buf: &[u8], off: usize) -> Option<u32> {
    let b: [u8; 4] = buf.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_ne_bytes(b))
}
fn read_u16_ne(buf: &[u8], off: usize) -> Option<u16> {
    let b: [u8; 2] = buf.get(off..off + 2)?.try_into().ok()?;
    Some(u16::from_ne_bytes(b))
}

/// Parse a sock_diag netlink dump buffer into `inode -> (sent, received)`.
/// Returns `true` once NLMSG_DONE / NLMSG_ERROR is seen.
pub fn parse_sock_diag(buf: &[u8], out: &mut HashMap<u64, (u64, u64)>) -> bool {
    const NLMSG_HDR: usize = 16;
    const INET_DIAG_MSG: usize = 72; // family..inode
    const INODE_OFF: usize = 68;
    let mut off = 0usize;
    while let Some(len) = read_u32_ne(buf, off).map(|l| l as usize) {
        if len < NLMSG_HDR || off + len > buf.len() {
            break;
        }
        let ty = read_u16_ne(buf, off + 4).unwrap_or(0);
        if ty == libc::NLMSG_DONE as u16 || ty == libc::NLMSG_ERROR as u16 {
            return true;
        }
        let body = off + NLMSG_HDR;
        if let Some(inode) = read_u32_ne(buf, body + INODE_OFF) {
            // Walk rtattrs after inet_diag_msg.
            let mut a = body + INET_DIAG_MSG;
            let end = off + len;
            while a + 4 <= end {
                let alen = read_u16_ne(buf, a).unwrap_or(0) as usize;
                let aty = read_u16_ne(buf, a + 2).unwrap_or(0);
                if alen < 4 {
                    break;
                }
                if aty == INET_DIAG_INFO {
                    let info = buf.get(a + 4..(a + alen).min(end)).unwrap_or(&[]);
                    if let (Some(sent), Some(recv)) = (
                        read_u64_ne(info, TCPI_BYTES_ACKED_OFF),
                        read_u64_ne(info, TCPI_BYTES_RECEIVED_OFF),
                    ) {
                        out.insert(u64::from(inode), (sent, recv));
                    }
                }
                a += (alen + 3) & !3;
            }
        }
        off += (len + 3) & !3;
    }
    false
}

/// Dump every TCP socket (v4 + v6) with its byte counters. Unprivileged:
/// the kernel returns all sockets, the inode tells us who owns them.
fn tcp_socket_bytes() -> Option<HashMap<u64, (u64, u64)>> {
    // SAFETY: plain socket(2); the fd is closed below on every path.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            libc::NETLINK_SOCK_DIAG,
        )
    };
    if fd < 0 {
        return None;
    }
    let mut out = HashMap::new();
    let mut ok = true;
    for family in [libc::AF_INET as u8, libc::AF_INET6 as u8] {
        // nlmsghdr (16) + inet_diag_req_v2 (56)
        let mut req = [0u8; 72];
        let put32 = |r: &mut [u8; 72], o: usize, v: u32| {
            if let Some(s) = r.get_mut(o..o + 4) {
                s.copy_from_slice(&v.to_ne_bytes());
            }
        };
        put32(&mut req, 0, 72);
        if let Some(s) = req.get_mut(4..6) {
            s.copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
        }
        let flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
        if let Some(s) = req.get_mut(6..8) {
            s.copy_from_slice(&flags.to_ne_bytes());
        }
        if let Some(b) = req.get_mut(16) {
            *b = family;
        }
        if let Some(b) = req.get_mut(17) {
            *b = libc::IPPROTO_TCP as u8;
        }
        if let Some(b) = req.get_mut(18) {
            *b = 1 << (INET_DIAG_INFO - 1); // idiag_ext
        }
        put32(&mut req, 20, u32::MAX); // idiag_states: all
                                       // SAFETY: send a fully initialised local buffer of known length.
        let sent = unsafe { libc::send(fd, req.as_ptr().cast(), req.len(), 0) };
        if sent < 0 {
            ok = false;
            break;
        }
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            // SAFETY: recv into an owned buffer; length bounds the write.
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n <= 0 {
                break;
            }
            let chunk = buf.get(..n as usize).unwrap_or(&[]);
            if parse_sock_diag(chunk, &mut out) {
                break;
            }
        }
    }
    // SAFETY: closing the fd we opened above.
    unsafe { libc::close(fd) };
    ok.then_some(out)
}

#[derive(Default)]
struct Caches {
    /// pid -> group (cgroup never changes for a running process).
    groups: HashMap<u32, GroupKey>,
    /// desktop id -> display name.
    names: HashMap<String, String>,
    /// pid -> last GPU busy ns.
    gpu_ns: HashMap<u32, u64>,
    /// socket inode -> last (sent, received) bytes.
    sock_bytes: HashMap<u64, (u64, u64)>,
    /// False until the first socket dump, so existing sockets' lifetime
    /// totals aren't reported as one huge burst.
    sock_primed: bool,
}

static CACHES: Mutex<Option<Caches>> = Mutex::new(None);

/// Build the application table from a `System` whose processes were just
/// refreshed with CPU, memory, disk usage and user. `elapsed_secs` is the
/// time since the previous refresh (for I/O and GPU rates).
pub fn collect_applications(sys: &System, users: &Users, elapsed_secs: f64) -> Vec<AppInfo> {
    let mut guard = CACHES.lock().unwrap_or_else(|e| e.into_inner());
    let caches = guard.get_or_insert_with(Caches::default);

    let ncpu = sys.cpus().len().max(1) as f32;
    let elapsed = elapsed_secs.max(0.1);
    let live: HashSet<u32> = sys.processes().keys().map(|p| p.as_u32()).collect();
    caches.groups.retain(|pid, _| live.contains(pid));
    caches.gpu_ns.retain(|pid, _| live.contains(pid));

    // Per-socket byte deltas since the previous tick.
    let sockets = tcp_socket_bytes();
    let mut sock_delta: HashMap<u64, (u64, u64)> = HashMap::new();
    if let Some(ref now) = sockets {
        for (inode, &(sent, recv)) in now {
            let (ps, pr) = match caches.sock_bytes.get(inode) {
                Some(&prev) => prev,
                None if caches.sock_primed => (0, 0),
                None => (sent, recv),
            };
            sock_delta.insert(*inode, (sent.saturating_sub(ps), recv.saturating_sub(pr)));
        }
        caches.sock_bytes = now.clone();
        caches.sock_primed = true;
    }

    let mut buckets: HashMap<GroupKey, Vec<AppProcess>> = HashMap::new();
    for (pid, proc_) in sys.processes() {
        // sysinfo lists every thread as a "process" on Linux; counting them
        // multiplies RSS/swap and fd scans. Keep real processes only, and
        // skip kernel threads (kthreadd = pid 2 and its children).
        if proc_.thread_kind().is_some()
            || proc_.parent() == Some(Pid::from_u32(2))
            || pid.as_u32() == 2
        {
            continue;
        }
        let pid_u = pid.as_u32();
        let key = caches
            .groups
            .entry(pid_u)
            .or_insert_with(|| {
                std::fs::read_to_string(format!("/proc/{pid_u}/cgroup"))
                    .map(|c| parse_cgroup_group(&c))
                    .unwrap_or(GroupKey::System)
            })
            .clone();

        let swap_mb = std::fs::read_to_string(format!("/proc/{pid_u}/status"))
            .map(|s| parse_vm_swap_kb(&s) / 1024)
            .unwrap_or(0);

        let scan = scan_fds(pid_u);
        let gpu_pct = match scan.as_ref().and_then(|s| s.gpu_ns) {
            Some(ns) => caches.gpu_ns.insert(pid_u, ns).map(|p| {
                ((ns.saturating_sub(p) as f64 / (elapsed * 1e9)) * 100.0).clamp(0.0, 100.0) as f32
            }),
            None => {
                caches.gpu_ns.remove(&pid_u);
                None
            }
        };
        let (down_kbps, up_kbps) = match (&scan, &sockets) {
            (Some(sc), Some(_)) => {
                let (up, down) = sc.sockets.iter().fold((0u64, 0u64), |(u, d), ino| {
                    let (s, r) = sock_delta.get(ino).copied().unwrap_or((0, 0));
                    (u + s, d + r)
                });
                (
                    Some(down as f64 / 1024.0 / elapsed),
                    Some(up as f64 / 1024.0 / elapsed),
                )
            }
            _ => (None, None),
        };

        let du = proc_.disk_usage();
        let user = proc_
            .user_id()
            .and_then(|uid| users.get_user_by_id(uid))
            .map(|u| u.name().to_string())
            .unwrap_or_else(|| "?".to_string());

        buckets.entry(key).or_default().push(AppProcess {
            pid: pid_u,
            name: proc_.name().to_string_lossy().to_string(),
            cpu_pct: proc_.cpu_usage() / ncpu,
            mem_mb: proc_.memory() / 1024 / 1024,
            swap_mb,
            gpu_pct,
            read_kbps: du.read_bytes as f64 / 1024.0 / elapsed,
            write_kbps: du.written_bytes as f64 / 1024.0 / elapsed,
            down_kbps,
            up_kbps,
            user,
            is_stopped: matches!(proc_.status(), sysinfo::ProcessStatus::Stop),
        });
    }

    let mut apps: Vec<AppInfo> = buckets
        .into_iter()
        .map(|(key, mut procs)| {
            procs.sort_by(|a, b| {
                b.cpu_pct
                    .partial_cmp(&a.cpu_pct)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.mem_mb.cmp(&a.mem_mb))
            });
            let (key_str, name, group_actions) = match &key {
                GroupKey::App(id) => {
                    let name = caches
                        .names
                        .entry(id.clone())
                        .or_insert_with(|| lookup_desktop_name(id).unwrap_or_else(|| id.clone()))
                        .clone();
                    (id.clone(), name, true)
                }
                GroupKey::Background => (
                    BACKGROUND_GROUP.to_string(),
                    BACKGROUND_GROUP.to_string(),
                    false,
                ),
                GroupKey::System => (SYSTEM_GROUP.to_string(), SYSTEM_GROUP.to_string(), false),
            };
            aggregate(key_str, name, group_actions, procs)
        })
        .collect();

    apps.sort_by(|a, b| {
        b.cpu_pct
            .partial_cmp(&a.cpu_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.mem_mb.cmp(&a.mem_mb))
    });
    apps
}

fn aggregate(key: String, name: String, group_actions: bool, procs: Vec<AppProcess>) -> AppInfo {
    let users: HashSet<&str> = procs.iter().map(|p| p.user.as_str()).collect();
    let user = match users.len() {
        0 => "?".to_string(),
        1 => users.into_iter().next().unwrap_or("?").to_string(),
        _ => "multiple".to_string(),
    };
    let gpu_pct = procs
        .iter()
        .filter_map(|p| p.gpu_pct)
        .fold(None, |acc: Option<f32>, g| Some(acc.unwrap_or(0.0) + g))
        .map(|g| g.min(100.0));
    AppInfo {
        key,
        name,
        cpu_pct: procs.iter().map(|p| p.cpu_pct).sum(),
        mem_mb: procs.iter().map(|p| p.mem_mb).sum(),
        swap_mb: procs.iter().map(|p| p.swap_mb).sum(),
        gpu_pct,
        read_kbps: procs.iter().map(|p| p.read_kbps).sum(),
        write_kbps: procs.iter().map(|p| p.write_kbps).sum(),
        down_kbps: sum_opt(procs.iter().map(|p| p.down_kbps)),
        up_kbps: sum_opt(procs.iter().map(|p| p.up_kbps)),
        user,
        is_stopped: !procs.is_empty() && procs.iter().all(|p| p.is_stopped),
        group_actions,
        processes: procs,
    }
}

/// Sum of the available values; `None` only if none were available.
fn sum_opt(it: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    it.flatten()
        .fold(None, |acc, v| Some(acc.unwrap_or(0.0) + v))
}

/// Signals the Applications table can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppSignal {
    /// SIGTERM — ask the app to quit.
    End,
    /// SIGKILL — force kill.
    Kill,
    /// SIGSTOP — freeze.
    Pause,
    /// SIGCONT — resume.
    Resume,
}

impl AppSignal {
    fn raw(self) -> libc::c_int {
        match self {
            AppSignal::End => libc::SIGTERM,
            AppSignal::Kill => libc::SIGKILL,
            AppSignal::Pause => libc::SIGSTOP,
            AppSignal::Resume => libc::SIGCONT,
        }
    }
}

/// Send `sig` to every pid. Refuses pid 0/1 and our own process.
/// Returns `(ok, failed)` counts.
pub fn signal_pids(pids: &[u32], sig: AppSignal) -> (usize, usize) {
    let me = std::process::id();
    let (mut ok, mut failed) = (0, 0);
    for &pid in pids {
        if pid <= 1 || pid == me {
            failed += 1;
            continue;
        }
        let Ok(raw_pid) = libc::pid_t::try_from(pid) else {
            failed += 1;
            continue;
        };
        // SAFETY: kill(2) has no memory-safety preconditions; we pass a
        // positive pid (never 0/-1 process-group broadcasts).
        let rc = unsafe { libc::kill(raw_pid, sig.raw()) };
        if rc == 0 {
            ok += 1;
        } else {
            failed += 1;
        }
    }
    (ok, failed)
}

/// Lower scheduling priority (`nice +10`, capped at 19). Unprivileged users
/// can only lower priority, which is what this does.
pub fn lower_priority(pids: &[u32]) -> (usize, usize) {
    let (mut ok, mut failed) = (0, 0);
    for &pid in pids {
        if pid <= 1 {
            failed += 1;
            continue;
        }
        let who: libc::id_t = pid;
        // SAFETY: get/setpriority take plain integers; errno is reset first
        // because -1 is a valid getpriority() return value.
        let rc = unsafe {
            *libc::__errno_location() = 0;
            let cur = libc::getpriority(libc::PRIO_PROCESS, who);
            let target = (cur + 10).min(19);
            libc::setpriority(libc::PRIO_PROCESS, who, target)
        };
        if rc == 0 {
            ok += 1;
        } else {
            failed += 1;
        }
    }
    (ok, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_kde_service_app() {
        let c = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.kde.konsole@a1b2c3.service\n";
        assert_eq!(
            parse_cgroup_group(c),
            GroupKey::App("org.kde.konsole".into())
        );
    }

    #[test]
    fn cgroup_flatpak_scope_app() {
        let c = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-org.telegram.desktop-12345.scope";
        assert_eq!(
            parse_cgroup_group(c),
            GroupKey::App("org.telegram.desktop".into())
        );
    }

    #[test]
    fn cgroup_escaped_dash() {
        let c = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-brave\\x2dbrowser-4242.scope";
        assert_eq!(parse_cgroup_group(c), GroupKey::App("brave-browser".into()));
    }

    #[test]
    fn cgroup_background_and_system() {
        let bg = "0::/user.slice/user-1000.slice/user@1000.service/session.slice/plasma-kwin_wayland.service";
        assert_eq!(parse_cgroup_group(bg), GroupKey::Background);
        assert_eq!(
            parse_cgroup_group("0::/system.slice/docker.service"),
            GroupKey::System
        );
        assert_eq!(parse_cgroup_group("0::/init.scope"), GroupKey::System);
    }

    #[test]
    fn desktop_name_only_from_desktop_entry() {
        let c = "[Desktop Action new]\nName=New Window\n[Desktop Entry]\nType=Application\nName=Konsole\n";
        assert_eq!(parse_desktop_name(c).as_deref(), Some("Konsole"));
    }

    #[test]
    fn vm_swap_parsed() {
        assert_eq!(parse_vm_swap_kb("Name:\tfoo\nVmSwap:\t  2048 kB\n"), 2048);
        assert_eq!(parse_vm_swap_kb("Name:\tkthread\n"), 0);
    }

    #[test]
    fn drm_fdinfo_busiest_engine() {
        let c = "pos:\t0\ndrm-driver:\ti915\ndrm-client-id:\t42\ndrm-engine-render:\t5000 ns\ndrm-engine-video:\t9000 ns\n";
        assert_eq!(parse_drm_fdinfo(c), Some(("42".into(), 9000)));
        assert_eq!(parse_drm_fdinfo("pos:\t0\nflags:\t02\n"), None);
    }

    #[test]
    fn sock_diag_parses_inode_and_bytes() {
        // One SOCK_DIAG message: nlmsghdr + inet_diag_msg + INET_DIAG_INFO attr.
        let info_len = 136usize;
        let attr_len = 4 + info_len;
        let total = 16 + 72 + attr_len;
        let mut b = vec![0u8; total + 16];
        b[0..4].copy_from_slice(&(total as u32).to_ne_bytes());
        b[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
        b[16 + 68..16 + 72].copy_from_slice(&777u32.to_ne_bytes());
        let a = 16 + 72;
        b[a..a + 2].copy_from_slice(&(attr_len as u16).to_ne_bytes());
        b[a + 2..a + 4].copy_from_slice(&INET_DIAG_INFO.to_ne_bytes());
        b[a + 4 + 120..a + 4 + 128].copy_from_slice(&5000u64.to_ne_bytes());
        b[a + 4 + 128..a + 4 + 136].copy_from_slice(&9000u64.to_ne_bytes());
        // Trailing NLMSG_DONE.
        b[total..total + 4].copy_from_slice(&16u32.to_ne_bytes());
        b[total + 4..total + 6].copy_from_slice(&(libc::NLMSG_DONE as u16).to_ne_bytes());
        let mut out = HashMap::new();
        assert!(parse_sock_diag(&b, &mut out));
        assert_eq!(out.get(&777), Some(&(5000, 9000)));
    }

    #[test]
    fn aggregate_sums_and_flags() {
        let p = |pid, cpu, user: &str| AppProcess {
            pid,
            cpu_pct: cpu,
            mem_mb: 100,
            user: user.into(),
            ..Default::default()
        };
        let a = aggregate(
            "x".into(),
            "X".into(),
            true,
            vec![p(10, 5.0, "a"), p(11, 2.5, "a")],
        );
        assert_eq!(a.cpu_pct, 7.5);
        assert_eq!(a.mem_mb, 200);
        assert_eq!(a.user, "a");
        assert_eq!(a.gpu_pct, None);
        let b = aggregate(
            "y".into(),
            "Y".into(),
            true,
            vec![p(1, 0.0, "a"), p(2, 0.0, "b")],
        );
        assert_eq!(b.user, "multiple");
    }

    #[test]
    fn signal_refuses_init_and_self() {
        let (ok, failed) = signal_pids(&[0, 1, std::process::id()], AppSignal::Resume);
        assert_eq!((ok, failed), (0, 3));
    }
}
