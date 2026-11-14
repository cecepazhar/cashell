//! Unit & integration tests for Telegram module (T-01, T-02, T-03).

use axum::extract::{Json, Path};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use serde_json::Value;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use super::client::truncate_message;
use super::polling::TelegramPoller;
use super::router::TelegramRouter;
use super::types::TelegramUser;
use crate::actions::MockDocker;
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

// ── Test 2: Router deterministic commands via typed actions (T-03) ───────────

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
    let help_res = router.handle_command("/help", &user).await;
    assert!(help_res.contains("/status"));
    assert!(help_res.contains("/uptime"));
    assert!(help_res.contains("/containers"));

    // /start
    let start_res = router.handle_command("/start", &user).await;
    assert!(start_res.contains("Welcome to CAShell Telegram Bot"));

    // /status
    let status_res = router.handle_command("/status", &user).await;
    assert!(status_res.contains("[get_status]"));

    // /uptime
    let uptime_res = router.handle_command("/uptime", &user).await;
    assert!(uptime_res.contains("[get_uptime]"));

    // /containers
    let containers_res = router.handle_command("/containers", &user).await;
    assert!(containers_res.contains("[list_containers]"));
    assert!(containers_res.contains("Found 1 containers"));

    // /restart web-prod
    let restart_res = router.handle_command("/restart web-prod", &user).await;
    assert!(restart_res.contains("[restart_container]"));
    assert!(restart_res.contains("web-prod"));

    // /restart (missing args)
    let restart_err = router.handle_command("/restart", &user).await;
    assert_eq!(restart_err, "Usage: /restart <container>");

    // /deploy api-gateway
    let deploy_res = router.handle_command("/deploy api-gateway", &user).await;
    assert!(deploy_res.contains("[deploy_app]"));
    assert!(deploy_res.contains("Deployment triggered for application 'api-gateway'"));

    // /logs web-prod 30
    let logs_res = router.handle_command("/logs web-prod 30", &user).await;
    assert!(logs_res.contains("[get_logs]"));

    // /logs (missing app)
    let logs_err = router.handle_command("/logs", &user).await;
    assert_eq!(logs_err, "Usage: /logs <app> [lines≤200]");

    // /analytics
    let analytics_res = router
        .handle_command("/analytics fathforce 14", &user)
        .await;
    assert!(analytics_res.contains("[get_analytics]"));

    // Unknown command
    let unknown_res = router.handle_command("/unknowncmd", &user).await;
    assert_eq!(
        unknown_res,
        "Unknown command. Send /help for available commands."
    );

    // Non-slash text
    let plain_res = router.handle_command("hello bot", &user).await;
    assert_eq!(plain_res, "Send /help to view available commands.");
}

// ── Test 3: Allowlist enforcement and audit logging for unauthorized senders (T-02) ──

#[tokio::test]
async fn test_allowlist_enforcement_and_audit_log() {
    let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
    let mock_docker = Arc::new(MockDocker::new(&[]));

    // Allow user 1001, but NOT user 2002
    db.lock()
        .unwrap()
        .add_telegram_user(1001, Some("admin"))
        .unwrap();

    let sent_messages = Arc::new(Mutex::new(Vec::<(i64, String)>::new()));
    let sent_clone = sent_messages.clone();

    // Mock Telegram HTTP Server
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
                            // First run: return updates 50 and 51
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
                            // Offset advanced: return empty
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

    // Run instance 1 of daemon poller
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

    // Verify offset was persisted in DB: highest update_id + 1 = 51 + 1 = 52
    let persisted_offset = db.lock().unwrap().get_telegram_offset().unwrap();
    assert_eq!(persisted_offset, 52);

    // SIMULATE DAEMON RESTART: Create a brand new poller instance with same DB
    let poller2 = TelegramPoller::with_base_url(
        "bot-token",
        format!("http://{addr}"),
        db.clone(),
        mock_docker.clone(),
        None,
    )
    .with_timeout_secs(1);

    let processed2 = poller2.poll_once().await.unwrap();
    assert_eq!(processed2, 0); // No messages reprocessed!

    // Verify that the second poller requested with offset 52
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
            ).into_response()
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
