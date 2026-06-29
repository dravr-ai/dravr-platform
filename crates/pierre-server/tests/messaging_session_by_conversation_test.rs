// ABOUTME: Integration tests for MessagingRepository::get_session_by_pierre_conversation_id reverse lookup
// ABOUTME: Validates channel recovery from a Pierre conversation id, tenant scoping, and the unknown-id miss
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::uninlined_format_args
)]

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

use pierre_core::models::TenantId;
use pierre_database::backends::{factory::Database, CreateSessionParams};
use uuid::Uuid;

/// Create a real `chat_conversations` row and return its id.
///
/// `messaging_sessions.pierre_conversation_id` has a FK to
/// `chat_conversations(id)` (migration `20260417000001`), so a session seeded
/// with a non-null conversation id needs the conversation to exist first.
async fn seed_conversation(db: &Database, user_id: &str, tenant_id: TenantId) -> String {
    db.repositories()
        .chat
        .create_conversation(user_id, tenant_id, "test", "test-model", None, None)
        .await
        .unwrap()
        .id
}

/// A session seeded with a `pierre_conversation_id` is recoverable by that id,
/// and the returned row exposes `channel_type` + `channel_conversation_id` so a
/// caller can route a notice back to the originating channel.
#[tokio::test]
async fn test_get_session_by_pierre_conversation_id_returns_channel() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let session_id = Uuid::new_v4().to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_id).await;

    let params = CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id,
        channel_type: "telegram",
        channel_user_id: "tg_user_777",
        channel_conversation_id: Some("tg_chat_888"),
        pierre_conversation_id: Some(&conversation_id),
    };
    db.repositories()
        .messaging
        .create_session(&params)
        .await
        .unwrap();

    let session = db
        .repositories()
        .messaging
        .get_session_by_pierre_conversation_id(tenant_id, &conversation_id)
        .await
        .unwrap();

    assert!(
        session.is_some(),
        "session should be found by conversation id"
    );
    let session = session.unwrap();
    assert_eq!(session["channel_type"], "telegram");
    assert_eq!(session["channel_conversation_id"], "tg_chat_888");
    assert_eq!(session["pierre_conversation_id"], conversation_id);
}

/// The lookup is tenant-scoped: a different tenant asking for the same
/// conversation id gets `None`.
#[tokio::test]
async fn test_get_session_by_pierre_conversation_id_is_tenant_scoped() {
    let db = create_test_db().await;
    let (user_uuid, tenant_a) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let (_, tenant_b) = seed_user(&db).await;
    let session_id = Uuid::new_v4().to_string();
    let conversation_id = seed_conversation(&db, &user_id, tenant_a).await;

    let params = CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id: tenant_a,
        channel_type: "whatsapp",
        channel_user_id: "wa_user_55",
        channel_conversation_id: None,
        pierre_conversation_id: Some(&conversation_id),
    };
    db.repositories()
        .messaging
        .create_session(&params)
        .await
        .unwrap();

    // Same conversation id under tenant B must not resolve tenant A's session.
    let session = db
        .repositories()
        .messaging
        .get_session_by_pierre_conversation_id(tenant_b, &conversation_id)
        .await
        .unwrap();
    assert!(
        session.is_none(),
        "tenant B must not see tenant A's session"
    );

    // Sanity: tenant A still resolves it.
    let session_a = db
        .repositories()
        .messaging
        .get_session_by_pierre_conversation_id(tenant_a, &conversation_id)
        .await
        .unwrap();
    assert!(session_a.is_some(), "owning tenant should resolve session");
}

/// An unknown conversation id resolves to `None` rather than erroring.
#[tokio::test]
async fn test_get_session_by_pierre_conversation_id_unknown_returns_none() {
    let db = create_test_db().await;
    let (_, tenant_id) = seed_user(&db).await;

    let session = db
        .repositories()
        .messaging
        .get_session_by_pierre_conversation_id(tenant_id, "no-such-conversation")
        .await
        .unwrap();
    assert!(session.is_none(), "unknown conversation id should miss");
}
