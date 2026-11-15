//! Deterministic Telegram Command Router (T-03, T-04, T-07).
//!
//! Maps Telegram slash commands directly to strongly-typed [`Action`]s,
//! checks user role permissions (`Admin`, `Operator`, `Viewer`),
//! intercepts mutating actions for inline confirmation (T-04),
//! and manages multi-user allowlist with Pro gating (T-07).

use std::sync::{Arc, Mutex};

use super::auth::TelegramRole;
use super::client::{truncate_message, MAX_TELEGRAM_MESSAGE_LEN};
use super::types::{InlineKeyboardButton, InlineKeyboardMarkup, TelegramUser};
use crate::actions::{self, Action, ActionContext, DockerApi};
use crate::db::Database;

/// Output of command routing.
#[derive(Debug, Clone)]
pub enum RouterOutput {
    /// Plain text reply
    Text(String),
    /// Mutating (Write) action requiring inline button confirmation (T-04)
    RequireConfirmation { action: Action, description: String },
    /// Interactive menu with inline keyboard buttons
    Menu {
        text: String,
        keyboard: InlineKeyboardMarkup,
    },
}

/// Router for deterministic Telegram slash commands.
#[derive(Clone)]
pub struct TelegramRouter {
    pub docker: Arc<dyn DockerApi + Send + Sync>,
    pub db: Arc<Mutex<Database>>,
}

impl TelegramRouter {
    pub fn new(docker: Arc<dyn DockerApi + Send + Sync>, db: Arc<Mutex<Database>>) -> Self {
        Self { docker, db }
    }

