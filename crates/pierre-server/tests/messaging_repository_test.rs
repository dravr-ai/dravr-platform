// ABOUTME: Integration tests for MessagingRepository SQLite implementation
// ABOUTME: Validates channel config CRUD, session management, message idempotency, and outbound queue
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::wildcard_in_or_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args,
    clippy::redundant_closure_for_method_calls
)]

use chrono::Utc;
use pierre_database::{
    plugins::{
        factory::Database, CreateSessionParams, InsertMessageParams, UpsertChannelConfigParams,
    },
    DatabaseProvider,
};
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::environment::PostgresPoolConfig;
use pierre_mcp_server::models::{Tenant, TenantId, User};
use uuid::Uuid;

/// Create an in-memory `SQLite` database for testing
async fn create_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();

    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await
    .expect("Failed to create test database");

    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key)
        .await
        .expect("Failed to create test database");

    db.migrate().await.expect("Failed to run migrations");
    db
}

/// Seed a real user and tenant so FK constraints on `messaging_*` tables are satisfied.
/// Returns `(user_uuid, tenant_id)`; stringify `user_uuid` when passing to messaging params.
async fn seed_user(db: &Database) -> (Uuid, TenantId) {
    let email = format!("user-{}@test.local", Uuid::new_v4());
    let user = User::new(
        email,
        "hash_not_verified_in_tests".to_owned(),
        Some("Test User".to_owned()),
    );
    let user_id = user.id;
    db.repositories().users.create(&user).await.unwrap();

    let tenant_id = TenantId::new();
    let now = Utc::now();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Test Tenant {tenant_id}"),
        slug: tenant_id.to_string(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    db.repositories().tenants.create(&tenant).await.unwrap();

    (user_id, tenant_id)
}

/// Seed only a tenant (with a throwaway owner user).
async fn seed_tenant(db: &Database) -> TenantId {
    let (_, tenant_id) = seed_user(db).await;
    tenant_id
}

/// Fresh tenant id for tables that have no FK to tenants (e.g. `messaging_channel_configs`).
fn test_tenant_id() -> TenantId {
    TenantId::new()
}

/// Create a session for FK relationships (messages reference sessions)
async fn create_test_session(db: &Database, session_id: &str, tenant_id: TenantId, user_id: &str) {
    let params = CreateSessionParams {
        id: session_id,
        user_id,
        tenant_id,
        channel_type: "whatsapp",
        channel_user_id: &format!("ch_user_{session_id}"),
        channel_conversation_id: None,
        pierre_conversation_id: None,
    };
    db.repositories()
        .messaging
        .create_session(&params)
        .await
        .unwrap();
}

/// Create a session and a message for FK relationships (receipts/queue reference messages)
async fn create_test_message(
    db: &Database,
    msg_id: &str,
    session_id: &str,
    tenant_id: TenantId,
    user_id: &str,
    channel_message_id: &str,
) {
    create_test_session(db, session_id, tenant_id, user_id).await;
    let params = InsertMessageParams {
        id: msg_id,
        tenant_id,
        session_id,
        direction: "inbound",
        channel_type: "whatsapp",
        channel_message_id,
        sender_id: "test-sender",
        content_type: "text",
        content_body: Some("test"),
        correlation_id: "test-corr",
        raw_payload: None,
    };
    db.repositories()
        .messaging
        .insert_message(&params)
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════════
// Channel Config CRUD Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_upsert_and_get_channel_config() {
    let db = create_test_db().await;
    let tenant_id = test_tenant_id();
    let config_id = Uuid::new_v4().to_string();

    let params = UpsertChannelConfigParams {
        id: &config_id,
        tenant_id,
        channel_type: "whatsapp",
        api_key: Some("wa_access_token"),
        api_secret: None,
        webhook_secret: Some("wh_secret"),
        verify_token: None,
        account_id: Some("AC123"),
        phone_number: Some("+15551234567"),
        bot_token: None,
        is_active: true,
    };

    db.repositories()
        .messaging
        .upsert_channel_config(&params)
        .await
        .unwrap();

    let config = db
        .repositories()
        .messaging
        .get_channel_config(tenant_id, "whatsapp")
        .await
        .unwrap();
    assert!(config.is_some());
    let config = config.unwrap();
    assert_eq!(config["channel_type"], "whatsapp");
    assert_eq!(config["is_active"], true);
}

#[tokio::test]
async fn test_upsert_updates_existing_config() {
    let db = create_test_db().await;
    let tenant_id = test_tenant_id();
    let config_id = Uuid::new_v4().to_string();

    let params = UpsertChannelConfigParams {
        id: &config_id,
        tenant_id,
        channel_type: "slack",
        api_key: Some("old_key"),
        api_secret: None,
        webhook_secret: Some("old_secret"),
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: None,
        is_active: true,
    };
    db.repositories()
        .messaging
        .upsert_channel_config(&params)
        .await
        .unwrap();

    // Upsert again with new values
    let new_id = Uuid::new_v4().to_string();
    let params2 = UpsertChannelConfigParams {
        id: &new_id,
        tenant_id,
        channel_type: "slack",
        api_key: Some("new_key"),
        api_secret: None,
        webhook_secret: Some("new_secret"),
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: None,
        is_active: false,
    };
    db.repositories()
        .messaging
        .upsert_channel_config(&params2)
        .await
        .unwrap();

    let config = db
        .repositories()
        .messaging
        .get_channel_config(tenant_id, "slack")
        .await
        .unwrap();
    let config = config.unwrap();
    assert_eq!(config["is_active"], false);
}

#[tokio::test]
async fn test_list_channel_configs() {
    let db = create_test_db().await;
    let tenant_id = test_tenant_id();

    // Insert two configs
    for channel in &["whatsapp", "telegram"] {
        let id = Uuid::new_v4().to_string();
        let params = UpsertChannelConfigParams {
            id: &id,
            tenant_id,
            channel_type: channel,
            api_key: None,
            api_secret: None,
            webhook_secret: Some("secret"),
            verify_token: None,
            account_id: None,
            phone_number: None,
            bot_token: Some("bot_tok"),
            is_active: true,
        };
        db.repositories()
            .messaging
            .upsert_channel_config(&params)
            .await
            .unwrap();
    }

    let configs = db
        .repositories()
        .messaging
        .list_channel_configs(tenant_id)
        .await
        .unwrap();
    assert_eq!(configs.len(), 2);
}

#[tokio::test]
async fn test_delete_channel_config() {
    let db = create_test_db().await;
    let tenant_id = test_tenant_id();
    let config_id = Uuid::new_v4().to_string();

    let params = UpsertChannelConfigParams {
        id: &config_id,
        tenant_id,
        channel_type: "discord",
        api_key: None,
        api_secret: None,
        webhook_secret: None,
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: Some("bot_token"),
        is_active: true,
    };
    db.repositories()
        .messaging
        .upsert_channel_config(&params)
        .await
        .unwrap();

    let deleted = db
        .repositories()
        .messaging
        .delete_channel_config(tenant_id, "discord")
        .await
        .unwrap();
    assert!(deleted);

    let config = db
        .repositories()
        .messaging
        .get_channel_config(tenant_id, "discord")
        .await
        .unwrap();
    assert!(config.is_none());
}

#[tokio::test]
async fn test_delete_nonexistent_config() {
    let db = create_test_db().await;
    let deleted = db
        .repositories()
        .messaging
        .delete_channel_config(test_tenant_id(), "whatsapp")
        .await
        .unwrap();
    assert!(!deleted);
}

#[tokio::test]
async fn test_tenant_isolation_channel_config() {
    let db = create_test_db().await;
    let tenant_a = test_tenant_id();
    let tenant_b = test_tenant_id();

    let id_a = Uuid::new_v4().to_string();
    let params_a = UpsertChannelConfigParams {
        id: &id_a,
        tenant_id: tenant_a,
        channel_type: "whatsapp",
        api_key: Some("key_a"),
        api_secret: None,
        webhook_secret: None,
        verify_token: None,
        account_id: None,
        phone_number: None,
        bot_token: None,
        is_active: true,
    };
    db.repositories()
        .messaging
        .upsert_channel_config(&params_a)
        .await
        .unwrap();

    // Tenant B should not see tenant A's config
    let config_b = db
        .repositories()
        .messaging
        .get_channel_config(tenant_b, "whatsapp")
        .await
        .unwrap();
    assert!(config_b.is_none());

    let configs_b = db
        .repositories()
        .messaging
        .list_channel_configs(tenant_b)
        .await
        .unwrap();
    assert!(configs_b.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Session Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_create_and_lookup_session() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let session_id = Uuid::new_v4().to_string();

    let params = CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id,
        channel_type: "telegram",
        channel_user_id: "tg_user_100",
        channel_conversation_id: Some("tg_chat_200"),
        pierre_conversation_id: None,
    };
    db.repositories()
        .messaging
        .create_session(&params)
        .await
        .unwrap();

    let session = db
        .repositories()
        .messaging
        .get_session_by_channel_identity(tenant_id, "telegram", "tg_user_100")
        .await
        .unwrap();
    assert!(session.is_some());
    let session = session.unwrap();
    assert_eq!(session["channel_user_id"], "tg_user_100");
}

#[tokio::test]
async fn test_touch_session_updates_timestamp() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let session_id = Uuid::new_v4().to_string();

    let params = CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id,
        channel_type: "slack",
        channel_user_id: "U1234",
        channel_conversation_id: Some("C5678"),
        pierre_conversation_id: None,
    };
    db.repositories()
        .messaging
        .create_session(&params)
        .await
        .unwrap();

    // Touch should succeed without error
    db.repositories()
        .messaging
        .touch_session(&session_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_session_tenant_isolation() {
    let db = create_test_db().await;
    let (user_uuid, tenant_a) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let tenant_b = seed_tenant(&db).await;
    let session_id = Uuid::new_v4().to_string();

    let params = CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id: tenant_a,
        channel_type: "whatsapp",
        channel_user_id: "wa_user_42",
        channel_conversation_id: None,
        pierre_conversation_id: None,
    };
    db.repositories()
        .messaging
        .create_session(&params)
        .await
        .unwrap();

    // Tenant B should not see tenant A's session
    let session = db
        .repositories()
        .messaging
        .get_session_by_channel_identity(tenant_b, "whatsapp", "wa_user_42")
        .await
        .unwrap();
    assert!(session.is_none());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Message Idempotency Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_insert_message_returns_true_on_first_insert() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let msg_id = Uuid::new_v4().to_string();

    // Create session first (FK constraint)
    create_test_session(&db, "session-1", tenant_id, &user_id).await;

    let params = InsertMessageParams {
        id: &msg_id,
        tenant_id,
        session_id: "session-1",
        direction: "inbound",
        channel_type: "whatsapp",
        channel_message_id: "SM12345",
        sender_id: "sender-1",
        content_type: "text",
        content_body: Some("Hello"),
        correlation_id: "corr-1",
        raw_payload: Some(r#"{"Body":"Hello"}"#),
    };

    let inserted = db
        .repositories()
        .messaging
        .insert_message(&params)
        .await
        .unwrap();
    assert!(inserted, "First insert should return true");
}

#[tokio::test]
async fn test_insert_duplicate_message_returns_false() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let msg_id_1 = Uuid::new_v4().to_string();
    let msg_id_2 = Uuid::new_v4().to_string();

    create_test_session(&db, "session-dup", tenant_id, &user_id).await;

    let params1 = InsertMessageParams {
        id: &msg_id_1,
        tenant_id,
        session_id: "session-dup",
        direction: "inbound",
        channel_type: "whatsapp",
        channel_message_id: "SM12345",
        sender_id: "sender-1",
        content_type: "text",
        content_body: Some("Hello"),
        correlation_id: "corr-1",
        raw_payload: None,
    };
    let first = db
        .repositories()
        .messaging
        .insert_message(&params1)
        .await
        .unwrap();
    assert!(first);

    // Same channel_message_id + tenant_id = duplicate
    let params2 = InsertMessageParams {
        id: &msg_id_2,
        tenant_id,
        session_id: "session-dup",
        direction: "inbound",
        channel_type: "whatsapp",
        channel_message_id: "SM12345",
        sender_id: "sender-1",
        content_type: "text",
        content_body: Some("Hello again"),
        correlation_id: "corr-2",
        raw_payload: None,
    };
    let second = db
        .repositories()
        .messaging
        .insert_message(&params2)
        .await
        .unwrap();
    assert!(!second, "Duplicate message should return false");
}

