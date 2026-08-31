// ABOUTME: Unit tests for the chat database module
// ABOUTME: Tests conversation and message CRUD operations with multi-tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap (valid in tests per CLAUDE.md guidelines)
#![allow(missing_docs, clippy::unwrap_used)]

use chrono::Utc;
use pierre_core::models::coaches::{CoachCategory, CoachVisibility, CreateSystemCoachRequest};
use pierre_core::models::groups::{CoachingGroup, GroupRespondMode};
use pierre_core::models::{
    AddMessageParams, CoachingPersona, ParticipantRole, Tenant, TenantId,
    UpsertMessageFeedbackParams, User, UserStatus, UserTier,
};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::ChatRepository;
use std::sync::Arc;
use uuid::Uuid;

/// Deterministic tenant ID for tests (fixed bytes representing "tenant-1")
fn test_tenant_id() -> TenantId {
    TenantId::from_uuid(Uuid::from_bytes([
        0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
        0x01,
    ]))
}

/// Second deterministic tenant ID for multi-tenant isolation tests (fixed bytes representing "tenant-2")
fn test_tenant_id_2() -> TenantId {
    TenantId::from_uuid(Uuid::from_bytes([
        0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02, 0x02,
        0x02,
    ]))
}

/// One isolated database and the three users the tests address by role:
/// the athlete owns most threads, the member is added to them (or refused
/// from them as a stranger), and the other owner holds a thread the athlete
/// only joins. Each is a real `users` row, because `chat_conversations`,
/// `conversation_participants` and `coaching_groups` reference `users(id)`
/// on `PostgreSQL`; [`test_tenant_id`] is a real `tenants` row owned by the
/// athlete, because `coaches.tenant_id` references it on `SQLite`.
struct ChatFixture {
    db: Database,
    athlete: String,
    member: String,
    other_owner: String,
}

impl ChatFixture {
    fn chat(&self) -> Arc<dyn ChatRepository> {
        self.db.repositories().chat
    }

    fn athlete(&self) -> &str {
        &self.athlete
    }

    fn member(&self) -> &str {
        &self.member
    }

    fn other_owner(&self) -> &str {
        &self.other_owner
    }
}

/// Open an isolated database through the factory and seed the three users
/// and the tenant they share.
async fn open_fixture() -> ChatFixture {
    let db = create_test_db().await.unwrap();
    let athlete = seed_user(&db, "athlete").await;
    let member = seed_user(&db, "member").await;
    let other_owner = seed_user(&db, "other-owner").await;
    let tenant_id = test_tenant_id();
    let mut tenant = Tenant::new(
        "Chat Test Tenant".to_owned(),
        format!("chat-test-{tenant_id}"),
        None,
        "starter".to_owned(),
        uuid_of(&athlete),
    );
    tenant.id = tenant_id;
    db.repositories().tenants.create(&tenant).await.unwrap();
    ChatFixture {
        db,
        athlete,
        member,
        other_owner,
    }
}

/// Create a user row and return its id in the string form the chat API takes.
async fn seed_user(db: &Database, role: &str) -> String {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("{role}-{user_id}@example.com"),
        display_name: Some(format!("Chat {role}")),
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
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    db.repositories().users.create(&user).await.unwrap();
    user_id.to_string()
}

/// The `Uuid` behind a seeded user's string id, for the APIs that take one.
fn uuid_of(user_id: &str) -> Uuid {
    Uuid::parse_str(user_id).unwrap()
}

