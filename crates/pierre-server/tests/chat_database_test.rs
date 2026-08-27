// ABOUTME: Unit tests for the chat database module
// ABOUTME: Tests conversation and message CRUD operations with multi-tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap (valid in tests per CLAUDE.md guidelines)
#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::{ParticipantRole, TenantId};
use pierre_database::database::chat::{AddMessageParams, UpsertMessageFeedbackParams};
use pierre_database::database::ChatManager;
use sqlx::SqlitePool;
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

/// Create a test database with chat schema
async fn create_test_db() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    // Create users table first (for foreign key)
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            email TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            is_active INTEGER NOT NULL DEFAULT 1,
            user_status TEXT NOT NULL DEFAULT 'active',
            is_admin INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            last_active TEXT NOT NULL
        )
        ",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Create test user
    sqlx::query(
        r"
        INSERT INTO users (id, email, password_hash, created_at, last_active)
        VALUES ('user-1', 'test@example.com', 'hash', '2025-01-01', '2025-01-01'),
               ('user-2', 'member@example.com', 'hash', '2025-01-01', '2025-01-01'),
               ('user-3', 'other-owner@example.com', 'hash', '2025-01-01', '2025-01-01')
        ",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Create chat tables
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS chat_conversations (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            tenant_id TEXT NOT NULL,
            title TEXT NOT NULL,
            model TEXT NOT NULL DEFAULT 'gemini-1.5-flash',
            coach_id TEXT,
            session_id TEXT,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            group_id TEXT,
            onboarding_state TEXT,
            channel_type TEXT NOT NULL DEFAULT 'web'
        )
        ",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
            role TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool_call', 'tool_result')),
            content TEXT NOT NULL,
            token_count INTEGER,
            finish_reason TEXT,
            created_at TEXT NOT NULL,
            prompt_tokens INTEGER,
            model TEXT,
            structured_content TEXT,
            content_blocks TEXT
        )
        ",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Per-message thumbs up/down feedback (mirrors migration 20260610000001).
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS chat_message_feedback (
            id TEXT PRIMARY KEY,
            message_id TEXT NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
            conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
            user_id TEXT NOT NULL,
            tenant_id TEXT NOT NULL,
            rating TEXT NOT NULL CHECK (rating IN ('up', 'down')),
            comment TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(message_id, user_id)
        )
        ",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Who can read and post (mirrors migration 20260826000004). Every
    // membership-gated query joins on it, so the hand-rolled schema needs it
    // for any conversation read to answer at all.
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS conversation_participants (
            conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            tenant_id TEXT NOT NULL,
            role TEXT NOT NULL CHECK (role IN ('owner', 'member')),
            added_by TEXT NOT NULL,
            added_at TEXT NOT NULL,
            last_read_at TEXT,
            PRIMARY KEY (conversation_id, user_id)
        )
        ",
    )
    .execute(&pool)
    .await
    .unwrap();

    // The list joins the coach and the group a conversation names, for the
    // row's title/handle and group name. Only the columns the join reads.
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS coaches (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            slug TEXT
        )
        ",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS coaching_groups (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL
        )
        ",
    )
    .execute(&pool)
    .await
    .unwrap();

    pool
}

