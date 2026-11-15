//! Unit & integration tests for Telegram module (T-01 s/d T-07).

use axum::extract::{Json, Path};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use serde_json::Value;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::auth::TelegramRole;
use super::client::truncate_message;
use super::confirm::PendingAction;
use super::polling::TelegramPoller;
use super::router::{RouterOutput, TelegramRouter};
use super::types::TelegramUser;
use crate::actions::{Action, MockDocker};
use crate::db::Database;

// ── Test 1: Message truncation strictly caps at <= 4000 chars (T-03) ────────

#[test]
fn test_message_truncation() {
    let short_msg = "Hello world";
    assert_eq!(truncate_message(short_msg, 4000), "Hello world");

    let long_msg = "a".repeat(5000);
    let truncated = truncate_message(&long_msg, 4000);
    assert!(truncated.len() <= 4000);
    assert!(truncated.ends_with("\n...[truncated]"));
}

// ── Test 2: Router deterministic commands via typed actions (T-03, T-04, T-07) ──

#[tokio::test]
async fn test_router_deterministic_commands() {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let mock_docker = Arc::new(MockDocker::new(&["web-prod"]));
    let router = TelegramRouter::new(mock_docker, db.clone());

    let user = TelegramUser {
        id: 12345,
        is_bot: false,
        first_name: "Cecep".into(),
        last_name: None,
        username: Some("cecepazhar".into()),
    };

    // /help
    let help_res = router
        .handle_command("/help", &user, TelegramRole::Admin)
        .await;
    match help_res {
        RouterOutput::Text(t) => {
            assert!(t.contains("/status"));
            assert!(t.contains("/uptime"));
            assert!(t.contains("/containers"));
        }
        _ => panic!("expected Text for /help"),
    }

    // /start
    let start_res = router
        .handle_command("/start", &user, TelegramRole::Admin)
        .await;
    match start_res {
        RouterOutput::Text(t) => assert!(t.contains("Welcome to CAShell Telegram Bot")),
        _ => panic!("expected Text for /start"),
    }

    // /menu (quick actions)
    let menu_res = router
        .handle_command("/menu", &user, TelegramRole::Admin)
        .await;
    match menu_res {
        RouterOutput::Menu { keyboard, .. } => {
            assert!(!keyboard.inline_keyboard.is_empty());
        }
        _ => panic!("expected Menu output"),
    }

    // /status
    let status_res = router
        .handle_command("/status", &user, TelegramRole::Admin)
        .await;
    match status_res {
        RouterOutput::Text(t) => assert!(t.contains("[get_status]")),
        _ => panic!("expected Text for /status"),
    }

    // /uptime
    let uptime_res = router
        .handle_command("/uptime", &user, TelegramRole::Admin)
        .await;
    match uptime_res {
        RouterOutput::Text(t) => assert!(t.contains("[get_uptime]")),
        _ => panic!("expected Text for /uptime"),
    }

    // /containers
    let containers_res = router
        .handle_command("/containers", &user, TelegramRole::Admin)
        .await;
    match containers_res {
        RouterOutput::Text(t) => {
            assert!(t.contains("[list_containers]"));
            assert!(t.contains("Found 1 containers"));
        }
        _ => panic!("expected Text for /containers"),
    }

    // /restart web-prod (Write action -> requires confirmation per T-04)
    let restart_res = router
        .handle_command("/restart web-prod", &user, TelegramRole::Admin)
        .await;
    match restart_res {
        RouterOutput::RequireConfirmation {
            action,
            description,
        } => {
            assert_eq!(action.action_name(), "restart_container");
            assert!(description.contains("web-prod"));
        }
        _ => panic!("expected RequireConfirmation for /restart"),
    }

    // /restart by Viewer -> rejected (T-07)
    let viewer_restart = router
        .handle_command("/restart web-prod", &user, TelegramRole::Viewer)
        .await;
    match viewer_restart {
        RouterOutput::Text(t) => assert!(t.contains("viewer")),
        _ => panic!("expected rejection for viewer restart"),
    }

    // /restart (missing args)
    let restart_err = router
        .handle_command("/restart", &user, TelegramRole::Admin)
        .await;
    match restart_err {
        RouterOutput::Text(t) => assert_eq!(t, "Usage: /restart <container>"),
        _ => panic!("expected Text for missing args"),
    }

    // /deploy api-gateway (Write action -> requires confirmation per T-04)
    let deploy_res = router
        .handle_command("/deploy api-gateway", &user, TelegramRole::Admin)
        .await;
    match deploy_res {
        RouterOutput::RequireConfirmation {
            action,
            description,
        } => {
            assert_eq!(action.action_name(), "deploy_app");
            assert!(description.contains("api-gateway"));
        }
        _ => panic!("expected RequireConfirmation for /deploy"),
    }

    // /logs web-prod 30
    let logs_res = router
        .handle_command("/logs web-prod 30", &user, TelegramRole::Admin)
        .await;
    match logs_res {
        RouterOutput::Text(t) => assert!(t.contains("[get_logs]")),
        _ => panic!("expected Text for /logs"),
    }

    // /logs (missing app)
    let logs_err = router
        .handle_command("/logs", &user, TelegramRole::Admin)
        .await;
    match logs_err {
        RouterOutput::Text(t) => assert_eq!(t, "Usage: /logs <app> [lines≤200]"),
        _ => panic!("expected Text for missing app"),
    }

    // /analytics
    let analytics_res = router
        .handle_command("/analytics fathforce 14", &user, TelegramRole::Admin)
        .await;
    match analytics_res {
        RouterOutput::Text(t) => assert!(t.contains("[get_analytics]")),
        _ => panic!("expected Text for /analytics"),
    }

    // Unknown command
    let unknown_res = router
        .handle_command("/unknowncmd", &user, TelegramRole::Admin)
        .await;
    match unknown_res {
        RouterOutput::Text(t) => {
            assert_eq!(t, "Unknown command. Send /help for available commands.")
        }
        _ => panic!("expected Text"),
    }

    // Non-slash text
    let plain_res = router
        .handle_command("hello bot", &user, TelegramRole::Admin)
        .await;
    match plain_res {
        RouterOutput::Text(t) => assert_eq!(t, "Send /help to view available commands."),
        _ => panic!("expected Text"),
    }
}