    /// Process an incoming message text from an authorized Telegram user with a specific role.
    pub async fn handle_command(
        &self,
        text: &str,
        user: &TelegramUser,
        role: TelegramRole,
    ) -> RouterOutput {
        let trimmed = text.trim();
        if !trimmed.starts_with('/') {
            return RouterOutput::Text("Send /help to view available commands.".to_string());
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let cmd = parts[0];
        let base_cmd = cmd.split('@').next().unwrap_or(cmd);

        let ctx = ActionContext::new("telegram", user.id.to_string())
            .with_docker(self.docker.clone())
            .with_db(self.db.clone());

        match base_cmd {
            "/help" => RouterOutput::Text(Self::help_text(role)),
            "/start" => RouterOutput::Text(format!(
                "👋 Welcome to CAShell Telegram Bot!\nYou are authorized with role: *{}*.\nSend /help to view available commands.",
                role.as_str()
            )),
            "/menu" => RouterOutput::Menu {
                text: "⚡ *CAShell Quick Actions Menu*\nSelect an action below:".to_string(),
                keyboard: Self::quick_action_menu(),
            },
            "/status" => {
                Self::log_read_action(&ctx, "get_status");
                RouterOutput::Text(Self::run_action(Action::GetStatus, &ctx).await)
            }
            "/uptime" => {
                Self::log_read_action(&ctx, "get_uptime");
                RouterOutput::Text(Self::run_action(Action::GetUptime, &ctx).await)
            }
            "/logs" => {
                if parts.len() < 2 {
                    return RouterOutput::Text("Usage: /logs <app> [lines≤200]".to_string());
                }
                let app = parts[1];
                let lines = if parts.len() >= 3 {
                    parts[2].parse::<usize>().unwrap_or(50).min(200)
                } else {
                    50
                };
                Self::log_read_action(&ctx, "get_logs");
                RouterOutput::Text(
                    Self::run_action(
                        Action::GetLogs {
                            app: app.to_string(),
                            lines,
                        },
                        &ctx,
                    )
                    .await,
                )
            }
            "/containers" => {
                Self::log_read_action(&ctx, "list_containers");
                RouterOutput::Text(Self::run_action(Action::ListContainers, &ctx).await)
            }
            "/analytics" => {
                let site = parts.get(1).copied().unwrap_or_default().to_string();
                let days = parts.get(2).and_then(|d| d.parse::<u32>().ok()).unwrap_or(7);
                Self::log_read_action(&ctx, "get_analytics");
                RouterOutput::Text(Self::run_action(Action::GetAnalytics { site, days }, &ctx).await)
            }
            // ── Mutating (Write) actions with role check and inline confirmation ────
            "/restart" => {
                if !role.can_write() {
                    return RouterOutput::Text(
                        "⛔ Akses ditolak: Peran 'viewer' tidak diizinkan melakukan aksi modifikasi/tulis."
                            .to_string(),
                    );
                }
                if parts.len() < 2 {
                    return RouterOutput::Text("Usage: /restart <container>".to_string());
                }
                let name = parts[1];
                RouterOutput::RequireConfirmation {
                    action: Action::RestartContainer {
                        name: name.to_string(),
                    },
                    description: format!("Restart kontainer `{name}`"),
                }
            }
            "/deploy" => {
                if !role.can_write() {
                    return RouterOutput::Text(
                        "⛔ Akses ditolak: Peran 'viewer' tidak diizinkan melakukan aksi modifikasi/tulis."
                            .to_string(),
                    );
                }
                if parts.len() < 2 {
                    return RouterOutput::Text("Usage: /deploy <app>".to_string());
                }
                let app = parts[1];
                RouterOutput::RequireConfirmation {
                    action: Action::DeployApp {
                        app: app.to_string(),
                    },
                    description: format!("Deploy aplikasi `{app}`"),
                }
            }
            // ── Multi-user management with Pro gating (T-07) ────────────────────────
            "/users" => {
                if !role.can_manage_users() {
                    return RouterOutput::Text(
                        "⛔ Akses ditolak: Hanya pengguna dengan peran 'admin' yang dapat mengelola pengguna."
                            .to_string(),
                    );
                }
                self.handle_users_command(&parts[1..]).await
            }
            _ => RouterOutput::Text("Unknown command. Send /help for available commands.".to_string()),
        }
    }

    async fn handle_users_command(&self, args: &[&str]) -> RouterOutput {
        if args.is_empty() || args[0] == "list" {
            let db = self.db.lock().unwrap();
            let users = match db.list_telegram_users_with_role() {
                Ok(u) => u,
                Err(e) => return RouterOutput::Text(format!("✖ Failed querying users: {e}")),
            };
            let mut out = String::from("👥 *Daftar Pengguna Telegram:*\n");
            for (uid, uname, role, added_at) in users {
                out.push_str(&format!(
                    "• ID `{uid}` (@{}) | Peran: *{role}* | Terdaftar: {added_at}\n",
                    uname.as_deref().unwrap_or("none")
                ));
            }
            return RouterOutput::Text(out);
        }

        match args[0] {
            "add" => {
                if args.len() < 2 {
                    return RouterOutput::Text(
                        "Usage: /users add <user_id> [admin|operator|viewer]".to_string(),
                    );
                }
                let target_id = match args[1].parse::<i64>() {
                    Ok(id) => id,
                    Err(_) => {
                        return RouterOutput::Text(
                            "✖ User ID harus berupa angka numerik.".to_string(),
                        )
                    }
                };
                let role_str = args.get(2).copied().unwrap_or("operator");
                let role = TelegramRole::parse_role(role_str);

                // Pro gating: Free tier restricted to 1 user max!
                let is_pro = crate::entitlements::current().is_pro;
                let user_count = {
                    let db = self.db.lock().unwrap();
                    db.count_telegram_users().unwrap_or(0)
                };

                if !is_pro && user_count >= 1 {
                    return RouterOutput::Text(
                        "🔒 *Fitur CAShell Pro*\nFree tier dibatasi maksimal 1 pengguna admin.\nUpgrade ke Pro untuk menambah multi-pengguna dan pembagian peran (admin/operator/viewer)."
                            .to_string(),
                    );
                }

                let db = self.db.lock().unwrap();
                match db.add_telegram_user_with_role(target_id, None, role.as_str()) {
                    Ok(_) => RouterOutput::Text(format!(
                        "✔ Pengguna `{target_id}` berhasil ditambahkan dengan peran *{}*.",
                        role.as_str()
                    )),
                    Err(e) => RouterOutput::Text(format!("✖ Gagal menambahkan pengguna: {e}")),
                }
            }
            "remove" => {
                if args.len() < 2 {
                    return RouterOutput::Text("Usage: /users remove <user_id>".to_string());
                }
                let target_id = match args[1].parse::<i64>() {
                    Ok(id) => id,
                    Err(_) => {
                        return RouterOutput::Text(
                            "✖ User ID harus berupa angka numerik.".to_string(),
                        )
                    }
                };

                let db = self.db.lock().unwrap();
                match db.remove_telegram_user(target_id) {
                    Ok(true) => RouterOutput::Text(format!(
                        "✔ Pengguna `{target_id}` telah dihapus dari allowlist."
                    )),
                    Ok(false) => RouterOutput::Text(format!(
                        "ℹ Pengguna `{target_id}` tidak ditemukan dalam allowlist."
                    )),
                    Err(e) => RouterOutput::Text(format!("✖ Gagal menghapus pengguna: {e}")),
                }
            }
            "role" => {
                if args.len() < 3 {
                    return RouterOutput::Text(
                        "Usage: /users role <user_id> <admin|operator|viewer>".to_string(),
                    );
                }
                let target_id = match args[1].parse::<i64>() {
                    Ok(id) => id,
                    Err(_) => {
                        return RouterOutput::Text(
                            "✖ User ID harus berupa angka numerik.".to_string(),
                        )
                    }
                };
                let new_role = TelegramRole::parse_role(args[2]);

                let db = self.db.lock().unwrap();
                match db.set_telegram_user_role(target_id, new_role.as_str()) {
                    Ok(true) => RouterOutput::Text(format!(
                        "✔ Peran pengguna `{target_id}` diubah menjadi *{}*.",
                        new_role.as_str()
                    )),
                    Ok(false) => RouterOutput::Text(format!(
                        "ℹ Pengguna `{target_id}` tidak ditemukan dalam allowlist."
                    )),
                    Err(e) => RouterOutput::Text(format!("✖ Gagal mengubah peran: {e}")),
                }
            }
            _ => RouterOutput::Text(
                "Usage: /users <list | add <id> [role] | remove <id> | role <id> <role>>"
                    .to_string(),
            ),
        }
    }

    pub async fn run_action(action: Action, ctx: &ActionContext) -> String {
        let name = action.action_name();
        match actions::execute(action, ctx).await {
            Ok(res) => {
                let badge = if res.ok { "✔" } else { "✖" };
                let body = format!("{badge} [{name}]\n{}", res.output.trim());
                truncate_message(&body, MAX_TELEGRAM_MESSAGE_LEN)
            }
            Err(e) => {
                let body = format!("✖ [{name}] Error: {e}");
                truncate_message(&body, MAX_TELEGRAM_MESSAGE_LEN)
            }
        }
    }

    fn log_read_action(ctx: &ActionContext, action_name: &str) {
        if let Some(ref db_lock) = ctx.db {
            if let Ok(db) = db_lock.lock() {
                let _ = db.log_audit("telegram", &ctx.actor, action_name, None, Some("ok"), None);
            }
        }
    }

    pub fn quick_action_menu() -> InlineKeyboardMarkup {
        InlineKeyboardMarkup::new(vec![
            vec![
                InlineKeyboardButton::callback("🖥 Status", "cmd:/status"),
                InlineKeyboardButton::callback("📈 Uptime", "cmd:/uptime"),
            ],
            vec![
                InlineKeyboardButton::callback("📦 Kontainer", "cmd:/containers"),
                InlineKeyboardButton::callback("📊 Analitik", "cmd:/analytics"),
            ],
            vec![
                InlineKeyboardButton::callback("📜 Logs Web", "cmd:/logs web 50"),
                InlineKeyboardButton::callback("👥 Pengguna", "cmd:/users list"),
            ],
        ])
    }

    pub fn help_text(role: TelegramRole) -> String {
        let mut help = String::from(
            "CAShell Telegram Commands:\n/status - System and daemon status\n/uptime - Endpoint & fleet uptime summary\n/logs <app> [n] - View container logs (n ≤ 200)\n/containers - List docker containers\n/analytics [site] [days] - Sovereign analytics summary\n/menu - Show interactive quick action buttons\n",
        );

        if role.can_write() {
            help.push_str(
                "/restart <container> - Restart a target container (requires confirmation)\n/deploy <app> - Deploy an application (requires confirmation)\n",
            );
        }

        if role.can_manage_users() {
            help.push_str(
                "/users list - List authorized users\n/users add <id> [role] - Add authorized user (Pro)\n/users remove <id> - Revoke user authorization\n/users role <id> <role> - Update user role\n",
            );
        }

        help.push_str("/help - Show this help menu");
        help
    }
}