/// Append one row through the manager, as the pipeline does.
async fn add_row(
    manager: &ChatManager,
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
    let pool = create_test_db().await;
    sqlx::query("INSERT INTO coaches (id, title, slug) VALUES ('coach-1', 'Recovery Coach', 'recovery-coach')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO coaching_groups (id, name) VALUES ('group-1', 'Marathon Squad')")
        .execute(&pool)
        .await
        .unwrap();
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();

    let plain = manager
        .create_conversation("user-1", tenant_id, "Plain", "m", None, None)
        .await
        .unwrap();
    let grouped = manager
        .create_conversation("user-1", tenant_id, "Squad", "m", None, Some("group-1"))
        .await
        .unwrap();
    let coached = manager
        .create_conversation("user-1", tenant_id, "Coach", "m", Some("coach-1"), None)
        .await
        .unwrap();
    add_row(&manager, &coached.id, "user-1", tenant_id, "user", "salut").await;
    add_row(
        &manager,
        &coached.id,
        "user-1",
        tenant_id,
        "tool_result",
        "<tool_result/>",
    )
    .await;
    let last = add_row(
        &manager,
        &coached.id,
        "user-1",
        tenant_id,
        "assistant",
        "Repos demain.",
    )
    .await;

    let page = manager
        .list_conversations("user-1", tenant_id, 10, 0)
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
    assert_eq!(group_row.group_id.as_deref(), Some("group-1"));
    assert_eq!(group_row.group_name.as_deref(), Some("Marathon Squad"));
    assert_eq!(group_row.last_message, None);
    assert_eq!(group_row.unread_count, 0);
    assert_eq!(group_row.message_count, 0);

    // Reading up to the newest row clears the badge; the marker then holds.
    assert!(manager
        .mark_conversation_read(&coached.id, "user-1", tenant_id, Some(&last))
        .await
        .unwrap());
    let page = manager
        .list_conversations("user-1", tenant_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(page.items[0].unread_count, 0);
}

/// The unread count `user` sees on their first listed thread.
async fn unread_of(manager: &ChatManager, tenant_id: TenantId, user: &str) -> i64 {
    manager
        .list_conversations(user, tenant_id, 10, 0)
        .await
        .unwrap()
        .items[0]
        .unread_count
}

#[tokio::test]
async fn test_read_marker_is_monotonic_and_membership_gated() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation("user-1", tenant_id, "Marker", "m", None, None)
        .await
        .unwrap();
    // An empty thread: a participant marks nothing and is still answered.
    assert!(manager
        .mark_conversation_read(&conv.id, "user-1", tenant_id, None)
        .await
        .unwrap());
    let first = add_row(&manager, &conv.id, "user-1", tenant_id, "user", "one").await;
    let second = add_row(&manager, &conv.id, "user-1", tenant_id, "assistant", "two").await;
    let third = add_row(
        &manager,
        &conv.id,
        "user-1",
        tenant_id,
        "assistant",
        "three",
    )
    .await;

    assert_eq!(unread_of(&manager, tenant_id, "user-1").await, 3);

    assert!(manager
        .mark_conversation_read(&conv.id, "user-1", tenant_id, Some(&second))
        .await
        .unwrap());
    assert_eq!(
        unread_of(&manager, tenant_id, "user-1").await,
        1,
        "only the row after the marker"
    );

    // Never backwards: re-marking the first row leaves the marker on the second.
    assert!(manager
        .mark_conversation_read(&conv.id, "user-1", tenant_id, Some(&first))
        .await
        .unwrap());
    assert_eq!(unread_of(&manager, tenant_id, "user-1").await, 1);

    // `None` means the newest row.
    assert!(manager
        .mark_conversation_read(&conv.id, "user-1", tenant_id, None)
        .await
        .unwrap());
    assert_eq!(unread_of(&manager, tenant_id, "user-1").await, 0);

    // Mark unread: every turn counts again.
    assert!(manager
        .clear_conversation_read_marker(&conv.id, "user-1", tenant_id)
        .await
        .unwrap());
    assert_eq!(unread_of(&manager, tenant_id, "user-1").await, 3);
    assert!(manager
        .mark_conversation_read(&conv.id, "user-1", tenant_id, Some(&third))
        .await
        .unwrap());
    assert_eq!(unread_of(&manager, tenant_id, "user-1").await, 0);

    // A message outside the thread, a stranger, and a stranger's clear are all refused.
    assert!(!manager
        .mark_conversation_read(&conv.id, "user-1", tenant_id, Some("not-a-row"))
        .await
        .unwrap());
    assert!(!manager
        .mark_conversation_read(&conv.id, "user-2", tenant_id, None)
        .await
        .unwrap());
    assert!(!manager
        .clear_conversation_read_marker(&conv.id, "user-2", tenant_id)
        .await
        .unwrap());

    // A member added later keeps their own marker.
    manager
        .add_participant(&conv.id, tenant_id, "user-2", "user-1")
        .await
        .unwrap();
    let member_page = manager
        .list_conversations("user-2", tenant_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(member_page.items[0].unread_count, 3);
    assert_eq!(
        unread_of(&manager, tenant_id, "user-1").await,
        0,
        "the owner's marker is untouched"
    );
}

// ============================================================================
// Conversation Tests
// ============================================================================

#[tokio::test]
async fn test_create_conversation() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    assert!(!conv.id.is_empty());
    assert_eq!(conv.user_id, "user-1");
    assert_eq!(conv.tenant_id, tenant_id.to_string());
    assert_eq!(conv.title, "Test Chat");
    assert_eq!(conv.model, "gemini-1.5-flash");
    assert!(conv.coach_id.is_none());
    assert_eq!(conv.total_tokens, 0);
}

