//! System, GPU, Thermal, Fan & Swap telemetry for cashell status bar.
//!
//! Uses `sysinfo` for CPU, RAM, Swap, Disk, and CPU temperature.
//! GPU telemetry queries `nvidia-smi` or `rocm-smi` (or `/sys/class/drm`).

use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct SysSnapshot {
    pub cpu_pct: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
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
}

pub fn snapshot() -> SysSnapshot {
    use sysinfo::{Components, Disks, System};

    let mut sys = System::new_all();
    sys.refresh_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();

    let cpu_count = sys.cpus().len();
    let cpu_pct: f32 = sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>()
        / cpu_count.max(1) as f32;

    let ram_total = sys.total_memory();
    let ram_used = sys.used_memory();
    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();

    let disks = Disks::new_with_refreshed_list();
    let (disk_used, disk_total) = disks.iter().fold((0u64, 0u64), |(u, t), d| {
        (
            u + (d.total_space() - d.available_space()),
            t + d.total_space(),
        )
    });

    let cpu_temp_c = Components::new_with_refreshed_list()
        .iter()
        .map(|c| c.temperature())
        .filter(|t| t.is_finite())
        .fold(0.0f32, f32::max);

    let (gpu_label, gpu_pct, gpu_mem_used_mb, gpu_mem_total_mb, gpu_temp_c) = probe_gpu();
    let fan_speed = probe_fan();

    SysSnapshot {
        cpu_pct,
        ram_used_mb: ram_used / 1024 / 1024,
        ram_total_mb: ram_total / 1024 / 1024,
        swap_used_mb: swap_used / 1024 / 1024,
        swap_total_mb: swap_total / 1024 / 1024,
        disk_used_gb: disk_used / 1024 / 1024 / 1024,
        disk_total_gb: disk_total / 1024 / 1024 / 1024,
        cpu_temp_c,
        gpu_label,
        gpu_pct,
        gpu_mem_used_mb,
        gpu_mem_total_mb,
        gpu_temp_c,
        fan_speed,
    }
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
    let ram_pct = if s.ram_total_mb > 0 {
        s.ram_used_mb * 100 / s.ram_total_mb
    } else {
        0
    };
    let dsk_pct = if s.disk_total_gb > 0 {
        s.disk_used_gb * 100 / s.disk_total_gb
    } else {
        0
    };
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
