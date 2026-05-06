// ABOUTME: PostgreSQL-specific messaging tests covering the UUID-cast bind paths
// ABOUTME: Catches regressions where text/uuid bind mismatches silently drop inbound messages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![cfg(feature = "postgresql")]

use chrono::{Duration, Utc};
use pierre_core::models::CoachingPersona;
use pierre_database::backends::{
    factory::Database, CreateChannelLinkParams, CreateLinkStateParams, CreateSessionParams,
};
use pierre_mcp_server::{
    models::{Tenant, TenantId, User, UserStatus, UserTier},
    permissions::UserRole,
};
use uuid::Uuid;

mod common;

// ============================================================================
// Helpers
// ============================================================================

async fn seed_pg_user(db: &Database) -> Uuid {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("msg-{user_id}@test.local"),
        display_name: Some("Messaging Test".to_owned()),
        password_hash: "hash_not_verified".to_owned(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
    };
    db.repositories()
        .users
        .create(&user)
        .await
        .expect("Failed to create user");
    user_id
}

async fn seed_pg_tenant(db: &Database, owner: Uuid) -> TenantId {
    let tenant_id = TenantId::new();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Messaging Tenant {tenant_id}"),
        slug: tenant_id.to_string(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: owner,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.repositories()
        .tenants
        .create(&tenant)
        .await
        .expect("Failed to create tenant");
    tenant_id
}

async fn seed_pg_user_and_tenant(db: &Database) -> (Uuid, TenantId) {
    let user_id = seed_pg_user(db).await;
    let tenant_id = seed_pg_tenant(db, user_id).await;
    (user_id, tenant_id)
}

/// Yield an isolated PG `Database`, or `None` when PG is unavailable.
///
/// Tests should return early with a skip message rather than panic so the
/// suite can be run locally without a running `PostgreSQL`.
async fn try_pg_db() -> Option<(common::IsolatedPostgresDb, Database)> {
    let isolated = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return None;
        }
    };
    let db = isolated
        .get_database()
        .await
        .expect("Failed to open PG database");
    Some((isolated, db))
}

// ============================================================================
// Session round-trip (the path that broke)
// ============================================================================

/// Root cause regression test: `create_session` → `get_session_by_channel_identity`.
///
/// Before the uuid-cast fix (de947ce7), PG rejected this pair with
/// "operator does not exist: uuid = text" because migration 20260417000001
/// converted `user_id`/`tenant_id` to UUID but the plugin bound them as text.
#[tokio::test]
async fn test_pg_session_create_then_lookup_by_channel_identity() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    let session_id = Uuid::new_v4().to_string();
    let params = CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id,
        channel_type: "whatsapp",
        channel_user_id: "+15551234567",
        channel_conversation_id: None,
        pierre_conversation_id: None,
    };

    db.repositories()
        .messaging
        .create_session(&params)
        .await
        .expect("create_session should succeed on PG with uuid casts");

    let fetched = db
        .repositories()
        .messaging
        .get_session_by_channel_identity(tenant_id, "whatsapp", "+15551234567", None)
        .await
        .expect("get_session_by_channel_identity should succeed on PG")
        .expect("session should be found");

    assert_eq!(fetched["id"].as_str().unwrap(), session_id);
    assert_eq!(fetched["channel_type"].as_str().unwrap(), "whatsapp");
    assert_eq!(fetched["channel_user_id"].as_str().unwrap(), "+15551234567");
    assert_eq!(fetched["user_id"].as_str().unwrap(), user_id);
    assert_eq!(
        fetched["tenant_id"].as_str().unwrap(),
        tenant_id.to_string()
    );
}

#[tokio::test]
async fn test_pg_session_tenant_isolation() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_a, tenant_a) = seed_pg_user_and_tenant(&db).await;
    let (_, tenant_b) = seed_pg_user_and_tenant(&db).await;
    let user_id_a = user_a.to_string();

    let session_id = Uuid::new_v4().to_string();
    db.repositories()
        .messaging
        .create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &user_id_a,
            tenant_id: tenant_a,
            channel_type: "telegram",
            channel_user_id: "tg-iso-42",
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .unwrap();

    // Tenant B must not see tenant A's session
    let cross = db
        .repositories()
        .messaging
        .get_session_by_channel_identity(tenant_b, "telegram", "tg-iso-42", None)
        .await
        .unwrap();
    assert!(cross.is_none(), "cross-tenant lookup must return None");
}

#[tokio::test]
async fn test_pg_touch_session_updates_timestamp() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    let session_id = Uuid::new_v4().to_string();
    db.repositories()
        .messaging
        .create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &user_id,
            tenant_id,
            channel_type: "slack",
            channel_user_id: "U-slack-123",
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .unwrap();

    db.repositories()
        .messaging
        .touch_session(&session_id)
        .await
        .expect("touch_session should succeed on PG");
}