#[tokio::test]
async fn test_create_conversation_with_coach_id() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    // No coaches table in this isolated test DB; pass None and assert the plumbing
    // preserves the NULL. Full coach-attached conversation flow is exercised in the
    // route-level and orchestration integration tests.
    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let created = manager
        .create_conversation(
            "user-1",
            tenant_id,
            "Test Chat",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let fetched = manager
        .get_conversation(&created.id, "user-1", tenant_id)
        .await
        .unwrap();

    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.title, "Test Chat");
}

#[tokio::test]
async fn test_get_conversation_tenant_isolation() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let different_tenant = test_tenant_id_2();
    let conv = manager
        .create_conversation(
            "user-1",
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
        .get_conversation(&conv.id, "user-1", different_tenant)
        .await
        .unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_list_conversations() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    // Create multiple conversations
    manager
        .create_conversation(
            "user-1",
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
            "user-1",
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
            "user-1",
            tenant_id,
            "Chat 3",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let list = manager
        .list_conversations("user-1", tenant_id, 10, 0)
        .await
        .unwrap()
        .items;

    assert_eq!(list.len(), 3);
}

#[tokio::test]
async fn test_list_conversations_pagination() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    // Create multiple conversations
    for i in 1..=5 {
        manager
            .create_conversation(
                "user-1",
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
        .list_conversations("user-1", tenant_id, 2, 0)
        .await
        .unwrap()
        .items;
    assert_eq!(page1.len(), 2);

    // Get next 2
    let page2 = manager
        .list_conversations("user-1", tenant_id, 2, 2)
        .await
        .unwrap()
        .items;
    assert_eq!(page2.len(), 2);

    // Get remaining
    let page3 = manager
        .list_conversations("user-1", tenant_id, 2, 4)
        .await
        .unwrap()
        .items;
    assert_eq!(page3.len(), 1);
}

#[tokio::test]
async fn test_update_conversation_title() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
            tenant_id,
            "Original Title",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let updated = manager
        .update_conversation_title(&conv.id, "user-1", tenant_id, "New Title")
        .await
        .unwrap();

    assert!(updated);

    let fetched = manager
        .get_conversation(&conv.id, "user-1", tenant_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.title, "New Title");
}

#[tokio::test]
async fn test_set_conversation_channel_surfaces_in_list() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(
            "user-1",
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
        .list_conversations("user-1", tenant_id, 10, 0)
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
        .set_conversation_channel(&conv.id, "user-1", tenant_id, "telegram")
        .await
        .unwrap());

    // The durable channel now surfaces in the list for the badge.
    let after = manager
        .list_conversations("user-1", tenant_id, 10, 0)
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
            tenant_id,
            "To Delete",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let deleted = manager
        .delete_conversation(&conv.id, "user-1", tenant_id)
        .await
        .unwrap();

    assert!(deleted);

    let fetched = manager
        .get_conversation(&conv.id, "user-1", tenant_id)
        .await
        .unwrap();

    assert!(fetched.is_none());
}

// ============================================================================
// Message Tests
// ============================================================================

#[tokio::test]
async fn test_add_message() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
            user_id: "user-1",
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
            user_id: "user-1",
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
            user_id: "user-1",
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
            user_id: "user-1",
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
            user_id: "user-1",
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
        .get_messages(&conv.id, "user-1", tenant_id)
        .await
        .unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].content, "Hello");
    assert_eq!(messages[1].content, "Hi there!");
    assert_eq!(messages[2].content, "How are you?");
}

#[tokio::test]
async fn test_get_recent_messages() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
                user_id: "user-1",
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
        .get_recent_messages(&conv.id, "user-1", tenant_id, 3)
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
            user_id: "user-1",
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
            user_id: "user-1",
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
        .get_conversation(&conv.id, "user-1", tenant_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(updated.total_tokens, 25);
}

