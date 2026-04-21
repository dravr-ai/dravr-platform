// ABOUTME: Regression guard for open_status_adapter's channel_type → adapter dispatch
// ABOUTME: Uses per-channel mock endpoints so swapping match arms would break path assertions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::services::messaging_status_bridge::{open_status_adapter, OpenStatusParams};
use pierre_messaging::models::{ChannelConfig, ChannelType};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// One mock server with per-channel routes. Records every path hit
/// so the test can assert Telegram → `/bot*/sendMessage`, Slack →
/// `/chat.postMessage`, Discord → `/channels/*/messages` (and
/// nothing else).
async fn spawn_multi_channel_mock() -> (String, Arc<Mutex<Vec<String>>>, JoinHandle<()>) {
    use axum::extract::{Path, State};
    use axum::routing::post;
    use axum::{Json, Router};

    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let state = calls.clone();

    let app = Router::new()
        // Telegram placeholder: sendMessage returns a result with
        // a numeric `message_id` that `TelegramStatusAdapter::open`
        // parses and stores for later editMessageText calls.
        .route(
            "/bot{token}/sendMessage",
            post(
                |Path(_token): Path<String>,
                 State(state): State<Arc<Mutex<Vec<String>>>>,
                 Json(_body): Json<Value>| async move {
                    state
                        .lock()
                        .expect("mutex")
                        .push("telegram:sendMessage".to_owned());
                    Json(json!({ "ok": true, "result": { "message_id": 4242 } }))
                },
            ),
        )
        // Slack placeholder: chat.postMessage returns `ok: true` +
        // `ts` so `SlackStatusAdapter::open` can record the
        // timestamp for later chat.update calls.
        .route(
            "/chat.postMessage",
            post(
                |State(state): State<Arc<Mutex<Vec<String>>>>, Json(_body): Json<Value>| async move {
                    state
                        .lock()
                        .expect("mutex")
                        .push("slack:chat.postMessage".to_owned());
                    Json(json!({ "ok": true, "ts": "1700000000.000100", "channel": "C1" }))
                },
            ),
        )
        // Discord placeholder: POST /channels/{id}/messages returns
        // the created message's `id` so `DiscordStatusAdapter::open`
        // can target later PATCH calls at it.
        .route(
            "/channels/{channel_id}/messages",
            post(
                |Path(channel_id): Path<String>,
                 State(state): State<Arc<Mutex<Vec<String>>>>,
                 Json(_body): Json<Value>| async move {
                    state
                        .lock()
                        .expect("mutex")
                        .push("discord:POST-messages".to_owned());
                    Json(json!({ "id": "msg-1", "channel_id": channel_id }))
                },
            ),
        )
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), calls, handle)
}

/// Build a `ChannelConfig` carrying both `api_key` (Slack) and
/// `bot_token` (Telegram + Discord) so the same config works for
/// every adapter open path.
fn test_channel_config(channel_type: ChannelType) -> ChannelConfig {
    ChannelConfig {
        id: "cfg-1".into(),
        tenant_id: "t1".into(),
        channel_type,
        api_key: Some("xoxb-fake".into()),
        api_secret: None,
        webhook_secret: None,
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: Some("bot-fake".into()),
        is_active: true,
    }
}

/// `WhatsApp` + Messenger return `None` without any network call
/// because neither platform's API supports in-place message edits.
///
/// This pins the explicit architectural skip so a future refactor
/// that adds a new channel arm can't silently route WA/Messenger
/// through a status adapter and spam the user with new messages.
#[tokio::test(flavor = "multi_thread")]
async fn dispatch_whatsapp_and_messenger_skip_adapter() {
    let (mock_base, calls, handle) = spawn_multi_channel_mock().await;

    for channel in [ChannelType::WhatsApp, ChannelType::Messenger] {
        let cfg = test_channel_config(channel);
        let params = OpenStatusParams {
            channel_type: channel,
            channel_config: &cfg,
            conversation_id: "conv-1",
            thread_id: None,
            placeholder_text: "thinking…",
            api_base_override: Some(&mock_base),
        };
        let result = open_status_adapter(&params).await;
        assert!(
            result.is_none(),
            "{channel:?} must return None (no edit-message API)"
        );
    }

    // No network call happened for either variant.
    let captured = calls.lock().expect("mutex").clone();
    assert!(
        captured.is_empty(),
        "WhatsApp/Messenger dispatch must NOT hit the network: {captured:?}"
    );
    handle.abort();
}