/// Append one row through the manager, as the pipeline does.
async fn add_row(
    manager: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    role: &str,
    content: &str,
) -> String {
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id,
            user_id,
            role,
            content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn test_list_rows_carry_coach_group_preview_and_unread() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();
    let repos = fx.db.repositories();

    // The list joins the coach and the group a conversation names, for the
    // row's title/handle and group name.
    let coach_id = repos
        .coaches
        .create_system_coach(
            uuid_of(fx.athlete()),
            tenant_id,
            &CreateSystemCoachRequest {
                title: "Recovery Coach".to_owned(),
                description: Some("Rest and recovery".to_owned()),
                system_prompt: "You are the recovery coach.".to_owned(),
                category: CoachCategory::Recovery,
                tags: vec![],
                visibility: CoachVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    let handle = repos
        .store_listings
        .assign_catalogue_handle(&coach_id, tenant_id)
        .await
        .unwrap();
    assert_eq!(handle, "recovery-coach");
    let group_id = repos
        .groups
        .create_group(
            tenant_id,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant_id.to_string(),
                name: "Marathon Squad".to_owned(),
                description: None,
                coach_id: coach_id.clone(),
                owner_id: uuid_of(fx.athlete()),
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap()
        .id
        .to_string();

    let plain = manager
        .create_conversation(fx.athlete(), tenant_id, "Plain", "m", None, None)
        .await
        .unwrap();
    let grouped = manager
        .create_conversation(fx.athlete(), tenant_id, "Squad", "m", None, Some(&group_id))
        .await
        .unwrap();
    let coached = manager
        .create_conversation(fx.athlete(), tenant_id, "Coach", "m", Some(&coach_id), None)
        .await
        .unwrap();
    add_row(
        manager.as_ref(),
        &coached.id,
        fx.athlete(),
        tenant_id,
        "user",
        "salut",
    )
    .await;
    add_row(
        manager.as_ref(),
        &coached.id,
        fx.athlete(),
        tenant_id,
        "tool_result",
        "<tool_result/>",
    )
    .await;
    let last = add_row(
        manager.as_ref(),
        &coached.id,
        fx.athlete(),
        tenant_id,
        "assistant",
        "Repos demain.",
    )
    .await;

    let page = manager
        .list_conversations(fx.athlete(), tenant_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(page.total, 3);
    let ids: Vec<&str> = page.items.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [coached.id.as_str(), grouped.id.as_str(), plain.id.as_str()],
        "newest activity first: the coached thread just received rows"
    );

    let coached_row = &page.items[0];
    assert_eq!(coached_row.coach_handle.as_deref(), Some("recovery-coach"));
    assert_eq!(coached_row.coach_title.as_deref(), Some("Recovery Coach"));
    assert_eq!(coached_row.message_count, 2, "tool rows are not turns");
    assert_eq!(coached_row.unread_count, 2, "nothing read yet");
    let newest = coached_row.last_message.as_ref().unwrap();
    assert_eq!(newest.role, "assistant");
    assert_eq!(newest.content_head, "Repos demain.");
    assert!(!newest.created_at.is_empty());

    let group_row = &page.items[1];
    assert_eq!(group_row.group_id.as_deref(), Some(group_id.as_str()));
    assert_eq!(group_row.group_name.as_deref(), Some("Marathon Squad"));
    assert_eq!(group_row.last_message, None);
    assert_eq!(group_row.unread_count, 0);
    assert_eq!(group_row.message_count, 0);

    // Reading up to the newest row clears the badge; the marker then holds.
    assert!(manager
        .mark_conversation_read(&coached.id, fx.athlete(), tenant_id, Some(&last))
        .await
        .unwrap());
    let page = manager
        .list_conversations(fx.athlete(), tenant_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(page.items[0].unread_count, 0);
}

/// The unread count `user` sees on their first listed thread.
async fn unread_of(manager: &dyn ChatRepository, tenant_id: TenantId, user: &str) -> i64 {
    manager
        .list_conversations(user, tenant_id, 10, 0)
        .await
        .unwrap()
        .items[0]
        .unread_count
}

#[tokio::test]
async fn test_read_marker_is_monotonic_and_membership_gated() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(fx.athlete(), tenant_id, "Marker", "m", None, None)
        .await
        .unwrap();
    // An empty thread: a participant marks nothing and is still answered.
    assert!(manager
        .mark_conversation_read(&conv.id, fx.athlete(), tenant_id, None)
        .await
        .unwrap());
    let first = add_row(
        manager.as_ref(),
        &conv.id,
        fx.athlete(),
        tenant_id,
        "user",
        "one",
    )
    .await;
    let second = add_row(
        manager.as_ref(),
        &conv.id,
        fx.athlete(),
        tenant_id,
        "assistant",
        "two",
    )
    .await;
    let third = add_row(
        manager.as_ref(),
        &conv.id,
        fx.athlete(),
        tenant_id,
        "assistant",
        "three",
    )
    .await;

    assert_eq!(
        unread_of(manager.as_ref(), tenant_id, fx.athlete()).await,
        3
    );

    assert!(manager
        .mark_conversation_read(&conv.id, fx.athlete(), tenant_id, Some(&second))
        .await
        .unwrap());
    assert_eq!(
        unread_of(manager.as_ref(), tenant_id, fx.athlete()).await,
        1,
        "only the row after the marker"
    );

    // Never backwards: re-marking the first row leaves the marker on the second.
    assert!(manager
        .mark_conversation_read(&conv.id, fx.athlete(), tenant_id, Some(&first))
        .await
        .unwrap());
    assert_eq!(
        unread_of(manager.as_ref(), tenant_id, fx.athlete()).await,
        1
    );

    // `None` means the newest row.
    assert!(manager
        .mark_conversation_read(&conv.id, fx.athlete(), tenant_id, None)
        .await
        .unwrap());
    assert_eq!(
        unread_of(manager.as_ref(), tenant_id, fx.athlete()).await,
        0
    );

    // Mark unread: every turn counts again.
    assert!(manager
        .clear_conversation_read_marker(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap());
    assert_eq!(
        unread_of(manager.as_ref(), tenant_id, fx.athlete()).await,
        3
    );
    assert!(manager
        .mark_conversation_read(&conv.id, fx.athlete(), tenant_id, Some(&third))
        .await
        .unwrap());
    assert_eq!(
        unread_of(manager.as_ref(), tenant_id, fx.athlete()).await,
        0
    );

    // A message outside the thread, a stranger, and a stranger's clear are all refused.
    assert!(!manager
        .mark_conversation_read(&conv.id, fx.athlete(), tenant_id, Some("not-a-row"))
        .await
        .unwrap());
    assert!(!manager
        .mark_conversation_read(&conv.id, fx.member(), tenant_id, None)
        .await
        .unwrap());
    assert!(!manager
        .clear_conversation_read_marker(&conv.id, fx.member(), tenant_id)
        .await
        .unwrap());

    // A member added later keeps their own marker.
    manager
        .add_participant(&conv.id, tenant_id, fx.member(), fx.athlete())
        .await
        .unwrap();
    let member_page = manager
        .list_conversations(fx.member(), tenant_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(member_page.items[0].unread_count, 3);
    assert_eq!(
        unread_of(manager.as_ref(), tenant_id, fx.athlete()).await,
        0,
        "the owner's marker is untouched"
    );
}

// ============================================================================
// Conversation Tests
// ============================================================================

#[tokio::test]
async fn test_create_conversation() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    assert!(!conv.id.is_empty());
    assert_eq!(conv.user_id, fx.athlete());
    assert_eq!(conv.tenant_id, tenant_id.to_string());
    assert_eq!(conv.title, "Test Chat");
    assert_eq!(conv.model, "gemini-1.5-flash");
    assert!(conv.coach_id.is_none());
    assert_eq!(conv.total_tokens, 0);
}

#[tokio::test]
async fn test_create_conversation_with_coach_id() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    // Pass None and assert the plumbing preserves the NULL. A coach-attached
    // conversation is exercised by test_list_rows_carry_coach_group_preview_and_unread
    // and the route-level and orchestration integration tests.
    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Fitness Chat",
            "gemini-1.5-pro",
            None,
            None,
        )
        .await
        .unwrap();

    assert!(conv.coach_id.is_none());
    assert_eq!(conv.model, "gemini-1.5-pro");
}

#[tokio::test]
async fn test_get_conversation() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let created = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let fetched = manager
        .get_conversation(&created.id, fx.athlete(), tenant_id)
        .await
        .unwrap();

    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.title, "Test Chat");
}