#[tokio::test]
async fn test_get_message_count() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
        .get_message_count(&conv.id, "user-1", tenant_id)
        .await
        .unwrap();
    assert_eq!(count, 0);

    // Add messages
    manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: "user-1",
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
            user_id: "user-1",
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
        .get_message_count(&conv.id, "user-1", tenant_id)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn test_cascade_delete_messages() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
            user_id: "user-1",
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
            user_id: "user-1",
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
        .get_message_count(&conv.id, "user-1", tenant_id)
        .await
        .unwrap();
    assert_eq!(count, 2);

    // Delete conversation (should cascade delete messages)
    manager
        .delete_conversation(&conv.id, "user-1", tenant_id)
        .await
        .unwrap();

    // Messages should be gone (foreign key cascade)
    let messages = manager
        .get_messages(&conv.id, "user-1", tenant_id)
        .await
        .unwrap();
    assert!(messages.is_empty());
}

#[tokio::test]
async fn test_delete_all_user_conversations() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    // Create multiple conversations
    manager
        .create_conversation(
            "user-1",
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
            "user-1",
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
            "user-1",
            tenant_id,
            "Chat 3",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();

    let deleted = manager
        .delete_all_user_conversations("user-1", tenant_id)
        .await
        .unwrap();

    assert_eq!(deleted, 3);

    let remaining = manager
        .list_conversations("user-1", tenant_id, 10, 0)
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
            user_id: "user-1",
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
            user_id: "user-1",
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
            user_id: "user-1",
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
        .get_messages(&conv.id, "user-1", tenant_id)
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);

    let tenant_id = test_tenant_id();
    let conv = manager
        .create_conversation(
            "user-1",
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
            user_id: "user-1",
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
            user_id: "user-1",
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
            user_id: "user-1",
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
        .get_messages(&conv.id, "user-1", tenant_id)
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

/// Create a conversation owned by `user-1` with one assistant message and
/// return `(conversation_id, message_id)` for feedback assertions.
async fn seed_conversation_with_message(
    manager: &ChatManager,
    tenant_id: TenantId,
) -> (String, String) {
    let conv = manager
        .create_conversation(
            "user-1",
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
            user_id: "user-1",
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();
    let (conversation_id, message_id) = seed_conversation_with_message(&manager, tenant_id).await;

    // Create: thumbs up, no comment.
    let up = manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: "user-1",
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
            user_id: "user-1",
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();
    let (conversation_id, message_id) = seed_conversation_with_message(&manager, tenant_id).await;

    manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: "user-1",
            rating: "down",
            comment: Some("missing detail"),
        })
        .await
        .unwrap();

    let rows = manager
        .get_conversation_feedback(&conversation_id, "user-1", tenant_id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].message_id, message_id);
    assert_eq!(rows[0].rating, "down");
    assert_eq!(rows[0].comment.as_deref(), Some("missing detail"));
}

#[tokio::test]
async fn test_delete_message_feedback_is_idempotent_toggle_off() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();
    let (conversation_id, message_id) = seed_conversation_with_message(&manager, tenant_id).await;

    manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id,
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: "user-1",
            rating: "up",
            comment: None,
        })
        .await
        .unwrap();

    let removed = manager
        .delete_message_feedback(&message_id, "user-1", tenant_id)
        .await
        .unwrap();
    assert!(removed);
    assert!(manager
        .get_conversation_feedback(&conversation_id, "user-1", tenant_id)
        .await
        .unwrap()
        .is_empty());

    // Deleting again removes nothing but does not error.
    let removed_again = manager
        .delete_message_feedback(&message_id, "user-1", tenant_id)
        .await
        .unwrap();
    assert!(!removed_again);
}

#[tokio::test]
async fn test_upsert_message_feedback_rejects_unowned_message() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();
    let (conversation_id, message_id) = seed_conversation_with_message(&manager, tenant_id).await;

    // A different tenant cannot leave feedback on this conversation's message.
    let cross_tenant = manager
        .upsert_message_feedback(&UpsertMessageFeedbackParams {
            tenant_id: test_tenant_id_2(),
            conversation_id: &conversation_id,
            message_id: &message_id,
            user_id: "user-1",
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
            user_id: "user-1",
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
        .get_conversation_feedback(&conversation_id, "user-1", tenant_id)
        .await
        .unwrap()
        .is_empty());
}

// ============================================================================
// Participant Tests
// ============================================================================

#[tokio::test]
async fn test_create_conversation_writes_owner_participant_row() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation("user-1", tenant_id, "Owned", "gemini-1.5-flash", None, None)
        .await
        .unwrap();

    let participants = manager
        .list_participants(&conv.id, tenant_id)
        .await
        .unwrap();
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].user_id, "user-1");
    assert_eq!(participants[0].role, ParticipantRole::Owner);
    assert_eq!(participants[0].added_by, "user-1");
    assert_eq!(participants[0].conversation_id, conv.id);
    assert_eq!(participants[0].tenant_id, tenant_id.to_string());
}

