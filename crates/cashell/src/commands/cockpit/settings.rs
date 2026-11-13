//! Settings Tab & Modals — extracted from cockpit.rs (F-07, L-11, R-06).
//!
//! Subviews:
//! - [`SettingsSubView::Config`]: Interactive visual configuration editor for `config.toml`.
//! - [`SettingsSubView::License`]: Offline-first license screen with masked key input,
//!   status card, [Aktifkan], [Verifikasi sekarang], [Nonaktifkan perangkat ini] (L-11).
//! - [`SettingsSubView::DangerZone`]: Factory Reset trigger opening the confirm modal (R-06).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::sync::mpsc;
use std::thread;

use super::{cockpit_theme, CockpitApp, HitboxAction};
use crate::commands::license::{
    self, compute_days_left, compute_offline_grace_until, parse_display_ts, tier_display,
};
use crate::commands::reset;
use crate::license_gcc;
use crate::license_local;
use catui::layout::centered_rect;

// ── Settings Subview Selector ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsSubView {
    #[default]
    Config = 0,
    License = 1,
    DangerZone = 2,
}

// ── Settings GUI Form Fields ────────────────────────────────────────────────

pub const SETTINGS_FIELDS: &[&str] = &[
    "AI Provider",
    "Default Model",
    "API Key (Masked)",
    "Output Theme",
    "User Call Name",
    "Assistant Persona",
    "DB Retention Days",
    "DB Size (Read-only)",
    "Tailscale Target Host",
    "Doctor Target Port",
    "Port Matrix",
    "Master Password Protection",
    "Set/Change Master Password",
    "Password Hint",
    "Telegram Bot Token",
    "Telegram Chat ID",
    "Daemon Port",
    "Daemon Interval (s)",
    "Daemon Retry Threshold",
    "Coolify Base URL",
    "Coolify API Token",
    "DB Auto Prune",
    // ── Mascot & Emoticon Customization ──
    "Matrix Rain Effect",
    "Matrix Rain Speed",
    "Mascot Change Speed",
    "Mascot Sensitivity",
    "Mascot Hardware Tracking",
    "Mascot Color Scheme",
    "Mascot Animation Style",
    "Mascot Render Scale",
    "Mascot In AI Chat",
    "Audit Context Limit",
];

pub const PROVIDER_OPTIONS: &[&str] = &["", "openai", "anthropic", "local"];
pub const MASCOT_SPEED_OPTIONS: &[&str] = &["Fast", "Normal", "Slow", "Zen"];
pub const MASCOT_SENSITIVITY_OPTIONS: &[&str] = &["High", "Balanced", "Calm"];
pub const MASCOT_COLOR_OPTIONS: &[&str] = &[
    "Dynamic Reactive",
    "Phosphor Green",
    "Cyan Cyber",
    "Amber Retro",
    "Crimson Red",
    "Neon Violet",
];
pub const MASCOT_ANIMATION_OPTIONS: &[&str] = &[
    "Hover Bobbing",
    "Continuous 3D Spin",
    "Pulse Breathing",
    "Stationary",
];
pub const MASCOT_SCALE_OPTIONS: &[&str] = &["Auto-Fit", "Large", "Standard", "Compact"];
pub const MATRIX_SPEED_OPTIONS: &[&str] = &["Fast", "Normal", "Slow"];

// ── Field Editing Helpers ───────────────────────────────────────────────────

pub fn start_field_edit(app: &mut CockpitApp) {
    match app.selected_field {
        0 => app.cycle_field_value(true),
        1 => {
            // Default model
            let model = app
                .cfg
                .ai
                .providers
                .get(&app.cfg.ai.default_provider)
                .map(|p| p.default_model.clone())
                .unwrap_or_else(|| "ZA126_PRO".to_string());
            app.editing_field = Some(1);
            app.edit_buffer = model;
        }
        2 => {
            // API key
            let key = app
                .cfg
                .ai
                .providers
                .get(&app.cfg.ai.default_provider)
                .map(|p| p.api_key.clone())
                .unwrap_or_default();
            app.editing_field = Some(2);
            app.edit_buffer = key;
        }
        3 => app.cycle_field_value(true),
        4 => {
            // User call name
            app.editing_field = Some(4);
            app.edit_buffer = app.cfg.persona.user_name.clone();
        }
        5 => {
            // Assistant persona
            app.editing_field = Some(5);
            app.edit_buffer = app.cfg.persona.assistant_role.clone();
        }
        6 => {
            // DB retention days
            app.editing_field = Some(6);
            app.edit_buffer = app.cfg.database.retention_days.to_string();
        }
        7 => {
            let size = super::format_bytes(app.get_db_size());
            app.set_notification(format!("DB Size: {size} (read-only SQLite metric)"));
        }
        8 => {
            // Tailscale target host
            app.editing_field = Some(8);
            app.edit_buffer = app.cfg.doctor.target_addr.clone();
        }
        9 => {
            // Doctor target port
            app.editing_field = Some(9);
            app.edit_buffer = app.cfg.doctor.target_port.to_string();
        }
        10 => {
            // Port matrix
            let ports = app
                .cfg
                .doctor
                .ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            app.editing_field = Some(10);
            app.edit_buffer = ports;
        }
        11 => app.cycle_field_value(true),
        12 => {
            app.show_set_password_modal = true;
            app.set_password_field = 0;
            app.set_password_error = None;
        }
        13 => {
            // Password Hint
            app.editing_field = Some(13);
            app.edit_buffer = app.cfg.security.hint.clone();
        }
        14 => {
            // Telegram Bot Token
            app.editing_field = Some(14);
            let token = if !app.cfg.uptime.telegram_bot_token.is_empty() {
                app.cfg.uptime.telegram_bot_token.clone()
            } else {
                app.cfg.daemon.telegram_bot_token.clone()
            };
            app.edit_buffer = token;
        }
        15 => {
            // Telegram Chat ID
            app.editing_field = Some(15);
            let chat_id = if !app.cfg.uptime.telegram_chat_id.is_empty() {
                app.cfg.uptime.telegram_chat_id.clone()
            } else {
                app.cfg.daemon.telegram_chat_id.clone()
            };
            app.edit_buffer = chat_id;
        }
        16 => {
            // Daemon Port
            app.editing_field = Some(16);
            app.edit_buffer = app.cfg.daemon.port.to_string();
        }
        17 => {
            // Daemon Interval
            app.editing_field = Some(17);
            app.edit_buffer = app.cfg.daemon.interval_secs.to_string();
        }
        18 => {
            // Daemon Retry Threshold
            app.editing_field = Some(18);
            app.edit_buffer = app.cfg.daemon.retry_threshold.to_string();
        }
        19 => {
            // Coolify Base URL
            app.editing_field = Some(19);
            app.edit_buffer = app
                .cfg
                .deploy
                .coolify
                .as_ref()
                .map(|c| c.base_url.clone())
                .unwrap_or_default();
        }
        20 => {
            // Coolify API Token
            app.editing_field = Some(20);
            app.edit_buffer = app
                .cfg
                .deploy
                .coolify
                .as_ref()
                .map(|c| c.api_token.clone())
                .unwrap_or_default();
        }
        21..=31 => app.cycle_field_value(true),
        _ => {}
    }
}

