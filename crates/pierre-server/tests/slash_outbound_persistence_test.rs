// ABOUTME: Seam tests for the slash-egress delivery ledger — outbound_send with an OutboundPersistSpec
// ABOUTME: Pins per-part rows, synthetic ids, content types, failure rows without retry, and the reaction join
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::missing_panics_doc)]

#[path = "helpers/messaging_fixtures.rs"]
mod messaging_fixtures;

use std::sync::Arc;
use std::time::Duration;

use messaging_fixtures::{create_test_db, seed_user, CapturingChannel, FailingChannel};
use pierre_core::models::TenantId;
use pierre_database::backends::{
    factory::Database, CreateSessionParams, UpsertChannelConfigParams,
};
use pierre_mcp_server::services::messaging_ingress::outbound_send::{
    send_channel_response, send_private_channel_response, OutboundPersistSpec,
};
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::models::{ChannelType, MessageContent, OutgoingMessage};
use pierre_messaging::turn::ConversationTurnId;
use serde_json::Value;
use tokio::time::sleep;
use uuid::Uuid;

const CHANNEL: &str = "telegram";

async fn seed_session(db: &Database, tenant_id: TenantId, user_id: &str, session_id: &str) {
    // messaging_sessions.pierre_conversation_id has an FK onto
    // chat_conversations, so the conversation row comes first — and the
    // reaction-join test needs it non-NULL, matching a resolved session.
    let conversation = db
        .repositories()
        .chat
        .create_conversation(user_id, tenant_id, "ledger test", "test-model", None, None)
        .await
        .unwrap();
    db.repositories()
        .messaging
        .create_session(&CreateSessionParams {
            id: session_id,
            user_id,
            tenant_id,
            channel_type: CHANNEL,
            channel_user_id: &format!("tg_{session_id}"),
            channel_conversation_id: Some(&format!("room_{session_id}")),
            pierre_conversation_id: Some(&conversation.id),
        })
        .await
        .unwrap();
}

async fn seed_channel_config(db: &Database, tenant_id: TenantId) {
    db.repositories()
        .messaging
        .upsert_channel_config(&UpsertChannelConfigParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            channel_type: CHANNEL,
            api_key: None,
            api_secret: None,
            webhook_secret: Some("test_secret"),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("12345:LEDGER_BOT"),
            is_active: true,
        })
        .await
        .unwrap();
}