#[tokio::test]
async fn test_get_session_messages() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();

    create_test_session(&db, "session-abc", tenant_id, &user_id).await;

    // Insert 3 messages for the same session
    for i in 0..3 {
        let msg_id = Uuid::new_v4().to_string();
        let channel_msg_id = format!("SM{i}");
        let params = InsertMessageParams {
            id: &msg_id,
            tenant_id,
            session_id: "session-abc",
            direction: "inbound",
            channel_type: "telegram",
            channel_message_id: &channel_msg_id,
            sender_id: "sender-1",
            content_type: "text",
            content_body: Some(&format!("Message {i}")),
            correlation_id: &format!("corr-{i}"),
            raw_payload: None,
        };
        db.repositories()
            .messaging
            .insert_message(&params)
            .await
            .unwrap();
    }

    let messages = db
        .repositories()
        .messaging
        .get_session_messages("session-abc", tenant_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(messages.len(), 3);
}

#[tokio::test]
async fn test_get_session_messages_pagination() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();

    create_test_session(&db, "session-pag", tenant_id, &user_id).await;

    for i in 0..5 {
        let msg_id = Uuid::new_v4().to_string();
        let channel_msg_id = format!("MSG{i}");
        let params = InsertMessageParams {
            id: &msg_id,
            tenant_id,
            session_id: "session-pag",
            direction: "inbound",
            channel_type: "slack",
            channel_message_id: &channel_msg_id,
            sender_id: "sender-1",
            content_type: "text",
            content_body: Some("msg"),
            correlation_id: &format!("corr-{i}"),
            raw_payload: None,
        };
        db.repositories()
            .messaging
            .insert_message(&params)
            .await
            .unwrap();
    }

    // First page: limit 2
    let page1 = db
        .repositories()
        .messaging
        .get_session_messages("session-pag", tenant_id, 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);

    // Second page: offset 2, limit 2
    let page2 = db
        .repositories()
        .messaging
        .get_session_messages("session-pag", tenant_id, 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);

    // Third page: offset 4, limit 2
    let page3 = db
        .repositories()
        .messaging
        .get_session_messages("session-pag", tenant_id, 2, 4)
        .await
        .unwrap();
    assert_eq!(page3.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Delivery Receipt Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_insert_delivery_receipt() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let receipt_id = Uuid::new_v4().to_string();

    // Create session + message first (FK constraint)
    create_test_message(
        &db,
        "msg-rcpt",
        "session-rcpt",
        tenant_id,
        &user_id,
        "CM_RCPT1",
    )
    .await;

    db.repositories()
        .messaging
        .insert_delivery_receipt(
            &receipt_id,
            tenant_id,
            "msg-rcpt",
            Some("ext-msg-1"),
            "sent",
        )
        .await
        .unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════════
// Outbound Queue Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_enqueue_and_get_pending_outbound() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let queue_id = Uuid::new_v4().to_string();

    // Create session + message first (FK constraint)
    create_test_message(&db, "msg-q1", "session-q1", tenant_id, &user_id, "CM_Q1").await;

    let user_id = uuid::Uuid::new_v4().to_string();
    db.repositories()
        .messaging
        .enqueue_outbound(
            &queue_id,
            "msg-q1",
            tenant_id,
            Some(user_id.as_str()),
            "whatsapp",
            r#"{"Body":"Hi"}"#,
        )
        .await
        .unwrap();

    let pending = db
        .repositories()
        .messaging
        .get_pending_outbound(tenant_id, 10)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["channel_type"], "whatsapp");
    assert_eq!(pending[0]["status"], "pending");
    // Phase 4: user_id round-trips through enqueue + get_pending_outbound
    // so the dead-letter PostHog distinct_id has a real user to attach to.
    assert_eq!(pending[0]["user_id"].as_str(), Some(user_id.as_str()));
}