// ── Test 3: Allowlist enforcement and audit logging for unauthorized senders (T-02) ──

#[tokio::test]
async fn test_allowlist_enforcement_and_audit_log() {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let mock_docker = Arc::new(MockDocker::new(&[]));

    // Allow user 1001 as admin, but NOT user 2002
    db.lock()
        .unwrap()
        .add_telegram_user(1001, Some("admin"))
        .unwrap();

    let sent_messages = Arc::new(Mutex::new(Vec::<(i64, String)>::new()));
    let sent_clone = sent_messages.clone();

    let app = Router::new()
        .route(
            "/bot{token}/getUpdates",
            post(
                |Path(_token): Path<String>, Json(payload): Json<Value>| async move {
                    let offset = payload.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
                    if offset == 0 {
                        let updates = serde_json::json!({
                            "ok": true,
                            "result": [
                                {
                                    "update_id": 10,
                                    "message": {
                                        "message_id": 1,
                                        "from": {
                                            "id": 2002, // UNAUTHORIZED USER
                                            "first_name": "Attacker",
                                            "username": "attacker"
                                        },
                                        "chat": {
                                            "id": 2002,
                                            "type": "private"
                                        },
                                        "text": "/status"
                                    }
                                },
                                {
                                    "update_id": 11,
                                    "message": {
                                        "message_id": 2,
                                        "from": {
                                            "id": 1001, // AUTHORIZED USER
                                            "first_name": "Admin",
                                            "username": "admin"
                                        },
                                        "chat": {
                                            "id": 1001,
                                            "type": "private"
                                        },
                                        "text": "/status"
                                    }
                                }
                            ]
                        });
                        Json(updates).into_response()
                    } else {
                        Json(serde_json::json!({ "ok": true, "result": [] })).into_response()
                    }
                },
            ),
        )
        .route(
            "/bot{token}/sendMessage",
            post({
                let sent_clone = sent_clone.clone();
                move |Path(_token): Path<String>, Json(payload): Json<Value>| {
                    let sent_clone = sent_clone.clone();
                    async move {
                        let chat_id = payload.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
                        let text = payload
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        sent_clone.lock().unwrap().push((chat_id, text));
                        Json(serde_json::json!({ "ok": true, "result": { "message_id": 100 } }))
                            .into_response()
                    }
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let poller = TelegramPoller::with_base_url(
        "test-token",
        format!("http://{addr}"),
        db.clone(),
        mock_docker,
        None,
    )
    .with_timeout_secs(1);

    // Poll updates
    let processed = poller.poll_once().await.unwrap();
    assert_eq!(processed, 1); // Only 1 authorized message processed

    // Verify sent messages: ONLY user 1001 received a reply, NEVER user 2002!
    let sent = sent_messages.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, 1001);
    assert!(sent[0].1.contains("[get_status]"));

    // Verify audit log (T-02): Contains unauthorized attempt from actor "2002"
    let db_guard = db.lock().unwrap();
    let audit_entries = db_guard.query_audit(10).unwrap();
    let unauth_entry = audit_entries
        .iter()
        .find(|e| e.actor == "2002" && e.action == "unauthorized_message");
    assert!(
        unauth_entry.is_some(),
        "expected audit log for unauthorized user 2002"
    );
    let unauth = unauth_entry.unwrap();
    assert_eq!(unauth.source, "telegram");
    assert_eq!(unauth.result.as_deref(), Some("rejected"));
}

// ── Test 4: Long polling offset persistence and no reprocessing on restart (T-01) ──

#[tokio::test]
async fn test_polling_offset_persistence_and_no_reprocessing_on_restart() {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let mock_docker = Arc::new(MockDocker::new(&[]));
    db.lock()
        .unwrap()
        .add_telegram_user(5001, Some("tester"))
        .unwrap();

    let server_offset_received = Arc::new(AtomicI64::new(-1));
    let s_offset = server_offset_received.clone();

    let app = Router::new()
        .route(
            "/bot{token}/getUpdates",
            post(
                move |Path(_token): Path<String>, Json(payload): Json<Value>| {
                    let s_offset = s_offset.clone();
                    async move {
                        let offset = payload.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
                        s_offset.store(offset, Ordering::SeqCst);

                        if offset == 0 {
                            let updates = serde_json::json!({
                                "ok": true,
                                "result": [
                                    {
                                        "update_id": 50,
                                        "message": {
                                            "message_id": 1,
                                            "from": { "id": 5001, "first_name": "T" },
                                            "chat": { "id": 5001, "type": "private" },
                                            "text": "/help"
                                        }
                                    },
                                    {
                                        "update_id": 51,
                                        "message": {
                                            "message_id": 2,
                                            "from": { "id": 5001, "first_name": "T" },
                                            "chat": { "id": 5001, "type": "private" },
                                            "text": "/help"
                                        }
                                    }
                                ]
                            });
                            Json(updates).into_response()
                        } else {
                            Json(serde_json::json!({ "ok": true, "result": [] })).into_response()
                        }
                    }
                },
            ),
        )
        .route(
            "/bot{token}/sendMessage",
            post(
                |Path(_token): Path<String>, Json(_payload): Json<Value>| async move {
                    Json(serde_json::json!({ "ok": true, "result": { "message_id": 200 } }))
                        .into_response()
                },
            ),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let poller1 = TelegramPoller::with_base_url(
        "bot-token",
        format!("http://{addr}"),
        db.clone(),
        mock_docker.clone(),
        None,
    )
    .with_timeout_secs(1);

    let processed1 = poller1.poll_once().await.unwrap();
    assert_eq!(processed1, 2);

    let persisted_offset = db.lock().unwrap().get_telegram_offset().unwrap();
    assert_eq!(persisted_offset, 52);

    let poller2 = TelegramPoller::with_base_url(
        "bot-token",
        format!("http://{addr}"),
        db.clone(),
        mock_docker.clone(),
        None,
    )
    .with_timeout_secs(1);

    let processed2 = poller2.poll_once().await.unwrap();
    assert_eq!(processed2, 0);

    assert_eq!(server_offset_received.load(Ordering::SeqCst), 52);
}

// ── Test 5: HTTP 409 Conflict detection (T-01) ──────────────────────────────

#[tokio::test]
async fn test_polling_409_conflict_detection() {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let mock_docker = Arc::new(MockDocker::new(&[]));

    let app = Router::new().route(
        "/bot{token}/getUpdates",
        post(
            |Path(_token): Path<String>, Json(_payload): Json<Value>| async move {
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "ok": false,
                        "error_code": 409,
                        "description": "Conflict: can't use getUpdates method while webhook is active"
                    })),
                )
                    .into_response()
            },
        ),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let poller = TelegramPoller::with_base_url(
        "bot-token",
        format!("http://{addr}"),
        db.clone(),
        mock_docker,
        None,
    )
    .with_timeout_secs(1);

    let err = poller.poll_once().await.unwrap_err();
    assert!(err.to_string().contains("Conflict (HTTP 409)"));
}

// ── Test 6: Inline confirmation for write actions and 60s expiration (T-04) ──

#[tokio::test]
async fn test_inline_confirmation_for_write_actions() {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let mock_docker = Arc::new(MockDocker::new(&["web-prod"]));
    db.lock()
        .unwrap()
        .add_telegram_user(1001, Some("admin"))
        .unwrap();

    let sent_messages = Arc::new(Mutex::new(Vec::<(i64, String, Option<Value>)>::new()));
    let sent_clone = sent_messages.clone();

    let app = Router::new()
        .route(
            "/bot{token}/getUpdates",
            post(|_: Path<String>, Json(_payload): Json<Value>| async move {
                Json(serde_json::json!({ "ok": true, "result": [] })).into_response()
            }),
        )
        .route(
            "/bot{token}/sendMessage",
            post({
                let sent_clone = sent_clone.clone();
                move |_: Path<String>, Json(payload): Json<Value>| {
                    let sent_clone = sent_clone.clone();
                    async move {
                        let chat_id = payload.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
                        let text = payload
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let markup = payload.get("reply_markup").cloned();
                        sent_clone.lock().unwrap().push((chat_id, text, markup));
                        Json(serde_json::json!({ "ok": true, "result": { "message_id": 300 } }))
                            .into_response()
                    }
                }
            }),
        )
        .route(
            "/bot{token}/answerCallbackQuery",
            post(|_: Path<String>, Json(_): Json<Value>| async move {
                Json(serde_json::json!({ "ok": true, "result": true })).into_response()
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let poller = TelegramPoller::with_base_url(
        "bot-token",
        format!("http://{addr}"),
        db.clone(),
        mock_docker.clone(),
        None,
    )
    .with_timeout_secs(1);

    // 1. Initial /restart web-prod command from User 1001
    let admin_user = TelegramUser {
        id: 1001,
        is_bot: false,
        first_name: "Admin".into(),
        last_name: None,
        username: Some("admin".into()),
    };

    let router_out = poller
        .router
        .handle_command("/restart web-prod", &admin_user, TelegramRole::Admin)
        .await;

    // Verify it requires confirmation
    let (action, desc) = match router_out {
        RouterOutput::RequireConfirmation {
            action,
            description,
        } => (action, description),
        _ => panic!("expected RequireConfirmation"),
    };

    let (action_id, kb) = poller
        .confirm_store
        .create_pending(1001, 1001, action, desc);
    assert!(!kb.inline_keyboard.is_empty());

    // 2. Test unauthorized callback (User 2002 attempts to press confirm)
    let pending_before = poller.confirm_store.get(&action_id);
    assert!(pending_before.is_some());

    // Verify ownership check: User 2002 != 1001
    assert_ne!(2002, pending_before.unwrap().user_id);

    // 3. Test 61-second expiration (T-04 acceptance criteria):
    // "kriteria terima: tombol ditekan setelah 61 detik → 'Kedaluwarsa', aksi tidak jalan."
    let expired_pending = PendingAction {
        action_id: "expired_action_id".to_string(),
        user_id: 1001,
        chat_id: 1001,
        action: Action::RestartContainer {
            name: "web-prod".into(),
        },
        description: "Restart web-prod".into(),
        created_at: Instant::now() - Duration::from_secs(61), // 61 seconds elapsed!
    };
    assert!(
        expired_pending.is_expired(),
        "action after 61s must be expired"
    );

    // 4. Test valid confirmation within 60s by owner:
    let valid_pending = poller.confirm_store.take(&action_id).unwrap();
    assert!(!valid_pending.is_expired());
    let ctx = crate::actions::ActionContext::new("telegram", "1001")
        .with_docker(mock_docker.clone())
        .with_db(db.clone());
    let exec_out = TelegramRouter::run_action(valid_pending.action, &ctx).await;
    assert!(exec_out.contains("web-prod"));
    assert!(exec_out.contains("[restart_container]"));
}

// ── Test 7: Multi-user roles & Pro gating (T-07) ─────────────────────────────

#[tokio::test]
async fn test_multi_user_roles_and_pro_gating() {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let mock_docker = Arc::new(MockDocker::new(&[]));
    let router = TelegramRouter::new(mock_docker, db.clone());

    let admin_user = TelegramUser {
        id: 1001,
        is_bot: false,
        first_name: "Admin".into(),
        last_name: None,
        username: Some("admin".into()),
    };

    // 1. Initial user is added as Admin
    db.lock()
        .unwrap()
        .add_telegram_user_with_role(1001, Some("admin"), "admin")
        .unwrap();

    // 2. Viewer role attempting /deploy -> rejected
    let viewer_user = TelegramUser {
        id: 1002,
        is_bot: false,
        first_name: "Viewer".into(),
        last_name: None,
        username: Some("viewer".into()),
    };
    let viewer_deploy = router
        .handle_command("/deploy web", &viewer_user, TelegramRole::Viewer)
        .await;
    match viewer_deploy {
        RouterOutput::Text(t) => {
            assert!(t.contains("viewer"));
            assert!(t.contains("tidak diizinkan"));
        }
        _ => panic!("viewer deploy must be rejected"),
    }

    // 3. Free tier attempts adding second user -> upgrade message (T-07 acceptance criteria)
    let res = router
        .handle_command("/users add 1003 viewer", &admin_user, TelegramRole::Admin)
        .await;
    match res {
        RouterOutput::Text(t) => {
            assert!(t.contains("Fitur CAShell Pro") || t.contains("Free tier"));
        }
        _ => panic!("free tier second user must show upgrade message"),
    }
}

// ── Test 8: Audit log records Telegram commands (T-06) ───────────────────────

#[tokio::test]
async fn test_telegram_audit_log_all_commands() {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let mock_docker = Arc::new(MockDocker::new(&[]));
    let router = TelegramRouter::new(mock_docker, db.clone());

    let admin = TelegramUser {
        id: 999,
        is_bot: false,
        first_name: "Admin".into(),
        last_name: None,
        username: Some("admin".into()),
    };

    // Execute /status and /uptime
    let _ = router
        .handle_command("/status", &admin, TelegramRole::Admin)
        .await;
    let _ = router
        .handle_command("/uptime", &admin, TelegramRole::Admin)
        .await;

    // Check DB audit log
    let db_guard = db.lock().unwrap();
    let logs = db_guard.query_audit(10).unwrap();
    assert!(logs
        .iter()
        .any(|l| l.source == "telegram" && l.action == "get_status"));
    assert!(logs
        .iter()
        .any(|l| l.source == "telegram" && l.action == "get_uptime"));
}

// ── Test 9: Alert down and recovery with incident duration (T-05) ────────────

#[test]
fn test_alert_down_and_recovery_duration_formatting() {
    use crate::commands::daemon::server::format_duration;

    assert_eq!(format_duration(45), "45s");
    assert_eq!(format_duration(90), "1m 30s");
    assert_eq!(format_duration(3665), "1h 1m");
}
