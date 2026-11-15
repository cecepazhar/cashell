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
        let ctx = ActionContext::new("telegram", user.id.to_string())
            .with_docker(self.docker.clone())
            .with_db(self.db.clone());

        if !trimmed.starts_with('/') {
            return RouterOutput::Text("Send /help to view available commands.".to_string());
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let cmd = parts[0];
        let base_cmd = cmd.split('@').next().unwrap_or(cmd);

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
            "/ai" => {
                let prompt = parts[1..].join(" ");
                if prompt.trim().is_empty() {
                    return RouterOutput::Text("Usage: /ai <pertanyaan atau perintah>".to_string());
                }
                self.handle_ai_query(&prompt, user, role, &ctx).await
            }
            _ => RouterOutput::Text("Unknown command. Send /help for available commands.".to_string()),
        }
    }

    pub async fn handle_ai_query(
        &self,
        prompt: &str,
        user: &TelegramUser,
        role: TelegramRole,
        ctx: &ActionContext,
    ) -> RouterOutput {
        let session_id = format!("tg_{}", user.id);

        // 1. Load recent conversation history (sliding window 10, AI-06)
        let history = {
            if let Ok(db) = self.db.lock() {
                db.load_recent_chat_session(&session_id, 10)
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        };

        // 2. Prepare tools
        let tools = crate::ai::system_tool_definitions();

        // 3. Prepare system prompt & contextual prompt
        let sys_prompt = format!(
            "You are Hana, the autonomous DevOps AI assistant for CAShell. \
             You are interacting with Telegram user {} (role: {}). \
             You have access to tools for querying system stats, logs, containers, and managing services. \
             When the user asks to inspect or query, invoke the corresponding read tool. \
             When the user asks to restart or modify, invoke the corresponding mutating tool.",
            user.first_name, role.as_str()
        );

        let mut context_prompt = String::new();
        if !history.is_empty() {
            context_prompt.push_str("Recent conversation history:\n");
            for (r, msg) in &history {
                context_prompt.push_str(&format!("{r}: {msg}\n"));
            }
            context_prompt.push_str("\nCurrent user prompt:\n");
        }
        context_prompt.push_str(prompt);

        // 4. Query AI using blocking call wrapped in spawn_blocking
        let cfg = crate::config::Config::load(None).unwrap_or_default();
        let prompt_clone = context_prompt.clone();
        let sys_clone = sys_prompt.clone();
        let tools_clone = tools.clone();

        let ai_result = tokio::task::spawn_blocking(move || {
            let client = reqwest::blocking::Client::builder().build()?;
            crate::ai::query_with_tools(
                &client,
                &cfg,
                Some(&sys_clone),
                &prompt_clone,
                &tools_clone,
            )
        })
        .await;

        let response = match ai_result {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                let err_str = e.to_string();
                if err_str.contains("no AI provider configured") {
                    return RouterOutput::Text(
                        "🤖 [Hana AI] AI belum dikonfigurasi. Jalankan `cashell ai setup` di server untuk mengaktifkan AI BYOK.".to_string()
                    );
                }
                return RouterOutput::Text(format!("🤖 [Hana AI] Gagal memproses: {err_str}"));
            }
            Err(join_err) => {
                return RouterOutput::Text(format!("🤖 [Hana AI] Runtime error: {join_err}"));
            }
        };

        // Save user message to SQLite sliding window
        if let Ok(db) = self.db.lock() {
            let _ = db.append_chat_and_trim(&session_id, "user", prompt, 10);
        }

        match response {
            crate::ai::AiResponse::Text(answer) => {
                // Save assistant reply
                if let Ok(db) = self.db.lock() {
                    let _ = db.append_chat_and_trim(&session_id, "assistant", &answer, 10);
                }
                RouterOutput::Text(format!("🤖 [Hana AI]\n{answer}"))
            }
            crate::ai::AiResponse::ToolCalls(calls) => {
                if calls.is_empty() {
                    return RouterOutput::Text(
                        "🤖 [Hana AI] Tidak ada tindakan yang dihasilkan.".to_string(),
                    );
                }

                // AI-03: Process tool calls.
                // If the tool is a Read action: auto-execute immediately.
                // If the tool is a Write action: proposal only + require confirmation (T-04)!
                let call = &calls[0];
                match crate::ai::tool_call_to_action(call) {
                    Ok(action) => {
                        match action.kind() {
                            crate::actions::ActionKind::Read => {
                                // Auto-execute read tool
                                Self::log_read_action(ctx, action.action_name());
                                let output = Self::run_action(action, ctx).await;
                                let reply = format!(
                                    "🤖 [Hana AI] Auto-eksekusi tool `{}`:\n\n{output}",
                                    call.name
                                );
                                if let Ok(db) = self.db.lock() {
                                    let _ = db.append_chat_and_trim(
                                        &session_id,
                                        "assistant",
                                        &reply,
                                        10,
                                    );
                                }
                                RouterOutput::Text(reply)
                            }
                            crate::actions::ActionKind::Write => {
                                // Mutating write action: check role and REQUIRE CONFIRMATION
                                if !role.can_write() {
                                    return RouterOutput::Text(
                                        "⛔ [Hana AI] Peran 'viewer' tidak diizinkan mengeksekusi aksi modifikasi.".to_string()
                                    );
                                }
                                let desc = format!(
                                    "AI mengusulkan: `{}` dengan parameter `{}`",
                                    call.name, call.arguments
                                );
                                if let Ok(db) = self.db.lock() {
                                    let _ = db.append_chat_and_trim(
                                        &session_id,
                                        "assistant",
                                        &desc,
                                        10,
                                    );
                                }
                                RouterOutput::RequireConfirmation {
                                    action,
                                    description: desc,
                                }
                            }
                        }
                    }
                    Err(err) => RouterOutput::Text(format!(
                        "🤖 [Hana AI] Gagal memetakan tool `{}`: {err}",
                        call.name
                    )),
                }
            }
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