#[tokio::test]
async fn test_get_conversation_tenant_isolation() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let different_tenant = test_tenant_id_2();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    // Try to access from different tenant - should return None
    let result = manager
        .get_conversation(&conv.id, fx.athlete(), different_tenant)
        .await
        .unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_list_conversations() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    // Create multiple conversations
    manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Chat 1",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Chat 2",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Chat 3",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let list = manager
        .list_conversations(fx.athlete(), tenant_id, 10, 0)
        .await
        .unwrap()
        .items;

    assert_eq!(list.len(), 3);
}

#[tokio::test]
async fn test_list_conversations_pagination() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    // Create multiple conversations
    for i in 1..=5 {
        manager
            .create_conversation(
                fx.athlete(),
                tenant_id,
                &format!("Chat {i}"),
                "gemini-1.5-flash",
                None,
                None,
            )
            .await
            .unwrap();
    }

    // Get first 2
    let page1 = manager
        .list_conversations(fx.athlete(), tenant_id, 2, 0)
        .await
        .unwrap()
        .items;
    assert_eq!(page1.len(), 2);

    // Get next 2
    let page2 = manager
        .list_conversations(fx.athlete(), tenant_id, 2, 2)
        .await
        .unwrap()
        .items;
    assert_eq!(page2.len(), 2);

    // Get remaining
    let page3 = manager
        .list_conversations(fx.athlete(), tenant_id, 2, 4)
        .await
        .unwrap()
        .items;
    assert_eq!(page3.len(), 1);
}