#[tokio::test]
async fn test_added_participant_reads_and_posts_like_the_owner() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(
            "user-1",
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
        .get_conversation(&conv.id, "user-2", tenant_id)
        .await
        .unwrap()
        .is_none());
    assert!(manager
        .list_conversations("user-2", tenant_id, 10, 0)
        .await
        .unwrap()
        .items
        .is_empty());
    let refused = manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: "user-2",
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
        .add_participant(&conv.id, tenant_id, "user-2", "user-1")
        .await
        .unwrap();
    assert_eq!(added.role, ParticipantRole::Member);
    assert_eq!(added.added_by, "user-1");

    // After: the thread is theirs to read, list, rename and post in.
    let seen = manager
        .get_conversation(&conv.id, "user-2", tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        seen.user_id, "user-1",
        "ownership is unchanged by membership"
    );

    let listed = manager
        .list_conversations("user-2", tenant_id, 10, 0)
        .await
        .unwrap()
        .items;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, conv.id);

    let posted = manager
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv.id,
            user_id: "user-2",
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
        .get_messages(&conv.id, "user-2", tenant_id)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        manager
            .get_message_count(&conv.id, "user-1", tenant_id)
            .await
            .unwrap(),
        1
    );
    assert!(manager
        .update_conversation_title(&conv.id, "user-2", tenant_id, "Renamed by member")
        .await
        .unwrap());

    // Idempotent re-add keeps the existing row and never promotes to owner.
    let again = manager
        .add_participant(&conv.id, tenant_id, "user-1", "user-2")
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
    assert_eq!(participants[1].user_id, "user-2");
}

#[tokio::test]
async fn test_removing_a_participant_revokes_access_but_never_the_owner() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation(
            "user-1",
            tenant_id,
            "Shared",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .add_participant(&conv.id, tenant_id, "user-2", "user-1")
        .await
        .unwrap();

    assert!(manager
        .remove_participant(&conv.id, tenant_id, "user-2")
        .await
        .unwrap());
    assert!(manager
        .get_conversation(&conv.id, "user-2", tenant_id)
        .await
        .unwrap()
        .is_none());
    assert!(manager
        .get_messages(&conv.id, "user-2", tenant_id)
        .await
        .unwrap()
        .is_empty());
    // A second removal has nothing to remove.
    assert!(!manager
        .remove_participant(&conv.id, tenant_id, "user-2")
        .await
        .unwrap());

    // The owner's row is never removed, so the owner keeps the thread.
    assert!(!manager
        .remove_participant(&conv.id, tenant_id, "user-1")
        .await
        .unwrap());
    assert!(manager
        .get_conversation(&conv.id, "user-1", tenant_id)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn test_add_participant_refuses_a_conversation_outside_the_tenant() {
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();

    let conv = manager
        .create_conversation("user-1", tenant_id, "Mine", "gemini-1.5-flash", None, None)
        .await
        .unwrap();

    let err = manager
        .add_participant(&conv.id, test_tenant_id_2(), "user-2", "user-1")
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
    let pool = create_test_db().await;
    let manager = ChatManager::new(pool);
    let tenant_id = test_tenant_id();

    let owned = manager
        .create_conversation("user-1", tenant_id, "Owned", "gemini-1.5-flash", None, None)
        .await
        .unwrap();
    let joined = manager
        .create_conversation(
            "user-3",
            tenant_id,
            "Joined",
            "gemini-1.5-flash",
            None,
            None,
        )
        .await
        .unwrap();
    manager
        .add_participant(&joined.id, tenant_id, "user-1", "user-3")
        .await
        .unwrap();

    // The quota counts what the athlete opened, not what they were added to;
    // the listing shows both.
    assert_eq!(
        manager
            .count_conversations("user-1", tenant_id)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        manager
            .list_conversations("user-1", tenant_id, 10, 0)
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
            .delete_all_user_conversations("user-1", tenant_id)
            .await
            .unwrap(),
        1
    );
    assert!(manager
        .get_conversation(&owned.id, "user-1", tenant_id)
        .await
        .unwrap()
        .is_none());
    assert!(manager
        .get_conversation(&joined.id, "user-3", tenant_id)
        .await
        .unwrap()
        .is_some());
}