/// `ChannelType::Telegram` routes to the Telegram adapter, which
/// hits `POST /bot{token}/sendMessage`.
///
/// Using a real mock server + `api_base_override`, we assert the
/// captured path matches Telegram's endpoint shape. If a refactor
/// swapped the match arm to call `open_slack` or `open_discord`
/// instead, this test would fail because the mock would record the
/// wrong endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn dispatch_telegram_hits_send_message() {
    let (mock_base, calls, handle) = spawn_multi_channel_mock().await;

    let cfg = test_channel_config(ChannelType::Telegram);
    let params = OpenStatusParams {
        channel_type: ChannelType::Telegram,
        channel_config: &cfg,
        conversation_id: "chat-123",
        thread_id: None,
        placeholder_text: "thinking…",
        api_base_override: Some(&mock_base),
    };
    let adapter = open_status_adapter(&params).await;
    assert!(
        adapter.is_some(),
        "Telegram dispatch must return Some(adapter) when the mock responds with a valid handshake"
    );

    let captured = calls.lock().expect("mutex").clone();
    assert_eq!(
        captured,
        vec!["telegram:sendMessage".to_owned()],
        "Telegram dispatch must hit sendMessage exactly once"
    );
    handle.abort();
}

/// `ChannelType::Slack` routes to the Slack adapter, which hits
/// `POST /chat.postMessage`.
///
/// Parallel regression guard to the Telegram dispatch test — if the
/// match arms are swapped, the captured path changes.
#[tokio::test(flavor = "multi_thread")]
async fn dispatch_slack_hits_chat_post_message() {
    let (mock_base, calls, handle) = spawn_multi_channel_mock().await;

    let cfg = test_channel_config(ChannelType::Slack);
    let params = OpenStatusParams {
        channel_type: ChannelType::Slack,
        channel_config: &cfg,
        conversation_id: "C1",
        thread_id: None,
        placeholder_text: "thinking…",
        api_base_override: Some(&mock_base),
    };
    let adapter = open_status_adapter(&params).await;
    assert!(
        adapter.is_some(),
        "Slack dispatch must return Some(adapter) when the mock responds with a valid handshake"
    );

    let captured = calls.lock().expect("mutex").clone();
    assert_eq!(
        captured,
        vec!["slack:chat.postMessage".to_owned()],
        "Slack dispatch must hit chat.postMessage exactly once"
    );
    handle.abort();
}

/// `ChannelType::Discord` routes to the Discord adapter, which hits
/// `POST /channels/{channel_id}/messages`.
///
/// Parallel regression guard to Telegram and Slack — each channel
/// has a unique path prefix the mock recognizes.
#[tokio::test(flavor = "multi_thread")]
async fn dispatch_discord_hits_channel_messages() {
    let (mock_base, calls, handle) = spawn_multi_channel_mock().await;

    let cfg = test_channel_config(ChannelType::Discord);
    let params = OpenStatusParams {
        channel_type: ChannelType::Discord,
        channel_config: &cfg,
        conversation_id: "discord-chan-42",
        thread_id: None,
        placeholder_text: "thinking…",
        api_base_override: Some(&mock_base),
    };
    let adapter = open_status_adapter(&params).await;
    assert!(
        adapter.is_some(),
        "Discord dispatch must return Some(adapter) when the mock responds with a valid handshake"
    );

    let captured = calls.lock().expect("mutex").clone();
    assert_eq!(
        captured,
        vec!["discord:POST-messages".to_owned()],
        "Discord dispatch must hit POST /channels/*/messages exactly once"
    );
    handle.abort();
}