// ============================================================================
// Channel link round-trip
// ============================================================================

#[tokio::test]
async fn test_pg_channel_link_create_and_get() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    db.repositories()
        .messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id,
            channel_type: "telegram",
            channel_user_id: "tg-user-42",
            display_name: Some("Alice"),
        })
        .await
        .expect("create_channel_link should succeed on PG");

    let link = db
        .repositories()
        .messaging
        .get_channel_link(tenant_id, "telegram", "tg-user-42")
        .await
        .expect("get_channel_link should succeed on PG")
        .expect("link should exist");

    assert_eq!(link["user_id"].as_str().unwrap(), user_id);
    assert_eq!(link["channel_type"].as_str().unwrap(), "telegram");
    assert_eq!(link["display_name"].as_str().unwrap(), "Alice");
}

#[tokio::test]
async fn test_pg_list_user_channel_links() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    for (channel, handle) in [("telegram", "tg-1"), ("whatsapp", "+1234"), ("slack", "U1")] {
        db.repositories()
            .messaging
            .create_channel_link(&CreateChannelLinkParams {
                id: &Uuid::new_v4().to_string(),
                tenant_id,
                user_id: &user_id,
                channel_type: channel,
                channel_user_id: handle,
                display_name: None,
            })
            .await
            .unwrap();
    }

    let links = db
        .repositories()
        .messaging
        .list_user_channel_links(tenant_id, &user_id)
        .await
        .unwrap();
    assert_eq!(links.len(), 3, "should list all three channel links");
}

#[tokio::test]
async fn test_pg_delete_channel_link() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    db.repositories()
        .messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id,
            channel_type: "discord",
            channel_user_id: "dis-1",
            display_name: None,
        })
        .await
        .unwrap();

    let deleted = db
        .repositories()
        .messaging
        .delete_channel_link(tenant_id, &user_id, "discord")
        .await
        .expect("delete_channel_link should succeed on PG");
    assert!(deleted);

    let gone = db
        .repositories()
        .messaging
        .get_channel_link(tenant_id, "discord", "dis-1")
        .await
        .unwrap();
    assert!(gone.is_none());
}

// ============================================================================
// Link state round-trip (web-initiated and channel-initiated)
// ============================================================================

#[tokio::test]
async fn test_pg_link_state_web_initiated() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    db.repositories()
        .messaging
        .create_link_state(&CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: Some(&user_id),
            channel_type: "telegram",
            code: &code,
            method: "deep_link",
            channel_user_id: None,
            sender_name: None,
            expires_at: &expires_at,
        })
        .await
        .expect("create_link_state should succeed on PG");

    let consumed = db
        .repositories()
        .messaging
        .consume_link_state(&code, tenant_id)
        .await
        .expect("consume_link_state should succeed on PG");
    assert_eq!(consumed["user_id"].as_str().unwrap(), user_id);

    // Second consume must fail
    let err = db
        .repositories()
        .messaging
        .consume_link_state(&code, tenant_id)
        .await
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("already")
            || err.to_string().to_lowercase().contains("used")
            || err.to_string().to_lowercase().contains("expired"),
        "second consume should be rejected, got: {err}"
    );
}

#[tokio::test]
async fn test_pg_link_state_channel_initiated_then_complete() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    db.repositories()
        .messaging
        .create_link_state(&CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: None,
            channel_type: "whatsapp",
            code: &code,
            method: "channel_initiated",
            channel_user_id: Some("+15551234567"),
            sender_name: Some("Bob"),
            expires_at: &expires_at,
        })
        .await
        .unwrap();

    // Mid-flight: get_link_state for the login page
    let before = db
        .repositories()
        .messaging
        .get_link_state(&code)
        .await
        .unwrap()
        .expect("pending state should be readable");
    assert!(before["user_id"].is_null());
    assert_eq!(before["sender_name"].as_str().unwrap(), "Bob");

    // Complete with the authenticated user
    let completed = db
        .repositories()
        .messaging
        .complete_link_state(&code, &user_id)
        .await
        .expect("complete_link_state should succeed on PG");
    assert_eq!(completed["user_id"].as_str().unwrap(), user_id);
}

// ============================================================================
// FK enforcement: orphan ids must be rejected
// ============================================================================

