# CAShell (`cas`) v2.0.1

> **Sovereign DevOps Cockpit & Cloud Shell**  
> All-in-One CLI, TUI, dan 24/7 daemon mandiri untuk server engineering: btop-grade hardware telemetry, Caddy reverse proxy dengan SSL otomatis, UFW firewall + SSH auto-ban, zero-downtime container deploy, Uptime monitoring, privacy-first web analytics, dan AI BYOK.

[![Version](https://img.shields.io/badge/version-2.0.1-blue.svg)](Cargo.toml)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux-purple.svg)]()
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)]()

---

## ⚡ Quick Start

### 1. Launch Cockpit TUI (Fullscreen)
```bash
cas
# atau:
cashell cockpit
```

### 2. Common CLI Commands
```bash
# System diagnostics
cas doctor

# Server & Proxy management
cas server domain add api.example.com --proxy 8080
cas server firewall list
cas server db backup mydb --bucket my-backups

# Deploy engine
cas app list
cas app deploy myapp
cas app rollback myapp

# Security & Uptime
cas uptime probe
cas uptime status-page --output /var/www/status.html

# Codebase audit & AI remediation (BYOK)
cas audit
cas fix
```

---

## 📊 Measured Performance & Resource Footprint

Diukur secara riil pada arsitektur Linux x86_64 (baseline task V-02):
- **Daemon Memory (Idle 24/7)**: `VmRSS 2.2 MB` (2,248 kB)
- **Cockpit TUI Memory**: `VmRSS 4.8 MB` (4,752 kB)
- **Storage Tracker Script (`ca-track.js`)**: `< 1.2 KB` (gzipped)
- **Ketergantungan Eksternal**: Nol runtime dependencies, binary tunggal mandiri.

---

## 🛡️ Matriks Fitur Free vs Pro

Sesuai dokumen tata kelola lisensi tunggal (`05-tier-free-vs-pro.md`), CAShell memegang prinsip **Aturan Emas Open-Core**: seluruh fungsi yang berjalan lokal pada perangkat pengguna tanpa infrastruktur cloud Fathforce **gratis selamanya**. Pro hanya untuk kenyamanan multi-node, tim, dan sinkronisasi lintas perangkat terenkripsi.

### 1. Core & Cockpit
| Fitur | Free | Pro |
|---|:---:|:---:|
| CLI utilities (`doctor`, `git`, `git-status`, `project`, `version`, `config`) | ✅ | ✅ |
| Cockpit TUI untuk 1 server lokal | ✅ | ✅ |
| Cockpit multi-node fleet (>1 server via `caorch`) + pergantian cluster | ❌ | ✅ |
| Seluruh preset tema CATUI (`ca-classic`, `tokyo-night`, `monokai`, `nord`, dll) | ✅ | ✅ |

### 2. Server & Keamanan (DevOps Suite)
| Fitur | Free | Pro |
|---|:---:|:---:|
| Metrik CPU/RAM/Swap real-time & riwayat lokal tanpa batas buatan | ✅ | ✅ |
| Diagnostik SMART disk lengkap (`smartctl --json` + sysfs fallback) | ✅ | ✅ |
| Firewall UFW: inspeksi aturan, tambah, hapus (argv murni tanpa shell injection) | ✅ | ✅ |
| Auto-ban SSH brute-force login via journalctl dan pelepasan otomatis | ✅ | ✅ |
| Database: backup terjadwal ke bucket R2/S3 via SigV4 + pemulihan teruji | ✅ | ✅ |
| Virtual host reverse proxy + sertifikat SSL otomatis via Caddy Admin API | ✅ | ✅ |
| Hardening daemon: berjalan sebagai unprivileged user `cashell` + sudoers terbatas | ✅ | ✅ |

### 3. Deploy Engine
| Fitur | Free | Pro |
|---|:---:|:---:|
| Status Docker container & inspection logs (`bollard` API murni) | ✅ | ✅ |
| Auto-deploy via webhook Git dengan verifikasi HMAC signature | ✅ | ✅ |
| Antrean build berurutan & log eksekusi build ke SQLite | ✅ | ✅ |
| Rolling update zero-downtime & instant rollback commit sebelumnya | ✅ | ✅ |
| Injeksi secret variables dari vault terenkripsi lokal | ✅ | ✅ |

### 4. Uptime Monitoring
| Fitur | Free | Pro |
|---|:---:|:---:|
| Probe HTTP/TCP manual & daemon monitoring 24/7 untuk 1 node | ✅ | ✅ |
| Notifikasi insiden multi-channel (Telegram, Webhook, Discord) | ✅ | ✅ |
| Pemantauan masa berlaku SSL & peringatan dini berjenjang | ✅ | ✅ |
| Generator status page HTML statis tanpa kebocoran internal token | ✅ | ✅ |
| Outgoing heartbeat dead-man switch | ✅ | ✅ |
| Agregasi pemantauan uptime terpusat lintas node armada | ❌ | ✅ |

### 5. Web Analytics
| Fitur | Free | Pro |
|---|:---:|:---:|
| Query analitik, filter rentang tanggal, dan ekspor data CSV/JSON | ✅ | ✅ |
| Endpoint collector lokal (`POST /api/collect`) & tracker `ca-track.js` | ✅ | ✅ |
| Pemantauan jumlah domain/situs: tanpa batas | ✅ | ✅ |

### 6. AI & Otomasi (BYOK — Bring Your Own Key)
| Fitur | Free | Pro |
|---|:---:|:---:|
| Wizard `cashell ai setup` untuk OpenAI-compatible, Anthropic, Ollama | ✅ | ✅ |
| Chat TUI & diagnosis log otomatis di Cockpit (`Alt+E`) | ✅ | ✅ |
| Tool calling baca otomatis, tool mutasi/tulis wajib konfirmasi manusia (T-04) | ✅ | ✅ |
| Bot Telegram: kontrol server untuk 1 akun pemilik | ✅ | ✅ |
| Bot Telegram multi-pengguna dengan role (viewer/operator/admin) & audit log | ❌ | ✅ |

### 7. Lisensi & Sinkronisasi
| Fitur | Free | Pro |
|---|:---:|:---:|
| Penggunaan tanpa akun & tanpa batas perangkat lokal | ✅ | n/a |
| Zero-knowledge end-to-end encrypted sync (config, vault, snippets) via GCC | ❌ | ✅ |
| Pengelolaan lisensi mandiri & migrasi perangkat di member.fathforce.com | ❌ | ✅ |

---

## 📦 Instalasi & Service Systemd

### Instalasi Cepat
```bash
curl -fsSL https://cashell.fath.my.id/install.sh | bash
```

### Instalasi Service Daemon 24/7 (Hardened)
Untuk menjalankan monitoring background tanpa hak akses root:
```bash
# Mode user (lingering service)
cashell daemon install-service --user

# Mode system (user 'cashell' terisolasi)
sudo cashell daemon install-service
```
Perintah ini secara otomatis menghasilkan token otentikasi acak 32-byte pada konfigurasi dengan izin `0600` dan mengonfigurasi `systemd` unit dengan sandboxing (`NoNewPrivileges=true`, `ProtectSystem=strict`, `ProtectHome=read-only`).

---

## 📄 Lisensi

Didistribusikan di bawah lisensi MIT.  
Hak Cipta © Cecep Saeful Azhar Hidayat, ST.