#[tokio::test]
async fn test_update_conversation_title() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Original Title",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let updated = manager
        .update_conversation_title(&conv.id, fx.athlete(), tenant_id, "New Title")
        .await
        .unwrap();

    assert!(updated);

    let fetched = manager
        .get_conversation(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.title, "New Title");
}

#[tokio::test]
async fn test_set_conversation_channel_surfaces_in_list() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Messaging: telegram",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    // Before stamping, the column holds the 'web' default.
    let before = manager
        .list_conversations(fx.athlete(), tenant_id, 10, 0)
        .await
        .unwrap()
        .items;
    assert_eq!(
        before
            .iter()
            .find(|s| s.id == conv.id)
            .unwrap()
            .channel_type
            .as_deref(),
        Some("web"),
    );

    // Stamp the messaging channel, as messaging-ingress does when forging the
    // session conversation.
    assert!(manager
        .set_conversation_channel(&conv.id, fx.athlete(), tenant_id, "telegram")
        .await
        .unwrap());

    // The durable channel now surfaces in the list for the badge.
    let after = manager
        .list_conversations(fx.athlete(), tenant_id, 10, 0)
        .await
        .unwrap()
        .items;
    assert_eq!(
        after
            .iter()
            .find(|s| s.id == conv.id)
            .unwrap()
            .channel_type
            .as_deref(),
        Some("telegram"),
    );
}

#[tokio::test]
async fn test_delete_conversation() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "To Delete",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let deleted = manager
        .delete_conversation(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();

    assert!(deleted);

    let fetched = manager
        .get_conversation(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();

    assert!(fetched.is_none());
}

// ============================================================================
// Message Tests
// ============================================================================

#[tokio::test]
async fn test_add_message() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let msg = manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "user",
            content: "Hello, world!",
            token_count: Some(5),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    assert!(!msg.id.is_empty());
    assert_eq!(msg.conversation_id, conv.id);
    assert_eq!(msg.role, "user");
    assert_eq!(msg.content, "Hello, world!");
    assert_eq!(msg.token_count, Some(5));
}