pub fn commit_field_edit(app: &mut CockpitApp, field_idx: usize) {
    let val = app.edit_buffer.trim().to_string();
    match field_idx {
        1 => {
            let provider = app.cfg.ai.default_provider.clone();
            if let Some(p) = app.cfg.ai.providers.get_mut(&provider) {
                p.default_model = val.clone();
            }
            app.set_notification(format!("Model set to: {val}"));
        }
        2 => {
            let provider = app.cfg.ai.default_provider.clone();
            if let Some(p) = app.cfg.ai.providers.get_mut(&provider) {
                p.api_key = val.clone();
            }
            app.set_notification("API Key updated");
        }
        4 => {
            app.cfg.persona.user_name = val.clone();
            app.set_notification(format!("User call name set to: {val}"));
        }
        5 => {
            app.cfg.persona.assistant_role = val.clone();
            app.set_notification("Assistant persona updated");
        }
        6 => {
            if let Ok(days) = val.parse::<u32>() {
                app.cfg.database.retention_days = days.max(1);
                app.set_notification(format!(
                    "DB retention set to: {} days",
                    app.cfg.database.retention_days
                ));
            } else {
                app.set_notification("Invalid retention days (must be positive number)");
            }
        }
        8 => {
            app.cfg.doctor.target_addr = val.clone();
            app.set_notification(format!("Doctor target set to: {val}"));
        }
        9 => {
            if let Ok(port) = val.parse::<u16>() {
                app.cfg.doctor.target_port = port;
                app.set_notification(format!("Doctor port set to: {port}"));
            } else {
                app.set_notification("Invalid port number");
            }
        }
        10 => {
            let parsed_ports: Vec<u16> = val
                .split(',')
                .filter_map(|s| s.trim().parse::<u16>().ok())
                .collect();
            if !parsed_ports.is_empty() {
                app.cfg.doctor.ports = parsed_ports;
                app.apps = CockpitApp::init_apps(&app.cfg);
                app.probe_all_ports();
                app.set_notification("Port Matrix updated");
            } else {
                app.set_notification("Invalid port matrix list");
            }
        }
        13 => {
            app.cfg.security.hint = val.clone();
            app.set_notification(format!("Password hint set to: {val}"));
        }
        14 => {
            app.cfg.daemon.telegram_bot_token = val.clone();
            app.cfg.uptime.telegram_bot_token = val;
            app.set_notification("Telegram bot token updated");
        }
        15 => {
            app.cfg.daemon.telegram_chat_id = val.clone();
            app.cfg.uptime.telegram_chat_id = val.clone();
            app.set_notification(format!("Telegram chat ID set to: {val}"));
        }
        16 => {
            if let Ok(p) = val.parse::<u16>() {
                app.cfg.daemon.port = p;
                app.set_notification(format!("Daemon port set to: {p}"));
            } else {
                app.set_notification("Invalid port number");
            }
        }
        17 => {
            if let Ok(s) = val.parse::<u64>() {
                app.cfg.daemon.interval_secs = s.max(5);
                app.set_notification(format!("Daemon probe interval set to: {s}s"));
            } else {
                app.set_notification("Invalid interval number");
            }
        }
        18 => {
            if let Ok(r) = val.parse::<u32>() {
                app.cfg.daemon.retry_threshold = r.max(1);
                app.set_notification(format!("Daemon retry threshold set to: {r}"));
            } else {
                app.set_notification("Invalid threshold number");
            }
        }
        19 => {
            if app.cfg.deploy.coolify.is_none() {
                app.cfg.deploy.coolify = Some(crate::config::CoolifyConfig {
                    base_url: val.clone(),
                    api_token: String::new(),
                    timeout_seconds: 60,
                });
            } else if let Some(ref mut c) = app.cfg.deploy.coolify {
                c.base_url = val.clone();
            }
            app.set_notification(format!("Coolify base URL set to: {val}"));
        }
        20 => {
            if app.cfg.deploy.coolify.is_none() {
                app.cfg.deploy.coolify = Some(crate::config::CoolifyConfig {
                    base_url: String::new(),
                    api_token: val.clone(),
                    timeout_seconds: 60,
                });
            } else if let Some(ref mut c) = app.cfg.deploy.coolify {
                c.api_token = val.clone();
            }
            app.set_notification("Coolify API Token updated");
        }
        _ => {}
    }
}