fn outgoing(content: MessageContent) -> OutgoingMessage {
    OutgoingMessage {
        channel_type: ChannelType::Telegram,
        recipient_id: "room_1".to_owned(),
        content,
        turn_id: ConversationTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}

fn spec(
    db: &Database,
    tenant_id: TenantId,
    session_id: &str,
    chat_message_id: Option<String>,
) -> OutboundPersistSpec {
    OutboundPersistSpec {
        db: db.repositories().messaging.clone(),
        session_tenant_id: tenant_id,
        session_id: session_id.to_owned(),
        chat_message_id,
    }
}

/// Delivery + persistence run in a spawned task; poll until the ledger holds
/// `want` outbound rows (or fail after ~5s).
async fn wait_for_outbound_rows(
    db: &Database,
    session_id: &str,
    tenant_id: TenantId,
    want: usize,
) -> Vec<Value> {
    for _ in 0..100 {
        let rows: Vec<Value> = db
            .repositories()
            .messaging
            .get_session_messages(session_id, tenant_id, 50, 0)
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r["direction"] == "outbound")
            .collect();
        if rows.len() >= want {
            return rows;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("ledger never reached {want} outbound rows for session {session_id}");
}

#[tokio::test]
async fn long_reply_persists_one_outbound_row_per_part() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    seed_session(&db, tenant_id, &user_uuid.to_string(), "sess-parts").await;
    seed_channel_config(&db, tenant_id).await;

    let channel = Arc::new(CapturingChannel::default());
    channel.stub_receipt_ids([Some("tg-msg-1".to_owned()), Some("tg-msg-2".to_owned())]);
    let adapter: Arc<dyn MessagingChannel> = channel.clone();

    // Past Telegram's 4096 ceiling, so the body splits into two parts.
    let message = outgoing(MessageContent::Text {
        body: "plan line\n".repeat(500),
    });
    let turn = message.turn_id.to_string();
    send_channel_response(
        db.repositories().messaging.as_ref(),
        tenant_id,
        CHANNEL,
        &adapter,
        message,
        Some(spec(&db, tenant_id, "sess-parts", None)),
    )
    .await;

    let rows = wait_for_outbound_rows(&db, "sess-parts", tenant_id, 2).await;
    assert_eq!(rows.len(), 2, "one ledger row per delivered part");
    assert_eq!(rows[0]["channel_message_id"], "tg-msg-1");
    assert_eq!(rows[1]["channel_message_id"], "tg-msg-2");
    for row in &rows {
        assert_eq!(row["sender_id"], "pierre");
        assert_eq!(row["content_type"], "text");
        assert_eq!(
            row["correlation_id"],
            turn.as_str(),
            "both parts share the turn id"
        );
        assert!(
            row["content_body"].as_str().unwrap().contains("plan line"),
            "the delivered chunk is the recorded body"
        );
    }
    assert_eq!(channel.sent.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn missing_receipt_ids_get_distinct_synthetic_ids() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    seed_session(&db, tenant_id, &user_uuid.to_string(), "sess-synth").await;
    seed_channel_config(&db, tenant_id).await;

    let channel = Arc::new(CapturingChannel::default());
    let adapter: Arc<dyn MessagingChannel> = channel.clone();

    let message = outgoing(MessageContent::Text {
        body: "no receipt\n".repeat(500),
    });
    send_channel_response(
        db.repositories().messaging.as_ref(),
        tenant_id,
        CHANNEL,
        &adapter,
        message,
        Some(spec(&db, tenant_id, "sess-synth", None)),
    )
    .await;

    let rows = wait_for_outbound_rows(&db, "sess-synth", tenant_id, 2).await;
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r["channel_message_id"].as_str().unwrap())
        .collect();
    assert!(
        ids.iter().all(|id| id.starts_with("sent-")),
        "a channel returning no id gets the sent- synthetic key, got {ids:?}"
    );
    assert_ne!(
        ids[0], ids[1],
        "synthetic ids never collide on the unique index"
    );
}

#[tokio::test]
async fn card_reply_row_carries_card_content_type() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    seed_session(&db, tenant_id, &user_uuid.to_string(), "sess-card").await;
    seed_channel_config(&db, tenant_id).await;

    let channel = Arc::new(CapturingChannel::default());
    channel.stub_receipt_ids([Some("tg-card-1".to_owned())]);
    let adapter: Arc<dyn MessagingChannel> = channel.clone();

    let message = outgoing(MessageContent::Card {
        title: "Your week".to_owned(),
        body: "Tuesday: intervals".to_owned(),
        actions: Vec::new(),
    });
    send_channel_response(
        db.repositories().messaging.as_ref(),
        tenant_id,
        CHANNEL,
        &adapter,
        message,
        Some(spec(&db, tenant_id, "sess-card", None)),
    )
    .await;

    let rows = wait_for_outbound_rows(&db, "sess-card", tenant_id, 1).await;
    assert_eq!(
        rows[0]["content_type"], "card",
        "cards are not mislabeled as text"
    );
    assert_eq!(rows[0]["content_body"], "Tuesday: intervals");
}

#[tokio::test]
async fn private_reply_persists_without_chat_message_id() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    seed_session(&db, tenant_id, &user_uuid.to_string(), "sess-priv").await;
    seed_channel_config(&db, tenant_id).await;

    let channel = Arc::new(CapturingChannel::default());
    channel.stub_receipt_ids([Some("tg-priv-1".to_owned())]);
    let adapter: Arc<dyn MessagingChannel> = channel.clone();

    let message = outgoing(MessageContent::Text {
        body: "your providers".to_owned(),
    });
    send_private_channel_response(
        db.repositories().messaging.as_ref(),
        tenant_id,
        CHANNEL,
        &adapter,
        message,
        "tg_caller_7",
        Some(spec(&db, tenant_id, "sess-priv", None)),
    )
    .await;

    let rows = wait_for_outbound_rows(&db, "sess-priv", tenant_id, 1).await;
    assert_eq!(rows[0]["channel_message_id"], "tg-priv-1");
    // A private reply is never chat-persisted, so nothing resolves for rating.
    let target = db
        .repositories()
        .messaging
        .find_reaction_feedback_target(CHANNEL, "tg-priv-1", None)
        .await
        .unwrap();
    assert!(target.is_none(), "no chat row, so no reaction target");
}