#[tokio::test]
async fn test_add_assistant_message_with_finish_reason() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let msg = manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "assistant",
            content: "I'm here to help!",
            token_count: Some(10),
            finish_reason: Some("STOP"),
            prompt_tokens: Some(20),
            model: Some("gemini-1.5-flash"),
            content_blocks: None,
        })
        .await
        .unwrap();

    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.finish_reason, Some("STOP".to_owned()));
    assert_eq!(msg.prompt_tokens, Some(20));
    assert_eq!(msg.model, Some("gemini-1.5-flash".to_owned()));
}

#[tokio::test]
async fn test_get_messages() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    // Add messages
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "user",
            content: "Hello",
            token_count: Some(2),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "assistant",
            content: "Hi there!",
            token_count: Some(3),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "user",
            content: "How are you?",
            token_count: Some(4),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    let messages = manager
        .get_messages(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].content, "Hello");
    assert_eq!(messages[1].content, "Hi there!");
    assert_eq!(messages[2].content, "How are you?");
}

#[tokio::test]
async fn test_get_recent_messages() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    // Add 5 messages
    for i in 1..=5 {
        let content = format!("Message {i}");
        manager
            .add_message(&AddMessageParams {
                tenant_id,
                conversation_id: &conv.id,
                user_id: fx.athlete(),
                role: "user",
                content: &content,
                token_count: Some(2),
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
            })
            .await
            .unwrap();
    }

    // Get last 3
    let recent = manager
        .get_recent_messages(&conv.id, fx.athlete(), tenant_id, 3)
        .await
        .unwrap();

    assert_eq!(recent.len(), 3);
    // Should be in chronological order
    assert_eq!(recent[0].content, "Message 3");
    assert_eq!(recent[1].content, "Message 4");
    assert_eq!(recent[2].content, "Message 5");
}

#[tokio::test]
async fn test_message_updates_conversation_tokens() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    assert_eq!(conv.total_tokens, 0);

    // Add messages with token counts
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "user",
            content: "Hello",
            token_count: Some(10),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "assistant",
            content: "Hi!",
            token_count: Some(15),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    // Check total tokens updated
    let updated = manager
        .get_conversation(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(updated.total_tokens, 25);
}

#[tokio::test]
async fn test_get_message_count() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    // Initially 0
    let count = manager
        .get_message_count(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert_eq!(count, 0);

    // Add messages
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "user",
            content: "1",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "assistant",
            content: "2",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    let count = manager
        .get_message_count(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn test_cascade_delete_messages() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    // Add messages
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "user",
            content: "Hello",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "assistant",
            content: "Hi!",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    // Verify messages exist
    let count = manager
        .get_message_count(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert_eq!(count, 2);

    // Delete conversation (should cascade delete messages)
    manager
        .delete_conversation(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();

    // Messages should be gone (foreign key cascade)
    let messages = manager
        .get_messages(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert!(messages.is_empty());
}

#[tokio::test]
async fn test_delete_all_user_conversations() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    // Create multiple conversations
    manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Chat 1",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Chat 2",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Chat 3",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let deleted = manager
        .delete_all_user_conversations(fx.athlete(), tenant_id)
        .await
        .unwrap();

    assert_eq!(deleted, 3);

    let remaining = manager
        .list_conversations(fx.athlete(), tenant_id, 10, 0)
        .await
        .unwrap()
        .items;

    assert!(remaining.is_empty());
}

/// Persisting a `tool_call` row alongside its `tool_result` lets a follow-up
/// turn replay the grounded evidence the model already consumed. Without
/// these two roles the chat pipeline only stores the final assistant text,
/// and turn N+1 hits the "I don't have access to your Strava data"
/// refusal pattern even when turn N successfully called `get_activities`.
#[tokio::test]
async fn test_persist_tool_round_messages() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Tool Round",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "user",
            content: "Give me my last 7 activities",
            token_count: Some(6),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    // The assistant emitted only a tool call this round (no preamble text).
    // The chat pipeline persists assistant_text=None here, so we exercise
    // the path where only `tool_result` lands but `tool_call` does not.
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "tool_result",
            content: "[Tool Result for get_activities]: {\"activity_list\":\"...7 activities...\"}",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "assistant",
            content: "Here are your 7 most recent activities…",
            token_count: Some(20),
            finish_reason: Some("stop"),
            prompt_tokens: Some(150),
            model: Some("gemini-1.5-flash"),
            content_blocks: None,
        })
        .await
        .unwrap();

    let messages = manager
        .get_messages(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 3, "user → tool_result → assistant");
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "tool_result");
    assert!(messages[1].content.contains("activity_list"));
    assert_eq!(messages[2].role, "assistant");
}