pub fn cycle_field_value(app: &mut CockpitApp, forward: bool) {
    match app.selected_field {
        0 => {
            // AI Provider
            let current = &app.cfg.ai.default_provider;
            let cur_idx = PROVIDER_OPTIONS
                .iter()
                .position(|&p| p == current.as_str())
                .unwrap_or(0);
            let next_idx = if forward {
                (cur_idx + 1) % PROVIDER_OPTIONS.len()
            } else {
                (cur_idx + PROVIDER_OPTIONS.len() - 1) % PROVIDER_OPTIONS.len()
            };
            app.cfg.ai.default_provider = PROVIDER_OPTIONS[next_idx].to_string();
            app.set_notification(format!(
                "Provider set to: {}",
                if app.cfg.ai.default_provider.is_empty() {
                    "None"
                } else {
                    &app.cfg.ai.default_provider
                }
            ));
        }
        3 => {
            // Output Theme
            let current = &app.cfg.output.theme;
            let cur_idx = super::THEME_OPTIONS
                .iter()
                .position(|&t| t.eq_ignore_ascii_case(current.as_str()))
                .unwrap_or(0);
            let next_idx = if forward {
                (cur_idx + 1) % super::THEME_OPTIONS.len()
            } else {
                (cur_idx + super::THEME_OPTIONS.len() - 1) % super::THEME_OPTIONS.len()
            };
            app.cfg.output.theme = super::THEME_OPTIONS[next_idx].to_string();
            app.set_notification(format!("Theme set to: {}", app.cfg.output.theme));
        }
        11 => {
            // Master password protection
            app.cfg.security.require_password = !app.cfg.security.require_password;
            let state = if app.cfg.security.require_password {
                "ENABLED"
            } else {
                "DISABLED"
            };
            app.set_notification(format!("Master password lock: {state}"));
        }
        21 => {
            // DB Auto Prune
            app.cfg.database.auto_prune = !app.cfg.database.auto_prune;
            let state = if app.cfg.database.auto_prune {
                "ENABLED"
            } else {
                "DISABLED"
            };
            app.set_notification(format!("DB Auto-prune: {state}"));
        }
        22 => {
            // Matrix Rain Effect toggle
            app.cfg.output.matrix_rain = !app.cfg.output.matrix_rain;
            let state = if app.cfg.output.matrix_rain {
                "ENABLED"
            } else {
                "DISABLED"
            };
            app.set_notification(format!("Matrix Rain Effect: {state}"));
        }
        23 => {
            // Matrix Rain Speed (Fast, Normal, Slow, Zen)
            let current = &app.cfg.mascot.matrix_speed;
            let cur_idx = MASCOT_SPEED_OPTIONS
                .iter()
                .position(|&s| s.eq_ignore_ascii_case(current.as_str()))
                .unwrap_or(1);
            let next_idx = if forward {
                (cur_idx + 1) % MASCOT_SPEED_OPTIONS.len()
            } else {
                (cur_idx + MASCOT_SPEED_OPTIONS.len() - 1) % MASCOT_SPEED_OPTIONS.len()
            };
            app.cfg.mascot.matrix_speed = MASCOT_SPEED_OPTIONS[next_idx].to_string();
            app.set_notification(format!(
                "Matrix Rain Speed: {}",
                app.cfg.mascot.matrix_speed
            ));
        }
        24 => {
            // Mascot Change Speed
            let current = &app.cfg.mascot.speed;
            let cur_idx = MASCOT_SPEED_OPTIONS
                .iter()
                .position(|&s| s.eq_ignore_ascii_case(current.as_str()))
                .unwrap_or(1);
            let next_idx = if forward {
                (cur_idx + 1) % MASCOT_SPEED_OPTIONS.len()
            } else {
                (cur_idx + MASCOT_SPEED_OPTIONS.len() - 1) % MASCOT_SPEED_OPTIONS.len()
            };
            app.cfg.mascot.speed = MASCOT_SPEED_OPTIONS[next_idx].to_string();
            app.set_notification(format!("Mascot Speed: {}", app.cfg.mascot.speed));
        }
        25 => {
            // Mascot Sensitivity
            let current = &app.cfg.mascot.sensitivity;
            let cur_idx = MASCOT_SENSITIVITY_OPTIONS
                .iter()
                .position(|&s| s.eq_ignore_ascii_case(current.as_str()))
                .unwrap_or(1);
            let next_idx = if forward {
                (cur_idx + 1) % MASCOT_SENSITIVITY_OPTIONS.len()
            } else {
                (cur_idx + MASCOT_SENSITIVITY_OPTIONS.len() - 1) % MASCOT_SENSITIVITY_OPTIONS.len()
            };
            app.cfg.mascot.sensitivity = MASCOT_SENSITIVITY_OPTIONS[next_idx].to_string();
            app.set_notification(format!(
                "Mascot Sensitivity: {}",
                app.cfg.mascot.sensitivity
            ));
        }
        26 => {
            // Mascot Hardware Tracking toggle
            app.cfg.mascot.condition_tracking = !app.cfg.mascot.condition_tracking;
            let state = if app.cfg.mascot.condition_tracking {
                "ENABLED"
            } else {
                "DISABLED"
            };
            app.set_notification(format!("Mascot Hardware Tracking: {state}"));
        }
        27 => {
            // Mascot Color Scheme
            let current = &app.cfg.mascot.color_scheme;
            let cur_idx = MASCOT_COLOR_OPTIONS
                .iter()
                .position(|&s| s.eq_ignore_ascii_case(current.as_str()))
                .unwrap_or(0);
            let next_idx = if forward {
                (cur_idx + 1) % MASCOT_COLOR_OPTIONS.len()
            } else {
                (cur_idx + MASCOT_COLOR_OPTIONS.len() - 1) % MASCOT_COLOR_OPTIONS.len()
            };
            app.cfg.mascot.color_scheme = MASCOT_COLOR_OPTIONS[next_idx].to_string();
            app.set_notification(format!("Mascot Color: {}", app.cfg.mascot.color_scheme));
        }
        28 => {
            // Mascot Animation Style
            let current = &app.cfg.mascot.animation_style;
            let cur_idx = MASCOT_ANIMATION_OPTIONS
                .iter()
                .position(|&s| s.eq_ignore_ascii_case(current.as_str()))
                .unwrap_or(0);
            let next_idx = if forward {
                (cur_idx + 1) % MASCOT_ANIMATION_OPTIONS.len()
            } else {
                (cur_idx + MASCOT_ANIMATION_OPTIONS.len() - 1) % MASCOT_ANIMATION_OPTIONS.len()
            };
            app.cfg.mascot.animation_style = MASCOT_ANIMATION_OPTIONS[next_idx].to_string();
            app.set_notification(format!("Mascot Style: {}", app.cfg.mascot.animation_style));
        }
        29 => {
            // Mascot Render Scale
            let current = &app.cfg.mascot.render_scale;
            let cur_idx = MASCOT_SCALE_OPTIONS
                .iter()
                .position(|&s| s.eq_ignore_ascii_case(current.as_str()))
                .unwrap_or(1);
            let next_idx = if forward {
                (cur_idx + 1) % MASCOT_SCALE_OPTIONS.len()
            } else {
                (cur_idx + MASCOT_SCALE_OPTIONS.len() - 1) % MASCOT_SCALE_OPTIONS.len()
            };
            app.cfg.mascot.render_scale = MASCOT_SCALE_OPTIONS[next_idx].to_string();
            app.set_notification(format!("Mascot Scale: {}", app.cfg.mascot.render_scale));
        }
        30 => {
            // Mascot In AI Chat toggle
            app.cfg.mascot.chat_avatar_enabled = !app.cfg.mascot.chat_avatar_enabled;
            let state = if app.cfg.mascot.chat_avatar_enabled {
                "ENABLED"
            } else {
                "DISABLED"
            };
            app.set_notification(format!("Mascot in AI Chat: {state}"));
        }
        31 => {
            // Audit Context Limit cycle: 64KB -> 128KB -> 256KB -> 512KB -> 64KB
            let current = app.cfg.ai.audit_max_context_kb;
            let next = if forward {
                match current {
                    0..=64 => 128,
                    65..=128 => 256,
                    129..=256 => 512,
                    _ => 64,
                }
            } else {
                match current {
                    0..=64 => 512,
                    65..=128 => 64,
                    129..=256 => 128,
                    _ => 256,
                }
            };
            app.cfg.ai.audit_max_context_kb = next;
            app.cfg.ai.audit_context_limit_tokens = (next * 1024) / 4;
            app.set_notification(format!("Audit Context Limit set to: {next} KB"));
        }
        _ => {}
    }
}

// ── License Operations (L-11) ───────────────────────────────────────────────

pub fn trigger_license_activate(app: &mut CockpitApp) {
    if app.license_spinner_msg.is_some() {
        app.set_notification("Operasi lisensi sedang berjalan...");
        return;
    }
    let key = app.license_key_input.trim().to_string();
    if key.is_empty() {
        app.set_notification("Ketik atau masukkan kunci lisensi terlebih dahulu");
        return;
    }

    if let Err(e) = license::validate_license_format(&key) {
        app.set_notification(format!("Format kunci salah: {e}"));
        return;
    }

    let base_url = if !app.cfg.license.gcc_base_url.trim().is_empty() {
        app.cfg.license.gcc_base_url.clone()
    } else {
        "https://gcc.fathforce.com".to_string()
    };

    let (tx, rx) = mpsc::channel();
    app.license_rx = Some(rx);
    app.license_spinner_msg = Some("Mengaktifkan lisensi ke GCC...".to_string());
    app.set_notification("Menghubungi server GCC untuk aktivasi...");

    thread::spawn(move || {
        let res = license::activate_license_key(&key, &base_url)
            .map(|lic| {
                format!(
                    "Lisensi {} ({}) berhasil diaktifkan!",
                    license_gcc::mask_key(&lic.key),
                    tier_display(&lic.tier)
                )
            })
            .map_err(|e| format!("{e}"));
        let _ = tx.send(res);
    });
}

pub fn trigger_license_verify(app: &mut CockpitApp) {
    if app.license_spinner_msg.is_some() {
        app.set_notification("Operasi lisensi sedang berjalan...");
        return;
    }
    let lic = match license_local::load_license() {
        Ok(Some(l)) => l,
        _ => {
            app.set_notification("Tidak ada lisensi lokal terpasang untuk diverifikasi");
            return;
        }
    };

    let base_url = if !app.cfg.license.gcc_base_url.trim().is_empty() {
        app.cfg.license.gcc_base_url.clone()
    } else {
        "https://gcc.fathforce.com".to_string()
    };

    let (tx, rx) = mpsc::channel();
    app.license_rx = Some(rx);
    app.license_spinner_msg = Some("Memverifikasi lisensi ke GCC...".to_string());
    app.set_notification("Memverifikasi status lisensi ke GCC...");

    thread::spawn(move || {
        let res = license_local::verify_license(&lic, &base_url)
            .map(|()| "Verifikasi lisensi berhasil (status diperbarui)!".to_string())
            .map_err(|e| format!("Verifikasi gagal: {e}"));
        crate::entitlements::invalidate_cache();
        let _ = tx.send(res);
    });
}