#[tokio::test]
async fn test_update_outbound_status() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let queue_id = Uuid::new_v4().to_string();

    // Create session + message first (FK constraint)
    create_test_message(&db, "msg-q2", "session-q2", tenant_id, &user_id, "CM_Q2").await;

    db.repositories()
        .messaging
        .enqueue_outbound(
            &queue_id,
            "msg-q2",
            tenant_id,
            None,
            "telegram",
            r#"{"text":"Hi"}"#,
        )
        .await
        .unwrap();

    // Mark as sent
    db.repositories()
        .messaging
        .update_outbound_status(&queue_id, "sent", 1, None)
        .await
        .unwrap();

    // Should no longer appear in pending
    let pending = db
        .repositories()
        .messaging
        .get_pending_outbound(tenant_id, 10)
        .await
        .unwrap();
    assert!(
        pending.is_empty(),
        "Sent items should not appear as pending"
    );
}

#[tokio::test]
async fn test_outbound_queue_tenant_isolation() {
    let db = create_test_db().await;
    let (user_uuid, tenant_a) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let tenant_b = seed_tenant(&db).await;
    let queue_id = Uuid::new_v4().to_string();

    // Create session + message in tenant A (FK constraint)
    create_test_message(&db, "msg-q3", "session-q3", tenant_a, &user_id, "CM_Q3").await;

    db.repositories()
        .messaging
        .enqueue_outbound(
            &queue_id,
            "msg-q3",
            tenant_a,
            None,
            "slack",
            r#"{"text":"Hi"}"#,
        )
        .await
        .unwrap();

    // Tenant B should see empty queue
    let pending = db
        .repositories()
        .messaging
        .get_pending_outbound(tenant_b, 10)
        .await
        .unwrap();
    assert!(pending.is_empty());
}