/// Multi-tool round where the assistant emits a preamble alongside the call.
/// Both roles land in history so a follow-up turn replays the exact
/// `Vec<ChatMessage>` shape the in-memory tool loop produced.
#[tokio::test]
async fn test_persist_tool_round_with_assistant_preamble() {
    let fx = open_fixture().await;
    let manager = fx.chat();

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Preamble Round",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "user",
            content: "Recipe + stretching for my week",
            token_count: Some(5),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "tool_call",
            content: "Let me pull your last 7 activities first.",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.athlete(),
            role: "tool_result",
            content:
                "[Tool Result for get_activities]: {\"activity_list\":\"3 trails, 3 MTB, 1 hike\"}",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    let messages = manager
        .get_messages(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "tool_call");
    assert_eq!(messages[2].role, "tool_result");
}

// ============================================================================
// Message Feedback Tests
// ============================================================================

/// Create a conversation owned by `owner` with one assistant message and
/// return `(conversation_id, message_id)` for feedback assertions.
async fn seed_conversation_with_message(
    manager: &dyn ChatRepository,
    tenant_id: TenantId,
    owner: &str,
) -> (String, String) {
    let conv = manager
        .create_conversation(
            owner,
            tenant_id,
            "Feedback Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    let msg = manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: owner,
            role: "assistant",
            content: "Here is your plan.",
            token_count: Some(10),
            finish_reason: Some("stop"),
            prompt_tokens: Some(5),
            model: Some("gemini-1.5-flash"),
            content_blocks: None,
        })
        .await
        .unwrap();
    (conv.id, msg.id)
}

#[tokio::test]
async fn test_upsert_message_feedback_creates_then_updates_same_row() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();
    let (conversation_id, message_id) =
        seed_conversation_with_message(manager.as_ref(), tenant_id, fx.athlete()).await;

    // Create: thumbs up, no comment.
    let up = manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: fx.athlete(),
            rating: "up",
            comment: None,
        })
        .await
        .unwrap();
    assert_eq!(up.rating, "up");
    assert!(up.comment.is_none());
    assert_eq!(up.message_id, message_id);
    assert_eq!(up.conversation_id, conversation_id);

    // Switch to down with a reason — same row reused, rating + comment overwritten.
    let down = manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: fx.athlete(),
            rating: "down",
            comment: Some("too vague"),
        })
        .await
        .unwrap();
    assert_eq!(
        down.id, up.id,
        "upsert must reuse the existing feedback row"
    );
    assert_eq!(down.rating, "down");
    assert_eq!(down.comment.as_deref(), Some("too vague"));
    assert_eq!(
        down.created_at, up.created_at,
        "created_at is preserved across the update"
    );
}