#[tokio::test]
async fn failed_send_records_the_attempt_without_queueing() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    seed_session(&db, tenant_id, &user_uuid.to_string(), "sess-fail").await;
    seed_channel_config(&db, tenant_id).await;

    let channel = Arc::new(FailingChannel::for_channel(ChannelType::Telegram));
    let adapter: Arc<dyn MessagingChannel> = channel.clone();

    let message = outgoing(MessageContent::Text {
        body: "will not arrive".to_owned(),
    });
    send_channel_response(
        db.repositories().messaging.as_ref(),
        tenant_id,
        CHANNEL,
        &adapter,
        message,
        Some(spec(&db, tenant_id, "sess-fail", None)),
    )
    .await;

    let rows = wait_for_outbound_rows(&db, "sess-fail", tenant_id, 1).await;
    let id = rows[0]["channel_message_id"].as_str().unwrap();
    assert!(
        id.starts_with("failed-"),
        "the attempt is visible in the ledger: {id}"
    );
    assert_eq!(rows[0]["content_body"], "will not arrive");
    // Slash sends are synchronous request/response: the failed part is
    // recorded but never queued — a retried private reply would repost
    // through `send`, addressed to the room.
    let pending = db
        .repositories()
        .messaging
        .get_all_pending_outbound(10)
        .await
        .unwrap();
    assert!(pending.is_empty(), "no retry queue entry for a slash send");
}

#[tokio::test]
async fn no_spec_persists_nothing() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    seed_session(&db, tenant_id, &user_uuid.to_string(), "sess-none").await;
    seed_channel_config(&db, tenant_id).await;

    let channel = Arc::new(CapturingChannel::default());
    let adapter: Arc<dyn MessagingChannel> = channel.clone();

    let message = outgoing(MessageContent::Text {
        body: "pre-session furniture".to_owned(),
    });
    send_channel_response(
        db.repositories().messaging.as_ref(),
        tenant_id,
        CHANNEL,
        &adapter,
        message,
        None,
    )
    .await;

    // Wait for the spawned delivery itself, then confirm the ledger is empty.
    for _ in 0..100 {
        if !channel.sent.lock().unwrap().is_empty() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        channel.sent.lock().unwrap().len(),
        1,
        "the send itself happened"
    );
    let rows = db
        .repositories()
        .messaging
        .get_session_messages("sess-none", tenant_id, 50, 0)
        .await
        .unwrap();
    assert!(rows.is_empty(), "a None spec writes no ledger rows");
}

#[tokio::test]
async fn chat_message_id_rides_into_the_reaction_join() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    seed_session(&db, tenant_id, &user_id, "sess-join").await;
    seed_channel_config(&db, tenant_id).await;

    let channel = Arc::new(CapturingChannel::default());
    channel.stub_receipt_ids([Some("tg-join-1".to_owned())]);
    let adapter: Arc<dyn MessagingChannel> = channel.clone();

    let message = outgoing(MessageContent::Text {
        body: "shared plan".to_owned(),
    });
    send_channel_response(
        db.repositories().messaging.as_ref(),
        tenant_id,
        CHANNEL,
        &adapter,
        message,
        Some(spec(
            &db,
            tenant_id,
            "sess-join",
            Some("chat-row-42".to_owned()),
        )),
    )
    .await;

    wait_for_outbound_rows(&db, "sess-join", tenant_id, 1).await;
    // The production consumer: an emoji reaction on the delivered channel
    // message resolves to the assistant chat row the spec carried.
    let target = db
        .repositories()
        .messaging
        .find_reaction_feedback_target(CHANNEL, "tg-join-1", None)
        .await
        .unwrap()
        .expect("the outbound row is a reaction target");
    assert_eq!(target.chat_message_id, "chat-row-42");
    assert_eq!(target.tenant_id, tenant_id);
    assert_eq!(target.user_id, user_id);
}
