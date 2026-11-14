//! Telegram bot management CLI & setup wizard (T-01, T-02, T-03).

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::io::{self, Write};
use std::time::Duration;

use crate::config::Config;
use crate::db::get_db;
use crate::secrets::SecretStore;
use crate::telegram::client::TelegramClient;

/// Telegram bot CLI command.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "telegram",
    about = "Telegram bot management & setup wizard (T-01, T-02)"
)]
pub struct TelegramCommand {
    #[command(subcommand)]
    pub action: TelegramAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum TelegramAction {
    /// Interactive or scripted wizard to pair Telegram bot and authorize admin user (T-02)
    Setup(TelegramSetupArgs),

    /// Show Telegram integration status (bot info, authorized users, offset)
    Status,

    /// List authorized Telegram user IDs
    List,

    /// Authorize a Telegram user ID manually
    Allow {
        /// Telegram numeric user ID
        user_id: i64,
        /// Optional Telegram username
        #[arg(long)]
        username: Option<String>,
    },

    /// Revoke authorization for a Telegram user ID
    Revoke {
        /// Telegram numeric user ID
        user_id: i64,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct TelegramSetupArgs {
    /// Bot API token (prompted interactively if omitted)
    #[arg(long)]
    pub token: Option<String>,

    /// Pre-authorize a specific user ID directly
    #[arg(long)]
    pub user_id: Option<i64>,

    /// Optional username for pre-authorized user
    #[arg(long)]
    pub username: Option<String>,

    /// Automatically confirm without interactive prompt
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Base URL override for testing / proxy
    #[arg(long)]
    pub base_url: Option<String>,
}

pub fn run(cmd: &TelegramCommand, cfg: &mut Config) -> Result<i32> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    match &cmd.action {
        TelegramAction::Setup(args) => rt.block_on(run_setup(args, cfg)),
        TelegramAction::Status => rt.block_on(run_status(cfg)),
        TelegramAction::List => run_list(),
        TelegramAction::Allow { user_id, username } => run_allow(*user_id, username.as_deref()),
        TelegramAction::Revoke { user_id } => run_revoke(*user_id),
    }
}

async fn run_setup(args: &TelegramSetupArgs, cfg: &mut Config) -> Result<i32> {
    println!("{}", "=======================================".cyan());
    println!(
        "{}",
        "       CAShell Telegram Setup (T-02)   ".bold().cyan()
    );
    println!("{}", "=======================================".cyan());

    // 1. Determine Bot Token
    let token = if let Some(ref t) = args.token {
        t.trim().to_string()
    } else {
        let existing_token = SecretStore::load_default()
            .ok()
            .and_then(|s| s.telegram_bot_token)
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                if !cfg.daemon.telegram_bot_token.trim().is_empty() {
                    Some(cfg.daemon.telegram_bot_token.clone())
                } else {
                    None
                }
            });

        if let Some(existing) = existing_token {
            let masked = if existing.len() > 6 {
                format!("{}...", &existing[..6])
            } else {
                "******".to_string()
            };
            print!("Found existing bot token [{masked}]. Use it? [Y/n]: ");
            io::stdout().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            if answer.trim().eq_ignore_ascii_case("n") {
                prompt_bot_token()?
            } else {
                existing
            }
        } else {
            prompt_bot_token()?
        }
    };

    if token.is_empty() {
        bail!("Bot token cannot be empty");
    }

    // 2. Initialize Client and Verify Bot
    let base_url = args
        .base_url
        .as_deref()
        .unwrap_or("https://api.telegram.org");
    let client = TelegramClient::with_base_url(&token, base_url);

    println!("\nConnecting to Telegram Bot API...");
    let bot_user = match client.get_me().await {
        Ok(u) => {
            let bot_handle = u.username.as_deref().unwrap_or("<no-username>");
            println!(
                "{} Bot verified: {} (@{})",
                "✔".green().bold(),
                u.first_name.bold(),
                bot_handle.cyan()
            );
            u
        }
        Err(e) => {
            eprintln!("{} Failed to verify bot token: {e}", "✖".red());
            bail!("Invalid bot token or network error");
        }
    };

    let bot_username = bot_user.username.unwrap_or_else(|| "YourBot".to_string());