#[tokio::test]
async fn test_get_conversation_feedback_returns_callers_rows() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();
    let (conversation_id, message_id) =
        seed_conversation_with_message(manager.as_ref(), tenant_id, fx.athlete()).await;

    manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: fx.athlete(),
            rating: "down",
            comment: Some("missing detail"),
        })
        .await
        .unwrap();

    let rows = manager
        .get_conversation_feedback(&conversation_id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].message_id, message_id);
    assert_eq!(rows[0].rating, "down");
    assert_eq!(rows[0].comment.as_deref(), Some("missing detail"));
}

#[tokio::test]
async fn test_delete_message_feedback_is_idempotent_toggle_off() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();
    let (conversation_id, message_id) =
        seed_conversation_with_message(manager.as_ref(), tenant_id, fx.athlete()).await;

    manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: fx.athlete(),
            rating: "up",
            comment: None,
        })
        .await
        .unwrap();

    let removed = manager
        .delete_message_feedback(&message_id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert!(removed);
    assert!(manager
        .get_conversation_feedback(&conversation_id, fx.athlete(), tenant_id)
        .await
        .unwrap()
        .is_empty());

    // Deleting again removes nothing but does not error.
    let removed_again = manager
        .delete_message_feedback(&message_id, fx.athlete(), tenant_id)
        .await
        .unwrap();
    assert!(!removed_again);
}

#[tokio::test]
async fn test_upsert_message_feedback_rejects_unowned_message() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();
    let (conversation_id, message_id) =
        seed_conversation_with_message(manager.as_ref(), tenant_id, fx.athlete()).await;

    // A different tenant cannot leave feedback on this conversation's message.
    let cross_tenant = manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id: test_tenant_id_2(),
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: fx.athlete(),
            rating: "up",
            comment: None,
        })
        .await;
    assert!(
        cross_tenant.is_err(),
        "cross-tenant feedback must be rejected"
    );

    // A forged message id (not in this conversation) is rejected too.
    let forged = manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: "does-not-exist",
            user_id: fx.athlete(),
            rating: "up",
            comment: None,
        })
        .await;
    assert!(
        forged.is_err(),
        "feedback on a non-existent message must be rejected"
    );

    // No stray rows were written by the rejected attempts.
    assert!(manager
        .get_conversation_feedback(&conversation_id, fx.athlete(), tenant_id)
        .await
        .unwrap()
        .is_empty());
}

// ============================================================================
// Participant Tests
// ============================================================================

#[tokio::test]
async fn test_create_conversation_writes_owner_participant_row() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Owned",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let participants = manager
        .list_participants(&conv.id, tenant_id)
        .await
        .unwrap();
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].user_id, fx.athlete());
    assert_eq!(participants[0].role, ParticipantRole::Owner);
    assert_eq!(participants[0].added_by, fx.athlete());
    assert_eq!(participants[0].conversation_id, conv.id);
    assert_eq!(participants[0].tenant_id, tenant_id.to_string());
}