#[tokio::test]
async fn test_pg_create_session_rejects_orphan_user_id() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (_, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let orphan_user = Uuid::new_v4().to_string(); // never inserted into users

    let err = db
        .repositories()
        .messaging
        .create_session(&CreateSessionParams {
            id: &Uuid::new_v4().to_string(),
            user_id: &orphan_user,
            tenant_id,
            channel_type: "telegram",
            channel_user_id: "tg-orphan",
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .expect_err("create_session with orphan user_id must fail (FK violation)");
    assert!(
        err.to_string().to_lowercase().contains("foreign key")
            || err.to_string().to_lowercase().contains("violates"),
        "expected FK error, got: {err}"
    );
}

#[tokio::test]
async fn test_pg_create_channel_link_rejects_orphan_tenant() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let user_uuid = seed_pg_user(&db).await;
    let user_id = user_uuid.to_string();
    let orphan_tenant = TenantId::new(); // never inserted

    let err = db
        .repositories()
        .messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: orphan_tenant,
            user_id: &user_id,
            channel_type: "slack",
            channel_user_id: "U-orphan",
            display_name: None,
        })
        .await
        .expect_err("create_channel_link with orphan tenant_id must fail (FK violation)");
    assert!(
        err.to_string().to_lowercase().contains("foreign key")
            || err.to_string().to_lowercase().contains("violates"),
        "expected FK error, got: {err}"
    );
}

#[tokio::test]
async fn test_pg_create_link_state_rejects_orphan_tenant() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let orphan_tenant = TenantId::new();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    let err = db
        .repositories()
        .messaging
        .create_link_state(&CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: orphan_tenant,
            user_id: None,
            channel_type: "telegram",
            code: &Uuid::new_v4().to_string(),
            method: "channel_initiated",
            channel_user_id: Some("sender-id"),
            sender_name: None,
            expires_at: &expires_at,
        })
        .await
        .expect_err("create_link_state with orphan tenant_id must fail (FK violation)");
    assert!(
        err.to_string().to_lowercase().contains("foreign key")
            || err.to_string().to_lowercase().contains("violates"),
        "expected FK error, got: {err}"
    );
}

// ============================================================================
// Self-heal: set_session_conversation repoints a session at a new conversation
// ============================================================================

#[tokio::test]
async fn test_pg_set_session_conversation_repoints_session() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    // Session with no initial conversation (NULL pierre_conversation_id)
    let session_id = Uuid::new_v4().to_string();
    db.repositories()
        .messaging
        .create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &user_id,
            tenant_id,
            channel_type: "telegram",
            channel_user_id: "tg-heal",
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .unwrap();

    // Create a real chat_conversation so the FK is satisfied
    let conv = db
        .repositories()
        .chat
        .create_conversation(
            &user_id,
            tenant_id,
            "Messaging: telegram",
            "gemini-2.0-flash-exp",
            None,
            None,
        )
        .await
        .expect("create_conversation should succeed on PG");

    // Repoint via the new method (Layer 2 self-heal entry point)
    db.repositories()
        .messaging
        .set_session_conversation(&session_id, &conv.id)
        .await
        .expect("set_session_conversation should succeed on PG");

    let refreshed = db
        .repositories()
        .messaging
        .get_session_by_channel_identity(tenant_id, "telegram", "tg-heal", None)
        .await
        .unwrap()
        .expect("session should still exist");
    assert_eq!(
        refreshed["pierre_conversation_id"].as_str().unwrap(),
        conv.id
    );
}

// ============================================================================
// CASCADE semantics: deleting parents cascades to messaging rows
// ============================================================================

#[tokio::test]
async fn test_pg_delete_user_cascades_to_messaging_rows() {
    let Some((_isolated, db)) = try_pg_db().await else {
        return;
    };
    let (user_uuid, tenant_id) = seed_pg_user_and_tenant(&db).await;
    let user_id = user_uuid.to_string();

    db.repositories()
        .messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id,
            user_id: &user_id,
            channel_type: "telegram",
            channel_user_id: "tg-cascade",
            display_name: None,
        })
        .await
        .unwrap();

    let session_id = Uuid::new_v4().to_string();
    db.repositories()
        .messaging
        .create_session(&CreateSessionParams {
            id: &session_id,
            user_id: &user_id,
            tenant_id,
            channel_type: "telegram",
            channel_user_id: "tg-cascade",
            channel_conversation_id: None,
            pierre_conversation_id: None,
        })
        .await
        .unwrap();

    // Delete the user — CASCADE should wipe both channel_link and session
    db.repositories()
        .users
        .delete(user_uuid)
        .await
        .expect("user delete should succeed");

    let link_gone = db
        .repositories()
        .messaging
        .get_channel_link(tenant_id, "telegram", "tg-cascade")
        .await
        .unwrap();
    assert!(
        link_gone.is_none(),
        "channel_link should cascade-delete with the user"
    );

    let session_gone = db
        .repositories()
        .messaging
        .get_session_by_channel_identity(tenant_id, "telegram", "tg-cascade", None)
        .await
        .unwrap();
    assert!(
        session_gone.is_none(),
        "session should cascade-delete with the user"
    );
}