pub fn trigger_license_deactivate(app: &mut CockpitApp) {
    if app.license_spinner_msg.is_some() {
        app.set_notification("Operasi lisensi sedang berjalan...");
        return;
    }
    let lic = match license_local::load_license() {
        Ok(Some(l)) => l,
        _ => {
            app.set_notification("Tidak ada lisensi terpasang");
            return;
        }
    };

    let base_url = if !app.cfg.license.gcc_base_url.trim().is_empty() {
        app.cfg.license.gcc_base_url.clone()
    } else {
        "https://gcc.fathforce.com".to_string()
    };

    let (tx, rx) = mpsc::channel();
    app.license_rx = Some(rx);
    app.license_spinner_msg = Some("Menonaktifkan perangkat di GCC...".to_string());
    app.set_notification("Menonaktifkan perangkat...");

    thread::spawn(move || {
        let key_masked = license_gcc::mask_key(&lic.key);
        let res = license::deactivate_license_device(&base_url, false)
            .map(|_| {
                format!(
                    "Perangkat berhasil dinonaktifkan dari {key_masked}. Beralih ke Free Edition."
                )
            })
            .or_else(|e| {
                // Graceful fallback for offline / 401: remove local file anyway
                if let Ok(path) = license_local::license_path() {
                    if path.exists() {
                        let _ = std::fs::remove_file(&path);
                    }
                }
                let _ = crate::pro::FastBootCache::clear();
                crate::entitlements::invalidate_cache();
                Ok(format!(
                    "Perangkat dinonaktifkan lokal ({e}). Beralih ke Free Edition."
                ))
            });
        let _ = tx.send(res);
    });
}

pub fn poll_license_async(app: &mut CockpitApp) {
    if let Some(ref rx) = app.license_rx {
        if let Ok(res) = rx.try_recv() {
            app.license_spinner_msg = None;
            app.license_rx = None;
            match res {
                Ok(msg) => {
                    app.set_notification(format!("✔ {msg}"));
                    crate::entitlements::invalidate_cache();
                }
                Err(err) => {
                    app.set_notification(format!("✖ {err}"));
                }
            }
        }
    }
}

// ── Danger Zone & Factory Reset (R-06) ──────────────────────────────────────

pub fn open_factory_reset_modal(app: &mut CockpitApp) {
    app.factory_reset_items = reset::collect_resettable_items();
    app.factory_reset_input.clear();
    app.factory_reset_btn_focus = 0; // [Batal] focused by default!
    app.factory_reset_warning = if let Ok(Some(lic)) = crate::license_local::load_license() {
        Some(format!(
            "Lisensi aktif {} ({}) akan dinonaktifkan di GCC.",
            license_gcc::mask_key(&lic.key),
            lic.product
        ))
    } else {
        None
    };
    app.show_factory_reset_modal = true;
    app.set_notification("⚠ Factory Reset: ketik 'RESET' untuk menghapus semua data");
}

pub fn close_factory_reset_modal(app: &mut CockpitApp) {
    app.show_factory_reset_modal = false;
    app.factory_reset_input.clear();
}

pub fn confirm_factory_reset(app: &mut CockpitApp) {
    if app.factory_reset_input.trim() == "RESET" {
        app.show_factory_reset_modal = false;
        app.factory_reset_requested = true;
        app.quit_requested = true;
    } else {
        app.set_notification("Ketik kata 'RESET' persis untuk mengonfirmasi!");
    }
}

pub fn handle_factory_reset_key(key: KeyEvent, app: &mut CockpitApp) {
    match key.code {
        KeyCode::Esc => {
            close_factory_reset_modal(app);
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
            app.factory_reset_btn_focus = (app.factory_reset_btn_focus + 1) % 2;
        }
        KeyCode::Enter => {
            if app.factory_reset_btn_focus == 1 {
                confirm_factory_reset(app);
            } else {
                close_factory_reset_modal(app);
            }
        }
        KeyCode::Backspace => {
            app.factory_reset_input.pop();
        }
        KeyCode::Char(c) if app.factory_reset_input.len() < 16 => {
            app.factory_reset_input.push(c);
        }
        _ => {}
    }
}

// ── Main Settings Tab Renderer ──────────────────────────────────────────────

pub fn draw_settings_tab(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    // ── Top Action & Subview Bar ────────────────────────────────────────────
    let action_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border));
    let action_inner = action_block.inner(chunks[0]);
    f.render_widget(action_block, chunks[0]);

    let view = app.settings_subview;

    let tab_config_rect = Rect {
        x: action_inner.x + 1,
        y: action_inner.y,
        width: 17,
        height: 1,
    };
    let tab_license_rect = Rect {
        x: action_inner.x + 19,
        y: action_inner.y,
        width: 15,
        height: 1,
    };
    let tab_danger_rect = Rect {
        x: action_inner.x + 35,
        y: action_inner.y,
        width: 17,
        height: 1,
    };
    let btn_save_rect = Rect {
        x: action_inner.x + 54,
        y: action_inner.y,
        width: 11,
        height: 1,
    };
    let btn_reload_rect = Rect {
        x: action_inner.x + 66,
        y: action_inner.y,
        width: 12,
        height: 1,
    };
    let btn_vacuum_rect = Rect {
        x: action_inner.x + 79,
        y: action_inner.y,
        width: 14,
        height: 1,
    };

    app.register_hitbox(tab_config_rect, HitboxAction::SettingsSelectSubView(0));
    app.register_hitbox(tab_license_rect, HitboxAction::SettingsSelectSubView(1));
    app.register_hitbox(tab_danger_rect, HitboxAction::SettingsSelectSubView(2));
    app.register_hitbox(btn_save_rect, HitboxAction::SaveSettings);
    app.register_hitbox(btn_reload_rect, HitboxAction::ReloadSettings);
    app.register_hitbox(btn_vacuum_rect, HitboxAction::VacuumDb);

    let tab_config = Span::styled(
        " [1] Konfigurasi ",
        if view == SettingsSubView::Config {
            Style::default()
                .fg(Color::Black)
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.text)
        },
    );
    let tab_license = Span::styled(
        " [2] Lisensi ",
        if view == SettingsSubView::License {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(0, 200, 255))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.text)
        },
    );
    let tab_danger = Span::styled(
        " [3] Danger Zone ",
        if view == SettingsSubView::DangerZone {
            Style::default()
                .fg(Color::White)
                .bg(palette.danger)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.danger)
        },
    );

    let btn_save = Span::styled(
        " [💾 Save] ",
        Style::default()
            .fg(Color::Black)
            .bg(palette.success)
            .add_modifier(Modifier::BOLD),
    );
    let btn_reload = Span::styled(
        " [↺ Reload] ",
        Style::default()
            .fg(Color::Black)
            .bg(palette.warning)
            .add_modifier(Modifier::BOLD),
    );
    let btn_vacuum = Span::styled(
        " [🧹 Vacuum] ",
        Style::default()
            .fg(Color::Black)
            .bg(palette.accent)
            .add_modifier(Modifier::BOLD),
    );

    let status = app
        .settings_status_msg
        .as_deref()
        .unwrap_or("1: Konfig | 2: Lisensi | 3: Danger | s: Save | r: Reload");

    f.render_widget(
        Paragraph::new(Line::from(vec![
            tab_config,
            Span::raw(" "),
            tab_license,
            Span::raw(" "),
            tab_danger,
            Span::raw(" │ "),
            btn_save,
            Span::raw(" "),
            btn_reload,
            Span::raw(" "),
            btn_vacuum,
            Span::styled(
                format!("  {status}"),
                Style::default().fg(palette.text_muted),
            ),
        ])),
        action_inner,
    );

    match view {
        SettingsSubView::Config => draw_config_subview(f, app, chunks[1]),
        SettingsSubView::License => draw_license_subview(f, app, chunks[1]),
        SettingsSubView::DangerZone => draw_danger_zone_subview(f, app, chunks[1]),
    }
}

// ── 1. Configuration Editor Subview ─────────────────────────────────────────