    // 3. User Pairing / Allowlist
    let (target_user_id, target_username) = if let Some(uid) = args.user_id {
        (uid, args.username.clone())
    } else {
        println!("\n{}", "👉 Pairing Instructions:".bold().yellow());
        println!("  1. Open Telegram on your phone or desktop.");
        println!("  2. Search for @{}", bot_username.cyan().bold());
        println!("  3. Send {}", "/start".green().bold());
        println!("\nWaiting for incoming /start message (timeout 60s, Ctrl+C to cancel)...");

        let detected = poll_for_start(&client, 60).await?;
        println!(
            "{} Detected incoming user: ID {} (@{})",
            "✔".green().bold(),
            detected.id.to_string().cyan().bold(),
            detected.username.as_deref().unwrap_or("none")
        );
        (detected.id, detected.username)
    };

    // 4. Confirmation
    if !args.yes {
        print!(
            "Authorize user ID {} (@{}) for CAShell admin? [Y/n]: ",
            target_user_id,
            target_username.as_deref().unwrap_or("none")
        );
        io::stdout().flush()?;
        let mut confirm = String::new();
        io::stdin().read_line(&mut confirm)?;
        if confirm.trim().eq_ignore_ascii_case("n") {
            println!("{}", "Setup aborted by user.".yellow());
            return Ok(2);
        }
    }

    // 5. Persist Bot Token and Allowlist
    let mut store = SecretStore::load_default().unwrap_or_default();
    store.telegram_bot_token = Some(token.clone());
    store
        .save_default()
        .context("Failed saving bot token to encrypted secrets store")?;

    cfg.daemon.telegram_bot_token = token.clone();
    cfg.uptime.telegram_bot_token = token.clone();
    cfg.daemon.telegram_chat_id = target_user_id.to_string();
    cfg.uptime.telegram_chat_id = target_user_id.to_string();
    let _ = cfg.save(&crate::paths::config_path());

    let db = get_db()?;
    db.add_telegram_user(target_user_id, target_username.as_deref())?;

    // 6. Send pairing notification to Telegram
    let welcome = "✅ *CAShell Paired Successfully*\n\nYou are now authorized as an administrator.\nSend `/help` to view available commands.";
    if let Err(e) = client.send_message(target_user_id, welcome).await {
        eprintln!("⚠ Note: Could not send welcome message to chat: {e}");
    }

    println!("\n{}", "✔ Setup Complete!".green().bold());
    println!("  • Bot token securely stored in encrypted secrets (~/.config/cashell/secrets.enc)");
    println!(
        "  • User ID {} added to allowlist",
        target_user_id.to_string().cyan()
    );
    println!(
        "  • Daemon will poll and respond to authorized commands via `/status`, `/uptime`, etc."
    );

    Ok(0)
}

fn prompt_bot_token() -> Result<String> {
    print!("Enter Telegram Bot Token (from @BotFather): ");
    io::stdout().flush()?;
    let mut token = String::new();
    io::stdin().read_line(&mut token)?;
    Ok(token.trim().to_string())
}