#[tokio::test]
async fn test_added_participant_reads_and_posts_like_the_owner() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Shared",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    // Before the add: a stranger sees nothing and cannot write.
    assert!(manager
        .get_conversation(&conv.id, fx.member(), tenant_id)
        .await
        .unwrap()
        .is_none());
    assert!(manager
        .list_conversations(fx.member(), tenant_id, 10, 0)
        .await
        .unwrap()
        .items
        .is_empty());
    let refused = manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.member(),
            role: "user",
            content: "knock knock",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await;
    assert!(refused.is_err());

    let added = manager
        .add_participant(&conv.id, tenant_id, fx.member(), fx.athlete())
        .await
        .unwrap();
    assert_eq!(added.role, ParticipantRole::Member);
    assert_eq!(added.added_by, fx.athlete());

    // After: the thread is theirs to read, list, rename and post in.
    let seen = manager
        .get_conversation(&conv.id, fx.member(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        seen.user_id,
        fx.athlete(),
        "ownership is unchanged by membership"
    );

    let listed = manager
        .list_conversations(fx.member(), tenant_id, 10, 0)
        .await
        .unwrap()
        .items;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, conv.id);

    let posted = manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: fx.member(),
            role: "user",
            content: "hello from the member",
            token_count: Some(4),
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();
    assert_eq!(posted.content, "hello from the member");

    let messages = manager
        .get_messages(&conv.id, fx.member(), tenant_id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        manager
            .get_message_count(&conv.id, fx.athlete(), tenant_id)
            .await
            .unwrap(),
        1
    );
    assert!(manager
        .update_conversation_title(&conv.id, fx.member(), tenant_id, "Renamed by member")
        .await
        .unwrap());

    // Idempotent re-add keeps the existing row and never promotes to owner.
    let again = manager
        .add_participant(&conv.id, tenant_id, fx.athlete(), fx.member())
        .await
        .unwrap();
    assert_eq!(again.role, ParticipantRole::Owner);
    let participants = manager
        .list_participants(&conv.id, tenant_id)
        .await
        .unwrap();
    assert_eq!(participants.len(), 2);
    assert_eq!(
        participants[0].role,
        ParticipantRole::Owner,
        "owner sorts first"
    );
    assert_eq!(participants[1].user_id, fx.member());
}

#[tokio::test]
async fn test_removing_a_participant_revokes_access_but_never_the_owner() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Shared",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .add_participant(&conv.id, tenant_id, fx.member(), fx.athlete())
        .await
        .unwrap();

    assert!(manager
        .remove_participant(&conv.id, tenant_id, fx.member())
        .await
        .unwrap());
    assert!(manager
        .get_conversation(&conv.id, fx.member(), tenant_id)
        .await
        .unwrap()
        .is_none());
    assert!(manager
        .get_messages(&conv.id, fx.member(), tenant_id)
        .await
        .unwrap()
        .is_empty());
    // A second removal has nothing to remove.
    assert!(!manager
        .remove_participant(&conv.id, tenant_id, fx.member())
        .await
        .unwrap());

    // The owner's row is never removed, so the owner keeps the thread.
    assert!(!manager
        .remove_participant(&conv.id, tenant_id, fx.athlete())
        .await
        .unwrap());
    assert!(manager
        .get_conversation(&conv.id, fx.athlete(), tenant_id)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn test_add_participant_refuses_a_conversation_outside_the_tenant() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Mine",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let err = manager
        .add_participant(&conv.id, test_tenant_id_2(), fx.member(), fx.athlete())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Conversation not found"), "{err}");
    assert_eq!(
        manager
            .list_participants(&conv.id, tenant_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn test_count_and_delete_all_keep_owner_semantics() {
    let fx = open_fixture().await;
    let manager = fx.chat();
    let tenant_id = test_tenant_id();

    let owned = manager
        .create_conversation(
            fx.athlete(),
            tenant_id,
            "Owned",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    let joined = manager
        .create_conversation(
            fx.other_owner(),
            tenant_id,
            "Joined",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .add_participant(&joined.id, tenant_id, fx.athlete(), fx.other_owner())
        .await
        .unwrap();

    // The quota counts what the athlete opened, not what they were added to;
    // the listing shows both.
    assert_eq!(
        manager
            .count_conversations(fx.athlete(), tenant_id)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .list_conversations(fx.athlete(), tenant_id, 10, 0)
            .await
            .unwrap()
            .items
            .len(),
        2
    );

    // Account cleanup deletes the athlete's own thread and leaves the other
    // owner's thread standing.
    assert_eq!(
        manager
            .delete_all_user_conversations(fx.athlete(), tenant_id)
            .await
            .unwrap(),
        1
    );
    assert!(manager
        .get_conversation(&owned.id, fx.athlete(), tenant_id)
        .await
        .unwrap()
        .is_none());
    assert!(manager
        .get_conversation(&joined.id, fx.other_owner(), tenant_id)
        .await
        .unwrap()
        .is_some());
}