fn draw_config_subview(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let form_block = Block::default()
        .borders(Borders::ALL)
        .title(" Interactive Configuration Editor (~/.config/cashell/config.toml) ")
        .border_style(Style::default().fg(palette.border));
    let form_inner = form_block.inner(area);
    f.render_widget(form_block, area);

    let provider = &app.cfg.ai.default_provider;
    let model = app
        .cfg
        .ai
        .providers
        .get(provider)
        .map(|p| p.default_model.as_str())
        .unwrap_or("ZA126_PRO");

    let api_key = app
        .cfg
        .ai
        .providers
        .get(provider)
        .map(|p| p.api_key.as_str())
        .unwrap_or("");
    let masked_key = if api_key.is_empty() {
        "[Not Configured / None]"
    } else {
        "••••••••••••••••"
    };

    let theme = &app.cfg.output.theme;
    let db_size_str = super::format_bytes(app.get_db_size());
    let retention_str = format!(
        "{} days (auto_prune: {})",
        app.cfg.database.retention_days,
        if app.cfg.database.auto_prune {
            "on"
        } else {
            "off"
        }
    );
    let target_host = &app.cfg.doctor.target_addr;
    let target_port = app.cfg.doctor.target_port.to_string();
    let port_matrix = app
        .cfg
        .doctor
        .ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ");

    let pw_status = if app.cfg.security.require_password {
        "● ENABLED (Protected on startup)"
    } else {
        "○ Disabled (Open access)"
    };
    let pw_change_label = if app.cfg.security.master_password_hash.is_empty() {
        "[Click/Enter to Set Master Password]"
    } else {
        "•••••••• [Click/Enter to Change]"
    };
    let pw_hint = if app.cfg.security.hint.is_empty() {
        "[None]"
    } else {
        app.cfg.security.hint.as_str()
    };
    let tg_token_masked = if app.cfg.daemon.telegram_bot_token.is_empty() {
        "[Not Configured]"
    } else {
        "••••••••••••••••"
    };
    let tg_chat_id = if app.cfg.daemon.telegram_chat_id.is_empty() {
        "[Not Configured]"
    } else {
        app.cfg.daemon.telegram_chat_id.as_str()
    };
    let daemon_port_str = app.cfg.daemon.port.to_string();
    let daemon_interval_str = format!("{} seconds", app.cfg.daemon.interval_secs);
    let daemon_retry_str = format!("{} failures", app.cfg.daemon.retry_threshold);
    let coolify_url = if let Some(ref c) = app.cfg.deploy.coolify {
        c.base_url.as_str()
    } else {
        "[Not Configured]"
    };
    let coolify_token = if let Some(ref c) = app.cfg.deploy.coolify {
        if c.api_token.is_empty() {
            "[None]"
        } else {
            "••••••••"
        }
    } else {
        "[Not Configured]"
    };
    let auto_prune_str = if app.cfg.database.auto_prune {
        "Enabled"
    } else {
        "Disabled"
    };
    let matrix_rain_str = if app.cfg.output.matrix_rain {
        "Enabled (Matrix Rain Background)"
    } else {
        "Disabled (Clean Background)"
    };
    let matrix_speed_str = app.cfg.mascot.matrix_speed.as_str();
    let mascot_speed_str = match app.cfg.mascot.speed.as_str() {
        "Fast" => "Fast (2s interval)",
        "Slow" => "Slow (10s interval)",
        "Zen" => "Zen (20s interval)",
        _ => "Normal (5s interval)",
    };
    let mascot_sens_str = match app.cfg.mascot.sensitivity.as_str() {
        "High" => "High Reactive (>70°C / >70% Load)",
        "Calm" => "Calm (>90°C / >92% Load)",
        _ => "Balanced (>80°C / >85% Load)",
    };
    let mascot_track_str = if app.cfg.mascot.condition_tracking {
        "Enabled (Reflects CPU/RAM/Outage)"
    } else {
        "Disabled (Neutral)"
    };
    let mascot_color_str = app.cfg.mascot.color_scheme.as_str();
    let mascot_anim_str = app.cfg.mascot.animation_style.as_str();
    let mascot_scale_str = app.cfg.mascot.render_scale.as_str();
    let mascot_chat_str = if app.cfg.mascot.chat_avatar_enabled {
        "Enabled (Live Reactor & Avatars)"
    } else {
        "Disabled"
    };
    let audit_limit_str = format!(
        "{} KB ({} tokens)",
        app.cfg.ai.audit_max_context_kb, app.cfg.ai.audit_context_limit_tokens
    );

    let values: [&str; 32] = [
        provider.as_str(),
        model,
        masked_key,
        theme.as_str(),
        app.cfg.persona.user_name.as_str(),
        app.cfg.persona.assistant_role.as_str(),
        retention_str.as_str(),
        db_size_str.as_str(),
        target_host.as_str(),
        target_port.as_str(),
        port_matrix.as_str(),
        pw_status,
        pw_change_label,
        pw_hint,
        tg_token_masked,
        tg_chat_id,
        daemon_port_str.as_str(),
        daemon_interval_str.as_str(),
        daemon_retry_str.as_str(),
        coolify_url,
        coolify_token,
        auto_prune_str,
        matrix_rain_str,
        matrix_speed_str,
        mascot_speed_str,
        mascot_sens_str,
        mascot_track_str,
        mascot_color_str,
        mascot_anim_str,
        mascot_scale_str,
        mascot_chat_str,
        audit_limit_str.as_str(),
    ];

    let items: Vec<ListItem> = SETTINGS_FIELDS
        .iter()
        .enumerate()
        .map(|(i, &label)| {
            let is_selected = i == app.selected_field;
            let is_editing = app.editing_field == Some(i);

            let bg = if is_selected {
                palette.row_selected_bg
            } else {
                Color::Reset
            };

            let val_span = if is_editing {
                Span::styled(
                    format!(" [ {}█ ]", app.edit_buffer),
                    Style::default()
                        .fg(Color::Rgb(0, 0, 0))
                        .bg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    format!(" {}", values[i]),
                    Style::default()
                        .fg(if is_selected {
                            palette.text
                        } else {
                            palette.text_muted
                        })
                        .add_modifier(Modifier::BOLD),
                )
            };

            let prefix = if is_selected { "▶ " } else { "  " };

            let line = Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.accent)),
                Span::styled(
                    format!("{:<24}:", label),
                    Style::default().fg(if is_selected {
                        palette.accent
                    } else {
                        palette.text
                    }),
                ),
                val_span,
            ]);

            ListItem::new(line).style(Style::default().bg(bg))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.selected_field));
    let form_list = List::new(items);
    f.render_stateful_widget(form_list, form_inner, &mut state);

    // Register hitboxes for form rows
    for (i, _) in SETTINGS_FIELDS.iter().enumerate() {
        if (i as u16) < form_inner.height {
            let row_rect = Rect {
                x: form_inner.x,
                y: form_inner.y + (i as u16),
                width: form_inner.width,
                height: 1,
            };
            app.register_hitbox(row_rect, HitboxAction::SelectField(i));
        }
    }
}

// ── 2. License Subview (L-11) ───────────────────────────────────────────────