async fn poll_for_start(
    client: &TelegramClient,
    timeout_secs: u64,
) -> Result<crate::telegram::TelegramUser> {
    let start_time = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let mut offset: i64 = 0;

    while start_time.elapsed() < timeout {
        match client.get_updates(offset, 5).await {
            Ok(updates) => {
                for u in updates {
                    offset = offset.max(u.update_id + 1);
                    if let Some(msg) = u.message {
                        if let Some(text) = msg.text {
                            if text.trim().starts_with("/start") {
                                if let Some(from) = msg.from {
                                    return Ok(from);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠ getUpdates error: {e}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    bail!("Timed out waiting for incoming /start message");
}

async fn run_status(cfg: &Config) -> Result<i32> {
    let token = SecretStore::load_default()
        .ok()
        .and_then(|s| s.telegram_bot_token)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| cfg.daemon.telegram_bot_token.clone());

    println!("{}", "CAShell Telegram Integration Status:".bold());
    if token.is_empty() {
        println!("  • Bot Token: {}", "Not configured".yellow());
        println!("    Run 'cashell telegram setup' to configure.");
    } else {
        let masked = if token.len() > 6 {
            format!("{}...", &token[..6])
        } else {
            "******".to_string()
        };
        println!("  • Bot Token: {} (encrypted at rest)", masked.green());

        let client = TelegramClient::new(&token);
        match client.get_me().await {
            Ok(me) => {
                println!(
                    "  • Bot API Status: {} (@{})",
                    "Connected".green(),
                    me.username.as_deref().unwrap_or("none")
                );
            }
            Err(e) => {
                println!("  • Bot API Status: {} ({e})", "Connection Error".red());
            }
        }
    }

    let db = get_db()?;
    let users = db.list_telegram_users()?;
    let offset = db.get_telegram_offset()?;
    println!("  • Polling Offset: {}", offset);
    println!("  • Authorized Users: {}", users.len());
    for (uid, uname, added_at) in users {
        println!(
            "    - ID: {}, Username: @{}, Added: {}",
            uid,
            uname.as_deref().unwrap_or("none"),
            added_at
        );
    }

    Ok(0)
}

fn run_list() -> Result<i32> {
    let db = get_db()?;
    let users = db.list_telegram_users()?;
    println!("{}", "Authorized Telegram Users:".bold());
    if users.is_empty() {
        println!("  (No users in allowlist. Run 'cashell telegram setup' to add admin)");
    } else {
        for (uid, uname, added_at) in users {
            println!(
                "  • ID: {:<12} Username: @{:<16} Added: {}",
                uid,
                uname.as_deref().unwrap_or("none"),
                added_at
            );
        }
    }
    Ok(0)
}

fn run_allow(user_id: i64, username: Option<&str>) -> Result<i32> {
    let db = get_db()?;
    db.add_telegram_user(user_id, username)?;
    println!(
        "{} User {} added to Telegram allowlist.",
        "✔".green().bold(),
        user_id
    );
    Ok(0)
}

fn run_revoke(user_id: i64) -> Result<i32> {
    let db = get_db()?;
    if db.remove_telegram_user(user_id)? {
        println!(
            "{} User {} removed from Telegram allowlist.",
            "✔".green().bold(),
            user_id
        );
    } else {
        println!("{} User {} was not in allowlist.", "ℹ".yellow(), user_id);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Json, Path};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::Router;
    use serde_json::Value;

    #[test]
    fn test_cli_telegram_allow_revoke_list() {
        let tmp = std::env::temp_dir().join(format!("cashell-tg-test-1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        crate::paths::with_test_home(&tmp, || {
            assert_eq!(run_allow(8888, Some("botadmin")).unwrap(), 0);
            let db = get_db().unwrap();
            assert!(db.is_telegram_user_allowed(8888).unwrap());
            assert_eq!(run_list().unwrap(), 0);
            assert_eq!(run_revoke(8888).unwrap(), 0);
            assert!(!db.is_telegram_user_allowed(8888).unwrap());
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cli_telegram_setup_scripted_flow() {
        let tmp = std::env::temp_dir().join(format!("cashell-tg-test-2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let app = Router::new()
            .route(
                "/bot{token}/getMe",
                get(|Path(_token): Path<String>| async move {
                    Json(serde_json::json!({
                        "ok": true,
                        "result": {
                            "id": 99999,
                            "is_bot": true,
                            "first_name": "TestBot",
                            "username": "CashellTestBot"
                        }
                    }))
                    .into_response()
                }),
            )
            .route(
                "/bot{token}/sendMessage",
                post(
                    |Path(_token): Path<String>, Json(_payload): Json<Value>| async move {
                        Json(serde_json::json!({ "ok": true, "result": { "message_id": 1 } }))
                            .into_response()
                    },
                ),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        crate::paths::with_test_home(&tmp, || {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let mut cfg = Config::default();
                    let args = TelegramSetupArgs {
                        token: Some("bot12345:TOKEN".into()),
                        user_id: Some(7777),
                        username: Some("testuser".into()),
                        yes: true,
                        base_url: Some(format!("http://{addr}")),
                    };

                    let res = run_setup(&args, &mut cfg).await.unwrap();
                    assert_eq!(res, 0);

                    // Verify DB allowlist was populated
                    let db = get_db().unwrap();
                    assert!(db.is_telegram_user_allowed(7777).unwrap());

                    // Verify secret store has the bot token encrypted
                    let store = SecretStore::load_default().unwrap();
                    assert_eq!(store.telegram_bot_token.as_deref(), Some("bot12345:TOKEN"));
                });
            });
        });
    }
}
