# CAShell (`cas`)

> **CAShell — Sovereign Developer Cockpit & Cloud Shell**  
> All-in-One CLI & TUI untuk ekosistem CA: btop-grade hardware telemetry, Dokploy-style fleet manager, automated security audit & code fix, SQLite embedded history, dan embedded Hana AI.

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20|%20macOS-green.svg)]()
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)]()

---

## ⚡ Quick Start

### 1. Launch Cockpit TUI (Fullscreen)
Cukup jalankan satu perintah di terminal:
```bash
cas
```
*(atau `cashell cockpit`)*

### 2. Common CLI Commands
```bash
# Codebase security audit (Turbo multi-worker)
cas audit --turbo --max-files 50

# Interactive AI code remediation & diff fix
cas fix

# Review git diff / staged changes
cas review --staged

# System & Fleet diagnostics
cas doctor

# View or update configuration
cas config show
cas config set ai.model ZA126_PRO
```

---

## 🖥️ Cockpit TUI Features (`cas`)

### 1. `[1] Overview` — btop-grade Telemetry & Dokploy Armada
- **Per-Core Grid (C0 - C7)**: Mini-meter per thread CPU Intel i7 ThinkPad X1 dengan frekuensi GHz riil.
- **Braille Curves**: Grafik riwayat penggunaan CPU per detik menggunakan karakter resolusi tinggi Unicode Braille (`⣠⣰⣸⣿`).
- **Thermal Throttle Alert**: Badge merah `[🔥 THERMAL ALERT]` menyala otomatis saat suhu CPU > 88°C.
- **Granular RAM & Mounts**: Rincian RAM (Used, Cached, Free) dan tabel partisi fisik NVMe `/` serta `/DATA`.
- **Network & Mesh Matrix**: Kecepatan transfer per detik (`▼ RX` & `▲ TX`) dan latensi ping ke Hostinger VPS (`100.76.150.46`), local 9router (`localhost:20128`), dan YPC Dokploy (`100.127.232.32`).
- **Live Dokploy Fleet Table**: Menampilkan seluruh proses agen (`hermes`, `dev-agent`, `qa-agent`, `biz-agent`, `content-agent`, `9router`, `cloudflared`, `Telegram Desktop`) beserta PID, role, status pill, CPU%, dan memory footprint.

### 2. `[2] Services / Apps` — Dokploy Matrix
- Kartu status aplikasi ekosistem CA: `caterm`, `fathstore`, `gcc`, `9router`, matriks port `8081-8088`.
- Indikator status: `● Healthy` (hijau), `● Listening` (cyan), `✖ Down` (merah).
- Tombol aksi: `[🔍 Probe Ports]`, `[⚙ Inspect Config]`.

### 3. `[3] Audit & Code` — Security Audit & Remediation
- Ringkasan riwayat audit (`audit/Audit_*.md`).
- Tombol pemicu: `[⚡ Turbo Audit]` dan `[🔧 Run Fix]`.

### 4. `[4] Settings GUI` — Visual Configuration Form
- Form visual interaktif untuk mengedit `~/.config/cashell/config.toml` tanpa perlu membuka teks editor:
  - **AI Provider & Model**: 9router, Omniroute, OpenAI, Anthropic, Ollama.
  - **API Key**: Masked input (`********`).
  - **Theme Presets**: `ca-classic`, `tokyo-night`, `monokai`, `nord`, `high-contrast`.
  - **Persona Settings**: Nama panggilan (`Prof. Cecep`) dan peran sistem asisten Hana AI.
  - **Database Retention**: Batas penyimpanan data (default 7 hari) dan status ukuran file DB.
- Tombol `[💾 Save Changes]`, `[Clear Chat History]`, dan `[Vacuum DB]`.

### 5. `[5] AI Chat` — Embedded Hana AI Assistant
- Terhubung langsung ke 9router lokal (`localhost:20128`).
- Menyimpan histori percakapan ke SQLite lokal (`cashell.db`) sehingga konteks tersambung antar sesi.
- Menyuntikkan konteks sistem (spesifikasi mesin, status port, Tailscale mesh) ke dalam obrolan.

---

## ⌨️ Shortcuts & Navigation

| Tombol | Aksi |
|---|---|
| `1` - `5` | Pindah tab langsung |
| `Shift + 1` - `5` (`!`, `@`, `#`, `$`, `%`) | Pindah tab langsung dari mana saja (termasuk saat sedang mengetik chat) |
| `Tab` / `Shift + Tab` | Berpindah fokus antar panel / form input |
| `↑` / `↓` atau `k` / `j` | Navigasi baris / scroll riwayat chat & proses |
| `Enter` | Kirim chat / Mulai edit field / Konfirmasi |
| `?` / `F1` | Buka popup modal bantuan shortcut cheat-sheet |
| `q` / `Esc` | Tutup modal bantuan / Keluar dari cockpit |
| `Mouse Click` | Klik tab untuk pindah, klik tombol aksi, scroll wheel untuk scrolling |

---

## 🗄️ SQLite Embedded Persistence (`cashell.db`)

Semua histori disimpan lokal di `~/.config/cashell/cashell.db` menggunakan SQLite WAL mode:
- **`ai_chat`**: Riwayat chat AI tersimpan otomatis.
- **`command_history`**: Log eksekusi perintah audit/fix.
- **`audit_reports`**: Arsip hasil audit keamanan.
- **`system_events`**: Catatan kejadian sistem (lonjakan thermal, koneksi mesh).
- **Auto-Prune**: Rekapitulasi otomatis menghapus data lebih tua dari batas retensi (default 7 hari) setiap kali CAShell dibuka.

---

## 🛠️ Instalasi & Kompilasi

Kompilasi dan pasang seluruh binary (`caorch`, `cashell`, symlink `cas`) dengan satu langkah:
```bash
~/Product/install-ca.sh
```

---

## 📄 Lisensi

MIT — © Cecep Saeful Azhar Hidayat, ST