fn draw_license_subview(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12), // Status Card
            Constraint::Min(8),     // Input & Action Buttons
        ])
        .split(area);

    // ── Status Card ─────────────────────────────────────────────────────────
    let status_block = Block::default()
        .borders(Borders::ALL)
        .title(" ★ Status Lisensi CAShell (Offline-First) ")
        .border_style(Style::default().fg(palette.border));
    let status_inner = status_block.inner(chunks[0]);
    f.render_widget(status_block, chunks[0]);

    let lic_opt = license_local::load_license().ok().flatten();
    let is_pro = crate::entitlements::current().is_pro;

    let (tier_str, badge_span, active_period, days_left_str, dev_str, last_ver_str, grace_str) =
        match &lic_opt {
            Some(lic) => {
                let t_str = tier_display(&lic.tier).to_string();
                let b_span = if is_pro {
                    Span::styled(
                        " ● ACTIVE ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(palette.success)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if lic.revoked {
                    Span::styled(
                        " ✖ REVOKED ",
                        Style::default()
                            .fg(Color::White)
                            .bg(palette.danger)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(
                        " ✖ EXPIRED ",
                        Style::default()
                            .fg(Color::White)
                            .bg(palette.danger)
                            .add_modifier(Modifier::BOLD),
                    )
                };

                let p_str = match &lic.expires_at {
                    Some(ts) => format!("s.d. {}", parse_display_ts(ts)),
                    None => "Selamanya (Lifetime Perpetual)".to_string(),
                };

                let d_str = match compute_days_left(lic) {
                    Some(d) => format!("{d} hari lagi"),
                    None => "Permanent (Lifetime)".to_string(),
                };

                let d_dev = format!("{}/{}", lic.devices_used, lic.devices_max);
                let lv_str = if !lic.last_verified_at.is_empty() {
                    parse_display_ts(&lic.last_verified_at)
                } else {
                    "Belum pernah".to_string()
                };
                let g_str = compute_offline_grace_until(lic).unwrap_or_else(|| "-".to_string());

                (t_str, b_span, p_str, d_str, d_dev, lv_str, g_str)
            }
            None => (
                "Free Edition".to_string(),
                Span::styled(" ○ FREE ", Style::default().fg(palette.text_muted)),
                "Tidak terbatas (Fitur Dasar)".to_string(),
                "-".to_string(),
                "0/1".to_string(),
                "-".to_string(),
                "-".to_string(),
            ),
        };

    let masked_key = lic_opt
        .as_ref()
        .map(|l| license_gcc::mask_key(&l.key))
        .unwrap_or_else(|| "[Belum Terpasang]".to_string());

    let lines = vec![
        Line::from(vec![
            Span::styled(
                "  Kunci Lisensi       : ",
                Style::default().fg(palette.accent),
            ),
            Span::styled(masked_key, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("   "),
            badge_span,
        ]),
        Line::from(vec![
            Span::styled(
                "  Tier                : ",
                Style::default().fg(palette.accent),
            ),
            Span::styled(tier_str, Style::default().fg(palette.text)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Masa Aktif          : ",
                Style::default().fg(palette.accent),
            ),
            Span::styled(active_period, Style::default().fg(palette.text)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Sisa Hari           : ",
                Style::default().fg(palette.accent),
            ),
            Span::styled(days_left_str, Style::default().fg(palette.text)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Perangkat Terdaftar : ",
                Style::default().fg(palette.accent),
            ),
            Span::styled(dev_str, Style::default().fg(palette.text)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Verifikasi Terakhir : ",
                Style::default().fg(palette.accent),
            ),
            Span::styled(last_ver_str, Style::default().fg(palette.text)),
        ]),
        Line::from(vec![
            Span::styled(
                "  Batas Grace Offline : ",
                Style::default().fg(palette.accent),
            ),
            Span::styled(grace_str, Style::default().fg(palette.text_muted)),
        ]),
    ];

    f.render_widget(Paragraph::new(lines), status_inner);

    // ── Input & Action Buttons ──────────────────────────────────────────────
    let action_block = Block::default()
        .borders(Borders::ALL)
        .title(" Tindakan & Aktivasi Kunci ")
        .border_style(Style::default().fg(palette.border));
    let action_inner = action_block.inner(chunks[1]);
    f.render_widget(action_block, chunks[1]);

    let input_label = "Input Kunci Lisensi (format: CASH[99|24|12|01]-XXXXXX-C):";
    let is_editing = app.license_input_editing;
    let focus = app.license_action_focus;

    let key_display = if app.license_key_input.is_empty() {
        if is_editing {
            "█".to_string()
        } else {
            "[Ketik atau tempel kunci lisensi di sini...]".to_string()
        }
    } else if is_editing {
        format!("{}█", app.license_key_input)
    } else {
        // Masked key display
        license_gcc::mask_key(&app.license_key_input)
    };

    let key_box_style = if is_editing {
        Style::default()
            .fg(Color::Black)
            .bg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else if focus == 0 {
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.text_muted)
    };

    let btn_activate_style = if focus == 1 {
        Style::default()
            .fg(Color::Black)
            .bg(palette.success)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.success)
    };

    let btn_verify_style = if focus == 2 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(0, 200, 255))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(0, 200, 255))
    };

    let btn_deact_style = if focus == 3 {
        Style::default()
            .fg(Color::White)
            .bg(palette.danger)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.danger)
    };

    let spinner_line = if let Some(ref msg) = app.license_spinner_msg {
        let spinner_chars = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let spinner = spinner_chars[(app.pet_tick / 2) % spinner_chars.len()];
        Line::from(vec![
            Span::styled(
                format!("  {spinner} "),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg, Style::default().fg(Color::Yellow)),
        ])
    } else {
        Line::from("")
    };

    let content = vec![
        Line::from(Span::styled(
            input_label,
            Style::default().fg(palette.text_muted),
        )),
        Line::from(vec![
            Span::styled(
                if focus == 0 { "▶ " } else { "  " },
                Style::default().fg(palette.accent),
            ),
            Span::styled(format!(" [ {key_display} ] "), key_box_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [✓ Aktifkan] ", btn_activate_style),
            Span::raw("    "),
            Span::styled(" [↺ Verifikasi sekarang] ", btn_verify_style),
            Span::raw("    "),
            Span::styled(" [✖ Nonaktifkan perangkat ini] ", btn_deact_style),
        ]),
        Line::from(""),
        spinner_line,
    ];

    f.render_widget(Paragraph::new(content), action_inner);

    // Hitboxes
    let key_box_rect = Rect {
        x: action_inner.x + 2,
        y: action_inner.y + 1,
        width: action_inner.width.saturating_sub(4),
        height: 1,
    };
    let btn_act_rect = Rect {
        x: action_inner.x,
        y: action_inner.y + 3,
        width: 15,
        height: 1,
    };
    let btn_ver_rect = Rect {
        x: action_inner.x + 19,
        y: action_inner.y + 3,
        width: 25,
        height: 1,
    };
    let btn_deact_rect = Rect {
        x: action_inner.x + 48,
        y: action_inner.y + 3,
        width: 32,
        height: 1,
    };

    app.register_hitbox(key_box_rect, HitboxAction::LicenseFocusKeyInput);
    app.register_hitbox(btn_act_rect, HitboxAction::LicenseActivate);
    app.register_hitbox(btn_ver_rect, HitboxAction::LicenseVerify);
    app.register_hitbox(btn_deact_rect, HitboxAction::LicenseDeactivate);
}

// ── 3. Danger Zone Subview (R-06) ───────────────────────────────────────────

fn draw_danger_zone_subview(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ⚠ Danger Zone — Tindakan Berisiko Tinggi ")
        .border_style(Style::default().fg(palette.danger));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let desc = vec![
        Line::from(Span::styled(
            "PERINGATAN KERAS:",
            Style::default()
                .fg(palette.danger)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Factory Reset akan menghapus seluruh data lokal CAShell secara PERMANEN:"),
        Line::from("  • File konfigurasi (~/.config/cashell/config.toml)"),
        Line::from("  • Basis data SQLite (~/.local/share/cashell/cashell.db) & WAL journal"),
        Line::from("  • Kunci lisensi lokal & FastBootCache (revert ke Free Edition)"),
        Line::from("  • Riwayat audit, log audit, chat history, dan daemon PID file"),
        Line::from("  • Vault catatan & berkas knowledge base"),
        Line::from(""),
        Line::from("  Setelah reset berhasil, CAShell akan menutup antarmuka TUI Cockpit,"),
        Line::from("  menghapus berkas, menulis config default bersih, dan mencetak ringkasan ke terminal."),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " [⚠ BUKA FACTORY RESET] ",
                Style::default()
                    .fg(Color::White)
                    .bg(palette.danger)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" (Tekan Enter atau klik tombol di atas)", Style::default().fg(palette.text_muted)),
        ]),
    ];

    f.render_widget(Paragraph::new(desc), inner);

    let btn_rect = Rect {
        x: inner.x + 1,
        y: inner.y + 11,
        width: 25,
        height: 1,
    };
    app.register_hitbox(btn_rect, HitboxAction::OpenFactoryResetModal);
}