#[tokio::test]
async fn test_retry_worker_dead_letters_invalid_tenant() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let queue_id = Uuid::new_v4().to_string();

    // Create session + message (FK constraint)
    create_test_message(&db, "msg-dlq", "session-dlq", tenant_id, &user_id, "CM_DLQ").await;

    // Enqueue an outbound message
    db.repositories()
        .messaging
        .enqueue_outbound(
            &queue_id,
            "msg-dlq",
            tenant_id,
            None,
            "whatsapp",
            r#"{"Body":"Hi"}"#,
        )
        .await
        .unwrap();

    // Verify it appears in pending
    let pending = db
        .repositories()
        .messaging
        .get_all_pending_outbound(10)
        .await
        .unwrap();
    assert!(
        pending.iter().any(|e| e["id"].as_str() == Some(&queue_id)),
        "Entry should be pending before dead-letter"
    );

    // Simulate the retry worker dead-lettering an entry (e.g., invalid tenant_id parse)
    db.repositories()
        .messaging
        .update_outbound_status(&queue_id, "dlq", 1, None)
        .await
        .unwrap();

    // Entry should no longer appear in pending queue
    let pending_after = db
        .repositories()
        .messaging
        .get_all_pending_outbound(10)
        .await
        .unwrap();
    assert!(
        !pending_after
            .iter()
            .any(|e| e["id"].as_str() == Some(&queue_id)),
        "Dead-lettered entry should not appear in pending queue"
    );
}