// ── Factory Reset Modal (R-06) ──────────────────────────────────────────────

pub fn draw_factory_reset_modal(f: &mut Frame, app: &mut CockpitApp, area: Rect) {
    let palette = cockpit_theme(&app.cfg.output.theme);
    let popup_area = centered_rect(72, 65, area);
    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ⚠ KONFIRMASI FACTORY RESET CASHELL (HAPUS TOTAL) ")
        .border_style(
            Style::default()
                .fg(palette.danger)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.width < 30 || inner.height < 10 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Warning line
            Constraint::Min(4),    // Items list
            Constraint::Length(2), // License warning if any
            Constraint::Length(3), // Input box
            Constraint::Length(2), // Action buttons
        ])
        .split(inner);

    // 1. Warning line
    let warn_p = Paragraph::new(Line::from(vec![
        Span::styled(
            "⚠ PERHATIAN: ",
            Style::default()
                .fg(palette.danger)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Item berikut yang ada di sistem akan DIHAPUS PERMANEN:",
            Style::default().fg(palette.text),
        ),
    ]));
    f.render_widget(warn_p, chunks[0]);

    // 2. Items list
    let item_lines: Vec<ListItem> = if app.factory_reset_items.is_empty() {
        vec![ListItem::new(
            "  (Tidak ada file data CAShell yang ditemukan di sistem)",
        )]
    } else {
        app.factory_reset_items
            .iter()
            .map(|(p, desc, size)| {
                ListItem::new(Line::from(vec![
                    Span::styled("  ✗ ", Style::default().fg(palette.danger)),
                    Span::styled(
                        format!("{:<26} ", desc),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("({size}) "), Style::default().fg(Color::Yellow)),
                    Span::styled(
                        p.display().to_string(),
                        Style::default().fg(palette.text_muted),
                    ),
                ]))
            })
            .collect()
    };
    let items_list = List::new(item_lines);
    f.render_widget(items_list, chunks[1]);

    // 3. License warning
    if let Some(ref w) = app.factory_reset_warning {
        let lic_warn = Paragraph::new(Line::from(vec![
            Span::styled(
                "  ⚠ ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(w, Style::default().fg(Color::Yellow)),
        ]));
        f.render_widget(lic_warn, chunks[2]);
    }

    // 4. Text input box
    let is_reset_match = app.factory_reset_input.trim() == "RESET";
    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(" Ketik kata 'RESET' persis untuk konfirmasi ")
        .border_style(Style::default().fg(if is_reset_match {
            palette.success
        } else {
            palette.danger
        }));
    let input_inner = input_block.inner(chunks[3]);
    f.render_widget(input_block, chunks[3]);

    let input_text = Line::from(vec![
        Span::styled(" > ", Style::default().fg(palette.accent)),
        Span::styled(
            format!("{}█", app.factory_reset_input),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(Paragraph::new(input_text), input_inner);

    // 5. Buttons: [Batal] focused by default (focus = 0), [Hapus Semua] (focus = 1)
    let btn_focus = app.factory_reset_btn_focus;
    let btn_cancel_style = if btn_focus == 0 {
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let btn_delete_style = if is_reset_match {
        if btn_focus == 1 {
            Style::default()
                .fg(Color::White)
                .bg(palette.danger)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(palette.danger)
                .add_modifier(Modifier::BOLD)
        }
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let delete_label = if is_reset_match {
        " [ ✖ HAPUS SEMUA ] "
    } else {
        " [ Hapus Semua (Ketik RESET) ] "
    };

    let btn_line = Line::from(vec![
        Span::styled(" [ Batal ] (Esc) ", btn_cancel_style),
        Span::raw("        "),
        Span::styled(delete_label, btn_delete_style),
    ]);
    f.render_widget(Paragraph::new(btn_line), chunks[4]);

    // Hitboxes
    let btn_cancel_rect = Rect {
        x: chunks[4].x,
        y: chunks[4].y,
        width: 17,
        height: 1,
    };
    let btn_delete_rect = Rect {
        x: chunks[4].x + 25,
        y: chunks[4].y,
        width: 25,
        height: 1,
    };
    app.register_hitbox(btn_cancel_rect, HitboxAction::CloseFactoryResetModal);
    if is_reset_match {
        app.register_hitbox(btn_delete_rect, HitboxAction::ConfirmFactoryReset);
    }
}

// ── Settings Key Handler ────────────────────────────────────────────────────

pub fn handle_settings_key(key: KeyEvent, app: &mut CockpitApp) -> bool {
    match app.settings_subview {
        SettingsSubView::Config => {
            if let Some(idx) = app.editing_field {
                match key.code {
                    KeyCode::Esc => {
                        app.editing_field = None;
                        app.edit_buffer.clear();
                    }
                    KeyCode::Enter => {
                        commit_field_edit(app, idx);
                        app.editing_field = None;
                        app.edit_buffer.clear();
                    }
                    KeyCode::Backspace => {
                        app.edit_buffer.pop();
                    }
                    KeyCode::Char(c) => {
                        app.edit_buffer.push(c);
                    }
                    _ => {}
                }
                return false;
            }
        }
        SettingsSubView::License => {
            if app.license_input_editing {
                match key.code {
                    KeyCode::Esc => {
                        app.license_input_editing = false;
                    }
                    KeyCode::Enter => {
                        app.license_input_editing = false;
                        trigger_license_activate(app);
                    }
                    KeyCode::Backspace => {
                        app.license_key_input.pop();
                    }
                    KeyCode::Char(c) if !c.is_control() => {
                        app.license_key_input.push(c);
                    }
                    _ => {}
                }
                return false;
            }

            match key.code {
                KeyCode::Tab | KeyCode::Right => {
                    app.license_action_focus = (app.license_action_focus + 1) % 4;
                    return false;
                }
                KeyCode::BackTab | KeyCode::Left => {
                    app.license_action_focus = (app.license_action_focus + 3) % 4;
                    return false;
                }
                KeyCode::Enter => {
                    match app.license_action_focus {
                        0 => app.license_input_editing = true,
                        1 => trigger_license_activate(app),
                        2 => trigger_license_verify(app),
                        3 => trigger_license_deactivate(app),
                        _ => {}
                    }
                    return false;
                }
                KeyCode::Char('a') => {
                    trigger_license_activate(app);
                    return false;
                }
                KeyCode::Char('v') => {
                    trigger_license_verify(app);
                    return false;
                }
                KeyCode::Char('d') => {
                    trigger_license_deactivate(app);
                    return false;
                }
                KeyCode::Char('k') => {
                    app.license_action_focus = 0;
                    app.license_input_editing = true;
                    return false;
                }
                _ => {}
            }
        }
        SettingsSubView::DangerZone => match key.code {
            KeyCode::Enter | KeyCode::Char('f') => {
                open_factory_reset_modal(app);
                return false;
            }
            _ => {}
        },
    }

    match key.code {
        KeyCode::Char('1') => {
            app.settings_subview = SettingsSubView::Config;
            app.set_notification("Settings: Konfigurasi");
            false
        }
        KeyCode::Char('2') => {
            app.settings_subview = SettingsSubView::License;
            app.set_notification("Settings: Lisensi");
            false
        }
        KeyCode::Char('3') => {
            app.settings_subview = SettingsSubView::DangerZone;
            app.set_notification("Settings: Danger Zone");
            false
        }
        KeyCode::Char('s') => {
            app.save_current_config();
            false
        }
        KeyCode::Char('r') => {
            app.reload_config();
            false
        }
        _ => false,
    }
}

// ── Tests (F-07, L-11, R-06) ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::license_local::LicenseFile;
    use crate::paths;
    use chrono::{Duration, Utc};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    struct TempTestHome {
        dir: std::path::PathBuf,
    }

    impl TempTestHome {
        fn new() -> Self {
            let id = SEQ.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "cashell-settings-test-{}-{}",
                std::process::id(),
                id
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }
    }

    impl Drop for TempTestHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn make_key_event(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_settings_subviews_navigation() {
        let mut app = CockpitApp::new(&Config::default());
        assert_eq!(app.settings_subview, SettingsSubView::Config);

        // Switch to License
        handle_settings_key(make_key_event(KeyCode::Char('2')), &mut app);
        assert_eq!(app.settings_subview, SettingsSubView::License);

        // Switch to Danger Zone
        handle_settings_key(make_key_event(KeyCode::Char('3')), &mut app);
        assert_eq!(app.settings_subview, SettingsSubView::DangerZone);

        // Switch back to Config
        handle_settings_key(make_key_event(KeyCode::Char('1')), &mut app);
        assert_eq!(app.settings_subview, SettingsSubView::Config);
    }

    #[test]
    fn test_header_license_badge_states() {
        let home = TempTestHome::new();
        paths::with_test_home(&home.dir, || {
            let palette = catui::Theme::default();
            let app = CockpitApp::new(&Config::default());

            // 1. No license -> [ FREE ]
            let _ = license_local::remove_license();
            crate::entitlements::invalidate_cache();
            let b = super::super::get_header_license_badge(&palette, &app);
            assert_eq!(b.content, " [ FREE ] ");

            // 2. Lifetime license (tier 99, expires_at: None) -> [ PRO · Lifetime ]
            let now_iso = Utc::now().to_rfc3339();
            let lic_lifetime = LicenseFile {
                schema: 2,
                key: "CASH99-ABCDEF-2".to_string(),
                product: "SH".to_string(),
                tier: "99".to_string(),
                expires_at: None,
                activated_at: now_iso.clone(),
                last_verified_at: now_iso.clone(),
                activation_token: None,
                devices_used: 1,
                devices_max: 1,
                offline_grace_until: None,
                revoked: false,
                expired: false,
            };
            license_local::save_license(&lic_lifetime).unwrap();
            crate::entitlements::invalidate_cache();
            let b = super::super::get_header_license_badge(&palette, &app);
            assert_eq!(b.content, " [ PRO · Lifetime ] ");

            // 3. Pro subscription with 20 days left (> 14 days) -> [ PRO · 20 hari ]
            let exp_20d = (Utc::now() + Duration::days(20)).to_rfc3339();
            let mut lic_20d = lic_lifetime.clone();
            lic_20d.tier = "12".to_string();
            lic_20d.expires_at = Some(exp_20d);
            license_local::save_license(&lic_20d).unwrap();
            crate::entitlements::invalidate_cache();
            let b = super::super::get_header_license_badge(&palette, &app);
            assert!(b.content.contains("PRO · 20 hari") || b.content.contains("PRO · 19 hari"));
            assert_eq!(b.style.bg, Some(palette.success));

            // 4. Pro subscription with 10 days left (<= 14 days) -> yellow badge
            let exp_10d = (Utc::now() + Duration::days(10)).to_rfc3339();
            let mut lic_10d = lic_lifetime.clone();
            lic_10d.tier = "12".to_string();
            lic_10d.expires_at = Some(exp_10d);
            license_local::save_license(&lic_10d).unwrap();
            crate::entitlements::invalidate_cache();
            let b = super::super::get_header_license_badge(&palette, &app);
            assert!(b.content.contains("PRO · 10 hari") || b.content.contains("PRO · 9 hari"));
            assert_eq!(b.style.bg, Some(Color::Yellow));

            // 5. Expired license -> [ PRO · Kedaluwarsa ] (red)
            let exp_past = (Utc::now() - Duration::days(2)).to_rfc3339();
            let mut lic_exp = lic_lifetime.clone();
            lic_exp.tier = "12".to_string();
            lic_exp.expires_at = Some(exp_past);
            lic_exp.expired = true;
            license_local::save_license(&lic_exp).unwrap();
            crate::entitlements::invalidate_cache();
            let b = super::super::get_header_license_badge(&palette, &app);
            assert_eq!(b.content, " [ PRO · Kedaluwarsa ] ");
            assert_eq!(b.style.fg, Some(Color::LightRed));
        });
    }

    #[test]
    fn test_factory_reset_modal_logic() {
        let home = TempTestHome::new();
        paths::with_test_home(&home.dir, || {
            // Write a dummy config file so resettable item exists
            let cfg_path = paths::config_path();
            if let Some(p) = cfg_path.parent() {
                std::fs::create_dir_all(p).unwrap();
            }
            std::fs::write(&cfg_path, "# test config").unwrap();

            let mut app = CockpitApp::new(&Config::default());
            assert!(!app.show_factory_reset_modal);

            // 1. Open modal
            open_factory_reset_modal(&mut app);
            assert!(app.show_factory_reset_modal);
            assert_eq!(app.factory_reset_btn_focus, 0); // [Batal] focused by default
            assert!(!app.factory_reset_items.is_empty());

            // 2. Tab toggles focus
            handle_factory_reset_key(make_key_event(KeyCode::Tab), &mut app);
            assert_eq!(app.factory_reset_btn_focus, 1);
            handle_factory_reset_key(make_key_event(KeyCode::Tab), &mut app);
            assert_eq!(app.factory_reset_btn_focus, 0);

            // 3. Confirm with wrong word ("reset" lowercase) -> denied
            app.factory_reset_input = "reset".to_string();
            confirm_factory_reset(&mut app);
            assert!(!app.factory_reset_requested);

            // 4. Confirm with exact "RESET" -> triggers reset
            app.factory_reset_input = "RESET".to_string();
            confirm_factory_reset(&mut app);
            assert!(app.factory_reset_requested);
            assert!(app.quit_requested);
            assert!(!app.show_factory_reset_modal);

            // 5. Esc closes modal
            app.show_factory_reset_modal = true;
            handle_factory_reset_key(make_key_event(KeyCode::Esc), &mut app);
            assert!(!app.show_factory_reset_modal);
        });
    }

    #[test]
    fn test_license_screen_actions() {
        let home = TempTestHome::new();
        paths::with_test_home(&home.dir, || {
            let mut app = CockpitApp::new(&Config::default());
            app.settings_subview = SettingsSubView::License;

            // Trigger activate with empty key
            app.license_key_input.clear();
            trigger_license_activate(&mut app);
            let notif = app
                .notification
                .as_ref()
                .map(|(msg, _)| msg.as_str())
                .unwrap();
            assert!(notif.contains("Ketik atau masukkan kunci"));

            // Trigger activate with invalid format
            app.license_key_input = "INVALID-FORMAT".to_string();
            trigger_license_activate(&mut app);
            let notif = app
                .notification
                .as_ref()
                .map(|(msg, _)| msg.as_str())
                .unwrap();
            assert!(notif.contains("Format kunci salah"));

            // Trigger verify with no local license
            let _ = license_local::remove_license();
            trigger_license_verify(&mut app);
            let notif = app
                .notification
                .as_ref()
                .map(|(msg, _)| msg.as_str())
                .unwrap();
            assert!(notif.contains("Tidak ada lisensi"));

            // Trigger deactivate with no local license
            trigger_license_deactivate(&mut app);
            let notif = app
                .notification
                .as_ref()
                .map(|(msg, _)| msg.as_str())
                .unwrap();
            assert!(notif.contains("Tidak ada lisensi"));
        });
    }
}
