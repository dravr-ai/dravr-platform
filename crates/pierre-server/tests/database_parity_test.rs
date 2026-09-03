// ABOUTME: Database parity tests ensuring SQLite and PostgreSQL implementations behave identically
// ABOUTME: Tests that both database backends return equivalent results for Tool Selection and Chat
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `SQLite` and `PostgreSQL` database parity tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::recipes::{IngredientUnit, Recipe, RecipeIngredient};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{
    A2AClient, A2AUsage, ApiKey, ApiKeyTier, ApiKeyUsage, DataSource, DeviceType,
    StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
};
use pierre_core::models::{Tenant, TenantId, TenantPlan, ToolCategory, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::database::test_utils::{create_sqlite_test_db, create_test_db};
use pierre_database::{
    backends::factory::Database, database::AddMessageParams, repositories::SyncCursorRow,
    repository_registry::RepositoryRegistry,
};
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Tool Selection Parity Tests
// ============================================================================

/// Test that both `SQLite` and `PostgreSQL` return the same tool catalog
#[tokio::test]
async fn test_parity_tool_catalog() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    let sqlite_catalog = sqlite_repos
        .tool_selection
        .get_tool_catalog()
        .await
        .expect("SQLite: Failed to get catalog");

    let pg_catalog = pg_repos
        .tool_selection
        .get_tool_catalog()
        .await
        .expect("PostgreSQL: Failed to get catalog");

    // Both should return the same number of tools
    assert_eq!(
        sqlite_catalog.len(),
        pg_catalog.len(),
        "Tool catalog count should match: SQLite={}, PostgreSQL={}",
        sqlite_catalog.len(),
        pg_catalog.len()
    );

    // Compare tool names (both sorted for deterministic comparison)
    let mut sqlite_names: Vec<_> = sqlite_catalog.iter().map(|t| &t.tool_name).collect();
    let mut pg_names: Vec<_> = pg_catalog.iter().map(|t| &t.tool_name).collect();
    sqlite_names.sort();
    pg_names.sort();

    assert_eq!(
        sqlite_names, pg_names,
        "Tool names should match between SQLite and PostgreSQL"
    );
}

/// Test that both backends return the same tool entry by name
#[tokio::test]
async fn test_parity_get_tool_catalog_entry() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    let tool_name = "get_activities";

    let sqlite_entry = sqlite_repos
        .tool_selection
        .get_tool_catalog_entry(tool_name)
        .await
        .expect("SQLite: Failed to get entry");

    let pg_entry = pg_repos
        .tool_selection
        .get_tool_catalog_entry(tool_name)
        .await
        .expect("PostgreSQL: Failed to get entry");

    // Both should find the tool
    assert!(sqlite_entry.is_some(), "SQLite should find {tool_name}");
    assert!(pg_entry.is_some(), "PostgreSQL should find {tool_name}");

    let sqlite_entry = sqlite_entry.unwrap();
    let pg_entry = pg_entry.unwrap();

    // Compare key fields
    assert_eq!(
        sqlite_entry.tool_name, pg_entry.tool_name,
        "Tool name should match"
    );
    assert_eq!(
        sqlite_entry.display_name, pg_entry.display_name,
        "Display name should match"
    );
    assert_eq!(
        sqlite_entry.category, pg_entry.category,
        "Category should match"
    );
    assert_eq!(
        sqlite_entry.min_plan, pg_entry.min_plan,
        "Min plan should match"
    );
    assert_eq!(
        sqlite_entry.is_enabled_by_default, pg_entry.is_enabled_by_default,
        "Enabled by default should match"
    );
}

/// Test that both filter by category the same way
#[tokio::test]
async fn test_parity_tools_by_category() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    for category in [
        ToolCategory::Fitness,
        ToolCategory::Analysis,
        ToolCategory::Nutrition,
        ToolCategory::Configuration,
        ToolCategory::Coaches,
        ToolCategory::Admin,
        ToolCategory::Mobility,
    ] {
        let sqlite_tools = sqlite_repos
            .tool_selection
            .get_tools_by_category(category)
            .await
            .expect("SQLite: Failed to get tools by category");

        let pg_tools = pg_repos
            .tool_selection
            .get_tools_by_category(category)
            .await
            .expect("PostgreSQL: Failed to get tools by category");

        assert_eq!(
            sqlite_tools.len(),
            pg_tools.len(),
            "Category {:?} tool count should match: SQLite={}, PostgreSQL={}",
            category,
            sqlite_tools.len(),
            pg_tools.len()
        );
    }
}

/// Test that both filter by plan the same way
#[tokio::test]
async fn test_parity_tools_by_min_plan() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    for plan in [
        TenantPlan::Starter,
        TenantPlan::Professional,
        TenantPlan::Enterprise,
    ] {
        let sqlite_tools = sqlite_repos
            .tool_selection
            .get_tools_by_min_plan(plan)
            .await
            .expect("SQLite: Failed to get tools by plan");

        let pg_tools = pg_repos
            .tool_selection
            .get_tools_by_min_plan(plan)
            .await
            .expect("PostgreSQL: Failed to get tools by plan");

        assert_eq!(
            sqlite_tools.len(),
            pg_tools.len(),
            "Plan {:?} tool count should match: SQLite={}, PostgreSQL={}",
            plan,
            sqlite_tools.len(),
            pg_tools.len()
        );
    }
}

/// Test that tenant tool override operations behave identically
#[tokio::test]
async fn test_parity_tenant_tool_overrides() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    // Create users with tenants in both databases
    let (sqlite_user_id, sqlite_tenant_id) = create_test_user(&sqlite_repos).await;
    let (pg_user_id, pg_tenant_id) = create_test_user(&pg_repos).await;

    // Both should start with empty overrides
    let sqlite_overrides = sqlite_repos
        .tool_selection
        .get_overrides(sqlite_tenant_id)
        .await
        .expect("SQLite: Failed to get overrides");
    let pg_overrides = pg_repos
        .tool_selection
        .get_overrides(pg_tenant_id)
        .await
        .expect("PostgreSQL: Failed to get overrides");

    assert!(
        sqlite_overrides.is_empty(),
        "SQLite should have no overrides"
    );
    assert!(
        pg_overrides.is_empty(),
        "PostgreSQL should have no overrides"
    );

    // Create same override in both
    let sqlite_created = sqlite_repos
        .tool_selection
        .upsert_override(
            sqlite_tenant_id,
            "get_activities",
            false,
            Some(sqlite_user_id),
            Some("Test reason".to_owned()),
        )
        .await
        .expect("SQLite: Failed to create override");

    let pg_created = pg_repos
        .tool_selection
        .upsert_override(
            pg_tenant_id,
            "get_activities",
            false,
            Some(pg_user_id),
            Some("Test reason".to_owned()),
        )
        .await
        .expect("PostgreSQL: Failed to create override");

    // Verify same tool_name and is_enabled (upsert succeeded via expect() above)
    assert_eq!(sqlite_created.tool_name, pg_created.tool_name);
    assert_eq!(sqlite_created.is_enabled, pg_created.is_enabled);

    // Delete in both
    let sqlite_deleted = sqlite_repos
        .tool_selection
        .delete_override(sqlite_tenant_id, "get_activities")
        .await
        .expect("SQLite: Failed to delete override");
    let pg_deleted = pg_repos
        .tool_selection
        .delete_override(pg_tenant_id, "get_activities")
        .await
        .expect("PostgreSQL: Failed to delete override");

    assert!(sqlite_deleted, "SQLite delete should return true");
    assert!(pg_deleted, "PostgreSQL delete should return true");
}

// ============================================================================
// Chat Parity Tests
// ============================================================================

/// Test that conversation creation behaves identically
#[tokio::test]
async fn test_parity_chat_create_conversation() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    let (sqlite_user_id, sqlite_tenant_id) = create_test_user(&sqlite_repos).await;
    let (pg_user_id, pg_tenant_id) = create_test_user(&pg_repos).await;

    // Parity check: both backends round-trip a NULL coach_id identically.
    // Full coach-attached flows are covered by chat_routes_test and the
    // orchestration integration tests which seed a coaches row first.
    let sqlite_conv = sqlite_repos
        .chat
        .create_conversation(
            &sqlite_user_id.to_string(),
            sqlite_tenant_id,
            "Test Chat",
            "gpt-4",
            None,
            None,
        )
        .await
        .expect("SQLite: Failed to create conversation");

    let pg_conv = pg_repos
        .chat
        .create_conversation(
            &pg_user_id.to_string(),
            pg_tenant_id,
            "Test Chat",
            "gpt-4",
            None,
            None,
        )
        .await
        .expect("PostgreSQL: Failed to create conversation");

    // Compare structure (IDs will differ)
    assert_eq!(sqlite_conv.title, pg_conv.title, "Titles should match");
    assert_eq!(sqlite_conv.model, pg_conv.model, "Models should match");
    assert_eq!(
        sqlite_conv.coach_id, pg_conv.coach_id,
        "Coach IDs should match (both None when no coach attached)"
    );
    assert_eq!(
        sqlite_conv.total_tokens, pg_conv.total_tokens,
        "Token counts should match"
    );
}

/// `chat_conversations.channel_type` must read back identically on both
/// backends.
///
/// The column is `TEXT NOT NULL DEFAULT 'web'` and is now load-bearing: the
/// backfill push reads it to tell a first-party conversation (deliverable by
/// persisting a turn) from a messaging thread whose session was reset
/// (undeliverable). A column-name or decode divergence here would make the PG
/// tier either fail the read or answer the wrong verdict, and
/// `create_test_server_resources` hard-codes `SQLite`, so this file is the only
/// seam that exercises the PG path at all.
#[tokio::test]
async fn test_parity_chat_conversation_channel_type() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    for (repos, label) in [(&sqlite_repos, "SQLite"), (&pg_repos, "PostgreSQL")] {
        let (user_id, tenant_id) = create_test_user(repos).await;
        let user = user_id.to_string();
        let conv = repos
            .chat
            .create_conversation(&user, tenant_id, "Channel Test", "gpt-4", None, None)
            .await
            .unwrap_or_else(|e| panic!("{label}: create_conversation failed: {e}"));

        // The INSERT never names the column, so the returned record must carry
        // the schema default rather than a value the row does not hold.
        assert_eq!(conv.channel_type, "web", "{label}: fresh conversation");
        let read_back = repos
            .chat
            .get_conversation(&conv.id, &user, tenant_id)
            .await
            .unwrap_or_else(|e| panic!("{label}: get_conversation failed: {e}"))
            .unwrap_or_else(|| panic!("{label}: conversation vanished"));
        assert_eq!(read_back.channel_type, "web", "{label}: read back");

        repos
            .chat
            .set_conversation_channel(&conv.id, &user, tenant_id, "telegram")
            .await
            .unwrap_or_else(|e| panic!("{label}: set_conversation_channel failed: {e}"));
        let stamped = repos
            .chat
            .get_conversation(&conv.id, &user, tenant_id)
            .await
            .unwrap_or_else(|e| panic!("{label}: get_conversation failed: {e}"))
            .unwrap_or_else(|| panic!("{label}: conversation vanished"));
        assert_eq!(
            stamped.channel_type, "telegram",
            "{label}: a stamped channel must survive the round-trip"
        );
    }
}

/// Test that message operations behave identically
#[tokio::test]
async fn test_parity_chat_messages() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    let (sqlite_user_id, sqlite_tenant_id) = create_test_user(&sqlite_repos).await;
    let (pg_user_id, pg_tenant_id) = create_test_user(&pg_repos).await;

    // Create conversations
    let sqlite_conv = sqlite_repos
        .chat
        .create_conversation(
            &sqlite_user_id.to_string(),
            sqlite_tenant_id,
            "Message Test",
            "gpt-4",
            None,
            None,
        )
        .await
        .expect("SQLite: Failed to create conversation");

    let pg_conv = pg_repos
        .chat
        .create_conversation(
            &pg_user_id.to_string(),
            pg_tenant_id,
            "Message Test",
            "gpt-4",
            None,
            None,
        )
        .await
        .expect("PostgreSQL: Failed to create conversation");

    // Add same messages to both
    let messages = vec![
        ("user", "Hello!", None, None),
        ("assistant", "Hi there!", Some(10u32), Some("stop")),
        ("user", "How are you?", None, None),
    ];

    let sqlite_uid = sqlite_user_id.to_string();
    let pg_uid = pg_user_id.to_string();

    for (role, content, tokens, finish) in &messages {
        let sqlite_params = AddMessageParams {
            tenant_id: sqlite_tenant_id,
            conversation_id: &sqlite_conv.id,
            user_id: &sqlite_uid,
            role,
            content,
            token_count: *tokens,
            finish_reason: *finish,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        };
        sqlite_repos
            .chat
            .add_message(&sqlite_params)
            .await
            .expect("SQLite: Failed to add message");

        let pg_params = AddMessageParams {
            tenant_id: pg_tenant_id,
            conversation_id: &pg_conv.id,
            user_id: &pg_uid,
            role,
            content,
            token_count: *tokens,
            finish_reason: *finish,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        };
        pg_repos
            .chat
            .add_message(&pg_params)
            .await
            .expect("PostgreSQL: Failed to add message");
    }

    // Get all messages
    let sqlite_messages = sqlite_repos
        .chat
        .get_messages(&sqlite_conv.id, &sqlite_uid, sqlite_tenant_id)
        .await
        .expect("SQLite: Failed to get messages");

    let pg_messages = pg_repos
        .chat
        .get_messages(&pg_conv.id, &pg_uid, pg_tenant_id)
        .await
        .expect("PostgreSQL: Failed to get messages");

    assert_eq!(
        sqlite_messages.len(),
        pg_messages.len(),
        "Message count should match"
    );

    // Compare message content
    for (sqlite_msg, pg_msg) in sqlite_messages.iter().zip(pg_messages.iter()) {
        assert_eq!(sqlite_msg.role, pg_msg.role, "Roles should match");
        assert_eq!(sqlite_msg.content, pg_msg.content, "Content should match");
        assert_eq!(
            sqlite_msg.token_count, pg_msg.token_count,
            "Token counts should match"
        );
        assert_eq!(
            sqlite_msg.finish_reason, pg_msg.finish_reason,
            "Finish reasons should match"
        );
    }

    // Compare message counts
    let sqlite_count = sqlite_repos
        .chat
        .get_message_count(&sqlite_conv.id, &sqlite_uid, sqlite_tenant_id)
        .await
        .expect("SQLite: Failed to get count");

    let pg_count = pg_repos
        .chat
        .get_message_count(&pg_conv.id, &pg_uid, pg_tenant_id)
        .await
        .expect("PostgreSQL: Failed to get count");

    assert_eq!(sqlite_count, pg_count, "Message counts should match");
}

/// Test that listing conversations behaves identically
#[tokio::test]
async fn test_parity_chat_list_conversations() {
    let (sqlite_db, pg_db) = create_both_databases().await;
    let sqlite_repos = sqlite_db.repositories();
    let pg_repos = pg_db.repositories();

    let (sqlite_user_id, sqlite_tenant_id) = create_test_user(&sqlite_repos).await;
    let (pg_user_id, pg_tenant_id) = create_test_user(&pg_repos).await;

    // Create same conversations in both
    for i in 1..=5 {
        sqlite_repos
            .chat
            .create_conversation(
                &sqlite_user_id.to_string(),
                sqlite_tenant_id,
                &format!("Chat {i}"),
                "gpt-4",
                None,
                None,
            )
            .await
            .expect("SQLite: Failed to create conversation");

        pg_repos
            .chat
            .create_conversation(
                &pg_user_id.to_string(),
                pg_tenant_id,
                &format!("Chat {i}"),
                "gpt-4",
                None,
                None,
            )
            .await
            .expect("PostgreSQL: Failed to create conversation");
    }

    // Test pagination works the same
    let sqlite_list = sqlite_repos
        .chat
        .list_conversations(&sqlite_user_id.to_string(), sqlite_tenant_id, 3, 0)
        .await
        .expect("SQLite: Failed to list")
        .items;

    let pg_list = pg_repos
        .chat
        .list_conversations(&pg_user_id.to_string(), pg_tenant_id, 3, 0)
        .await
        .expect("PostgreSQL: Failed to list")
        .items;

    assert_eq!(
        sqlite_list.len(),
        pg_list.len(),
        "Pagination should return same count"
    );

    // Test delete all works the same
    let sqlite_deleted = sqlite_repos
        .chat
        .delete_all_user_conversations(&sqlite_user_id.to_string(), sqlite_tenant_id)
        .await
        .expect("SQLite: Failed to delete all");

    let pg_deleted = pg_repos
        .chat
        .delete_all_user_conversations(&pg_user_id.to_string(), pg_tenant_id)
        .await
        .expect("PostgreSQL: Failed to delete all");

    assert_eq!(
        sqlite_deleted, pg_deleted,
        "Delete all should remove same count"
    );
}

// ============================================================================
// Sync Cursor Parity Tests
// ============================================================================

/// Truncate to microseconds so timestamps survive the `PostgreSQL` TIMESTAMPTZ
/// round-trip (micro precision) unchanged; `SQLite` stores full-precision
/// RFC 3339 TEXT.
fn micros(ts: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(ts.timestamp_micros()).expect("in-range timestamp")
}

/// Regression: `sync_state.retry_count` is INTEGER (INT4) on `PostgreSQL` while
/// the row struct carried `i64` — sqlx's strict PG decode rejected the read and
/// `Row::get` panicked inside `get_sync_cursor`. The panic was unreachable
/// until the cursor *write* path was fixed (TEXT vs TIMESTAMPTZ binds); the
/// first row ever written then made every chat-pipeline refresh for that user
/// panic (dev outage 2026-07-05, correlation ids 7df0a1b1 / 69807284). Pins
/// the full write-then-read cycle on both backends so field/DDL type drift
/// fails here instead of in production.
#[tokio::test]
async fn test_parity_sync_cursor_roundtrip() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, tenant_id) = create_test_user(&repos).await;

        let last_sync_at = micros(Utc::now());
        let next_retry_at = micros(Utc::now() + Duration::minutes(15));
        let cursor = SyncCursorRow {
            id: format!("{user_id}:{tenant_id}:whoop:recovery"),
            user_id: user_id.to_string(),
            tenant_id: tenant_id.to_string(),
            provider: "whoop".to_owned(),
            data_type: "recovery".to_owned(),
            cursor_value: Some("cursor-token-abc".to_owned()),
            last_sync_at: Some(last_sync_at),
            last_sync_status: "failed".to_owned(),
            records_synced: 42,
            error_message: Some("provider returned 503".to_owned()),
            retry_count: 3,
            next_retry_at: Some(next_retry_at),
        };

        repos
            .sync_cursors
            .upsert_sync_cursor(&cursor)
            .await
            .unwrap_or_else(|e| panic!("{backend}: cursor upsert must succeed: {e}"));

        // The exact call the chat pipeline's refresh stage makes. Before the
        // fix this panicked on PostgreSQL: retry_count decoded as i64 from INT4.
        let read = repos
            .sync_cursors
            .get_sync_cursor(&user_id.to_string(), &tenant_id, "whoop", "recovery")
            .await
            .unwrap_or_else(|e| panic!("{backend}: get_sync_cursor must not error: {e}"))
            .unwrap_or_else(|| panic!("{backend}: upserted cursor must be found"));

        assert_eq!(
            read.retry_count, 3,
            "{backend}: INT4 retry_count must round-trip"
        );
        assert_eq!(read.records_synced, 42, "{backend}: records_synced");
        assert_eq!(
            read.last_sync_at,
            Some(last_sync_at),
            "{backend}: last_sync_at"
        );
        assert_eq!(
            read.next_retry_at,
            Some(next_retry_at),
            "{backend}: next_retry_at"
        );
        assert_eq!(
            read.cursor_value.as_deref(),
            Some("cursor-token-abc"),
            "{backend}: cursor_value"
        );
        assert_eq!(read.last_sync_status, "failed", "{backend}: status");
        assert_eq!(
            read.error_message.as_deref(),
            Some("provider returned 503"),
            "{backend}: error_message"
        );
        assert_eq!(read.user_id, user_id.to_string(), "{backend}: user_id");
        assert_eq!(
            read.tenant_id,
            tenant_id.to_string(),
            "{backend}: tenant_id"
        );

        // Upsert again with bumped counters to pin the ON CONFLICT update path.
        let bumped = SyncCursorRow {
            retry_count: 4,
            records_synced: 99,
            error_message: None,
            ..cursor
        };
        repos
            .sync_cursors
            .upsert_sync_cursor(&bumped)
            .await
            .unwrap_or_else(|e| panic!("{backend}: conflict-update upsert must succeed: {e}"));

        let read = repos
            .sync_cursors
            .get_sync_cursor(&user_id.to_string(), &tenant_id, "whoop", "recovery")
            .await
            .unwrap_or_else(|e| panic!("{backend}: re-read must not error: {e}"))
            .unwrap_or_else(|| panic!("{backend}: updated cursor must be found"));
        assert_eq!(
            read.retry_count, 4,
            "{backend}: conflict-update must persist retry_count"
        );
        assert_eq!(read.records_synced, 99, "{backend}: updated records_synced");
        assert_eq!(read.error_message, None, "{backend}: cleared error_message");
    }
}

/// `user_onboarding` write-then-read parity: durable onboarding step state must
/// round-trip identically on both backends, including the nullable
/// `chosen_channel` column, upsert-in-place on the `(user_id, step_id)` key, and
/// per-`user_id` isolation. All columns are TEXT by design, so this pins that
/// choice (no INT4/native-uuid decode surprise) against a real `PostgreSQL`.
#[tokio::test]
async fn test_parity_user_onboarding() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, _tenant_id) = create_test_user(&repos).await;
        let uid = user_id.to_string();

        // A plain step, plus the messaging-channel step carrying a chosen_channel
        // (exercises the nullable TEXT column on both backends).
        repos
            .user_onboarding
            .set_onboarding_step(&uid, "profile_type", "complete", None, Some("tenant-abc"))
            .await
            .unwrap_or_else(|e| panic!("{backend}: set profile_type must succeed: {e}"));
        repos
            .user_onboarding
            .set_onboarding_step(
                &uid,
                "messaging_channel",
                "complete",
                Some("telegram"),
                Some("tenant-abc"),
            )
            .await
            .unwrap_or_else(|e| panic!("{backend}: set messaging_channel must succeed: {e}"));

        // Re-mark profile_type to prove the upsert overwrites in place (PK conflict).
        repos
            .user_onboarding
            .set_onboarding_step(&uid, "profile_type", "skipped", None, None)
            .await
            .unwrap_or_else(|e| panic!("{backend}: re-set profile_type must succeed: {e}"));

        let mut steps = repos
            .user_onboarding
            .get_onboarding_steps(&uid)
            .await
            .unwrap_or_else(|e| panic!("{backend}: get_onboarding_steps must not error: {e}"));
        steps.sort_by(|a, b| a.step_id.cmp(&b.step_id));

        assert_eq!(
            steps.len(),
            2,
            "{backend}: two distinct steps (upsert overwrote, not inserted)"
        );

        let messaging = steps
            .iter()
            .find(|s| s.step_id == "messaging_channel")
            .unwrap_or_else(|| panic!("{backend}: messaging_channel row must be found"));
        assert_eq!(messaging.status, "complete", "{backend}: messaging status");
        assert_eq!(
            messaging.chosen_channel.as_deref(),
            Some("telegram"),
            "{backend}: chosen_channel round-trips"
        );

        let profile = steps
            .iter()
            .find(|s| s.step_id == "profile_type")
            .unwrap_or_else(|| panic!("{backend}: profile_type row must be found"));
        assert_eq!(
            profile.status, "skipped",
            "{backend}: upsert overwrote status in place"
        );
        assert_eq!(
            profile.chosen_channel, None,
            "{backend}: NULL chosen_channel round-trips as None"
        );

        // Isolation: a different user_id (never inserted) sees none of these rows.
        let other_user = Uuid::new_v4().to_string();
        let other = repos
            .user_onboarding
            .get_onboarding_steps(&other_user)
            .await
            .unwrap_or_else(|e| panic!("{backend}: get for other user must not error: {e}"));
        assert!(
            other.is_empty(),
            "{backend}: onboarding steps are scoped per user_id"
        );
    }
}

// ============================================================================
// Health Data Parity Tests (INT4 decode regressions)
// ============================================================================

/// Seed a `data_sources` row so health rows can reference it (the
/// `data_source_id` columns carry a foreign key on `PostgreSQL`).
async fn seed_data_source(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
) -> String {
    let source = DataSource {
        id: format!("ds-parity-{user_id}"),
        user_id: user_id.to_string(),
        provider: "whoop".to_owned(),
        device_model: None,
        software_version: None,
        source: None,
        device_type: DeviceType::Ring,
        original_source_name: None,
    };
    repos
        .data_sources
        .upsert_data_source(&tenant_id, &source)
        .await
        .expect("data source upsert must succeed")
}

/// Regression: `upsert_data_source` generated a fresh UUID and returned it even
/// when ON CONFLICT kept the existing row (no RETURNING), and the identity key
/// treated NULL `device_model`/`source` as distinct so provider-level sources
/// (no device metadata) never conflicted at all — every sync minted a new row
/// and health records were stamped with phantom ids that violated the
/// `data_source_id` foreign keys.
#[tokio::test]
async fn test_parity_data_source_upsert_stable_id() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, tenant_id) = create_test_user(&repos).await;

        // Provider-level source: empty id (store generates one), no device metadata.
        let source = DataSource {
            id: String::new(),
            user_id: user_id.to_string(),
            provider: "whoop".to_owned(),
            device_model: None,
            software_version: None,
            source: None,
            device_type: DeviceType::Unknown,
            original_source_name: Some("whoop".to_owned()),
        };

        let first = repos
            .data_sources
            .upsert_data_source(&tenant_id, &source)
            .await
            .expect("first upsert must succeed");
        assert!(!first.is_empty(), "{backend}: upsert must return an id");

        let second = repos
            .data_sources
            .upsert_data_source(&tenant_id, &source)
            .await
            .expect("second upsert must succeed");
        assert_eq!(
            first, second,
            "{backend}: re-upserting the same provider-level source must return the existing id"
        );

        // The returned id must satisfy the sleep_sessions.data_source_id FK —
        // this is the exact write that failed 100% of WHOOP health syncs.
        let now = chrono::Utc::now();
        let session = StoredSleepSession {
            id: format!("sleep-fk-{user_id}"),
            user_id: user_id.to_string(),
            data_source_id: first.clone(),
            is_nap: false,
            start_datetime: now - chrono::Duration::hours(8),
            end_datetime: now,
            total_sleep_seconds: Some(25_200),
            deep_sleep_seconds: Some(5_000),
            light_sleep_seconds: Some(12_000),
            rem_sleep_seconds: Some(8_200),
            awake_seconds: Some(1_800),
            sleep_efficiency: Some(93.0),
            avg_heart_rate: Some(52.0),
            min_heart_rate: Some(45),
            avg_hrv: Some(65.0),
            sleep_score: Some(88),
            stages: Vec::new(),
            source_name: "whoop".to_owned(),
        };
        repos
            .sleep
            .upsert_sleep_session(&tenant_id, &session)
            .await
            .expect("sleep upsert with the returned data_source_id must satisfy the FK");
    }
}

/// Regression companion to `test_parity_sync_cursor_roundtrip`:
/// `recovery_metrics.resting_heart_rate` is INTEGER (INT4) on `PostgreSQL` and
/// was decoded as `i64`, so any non-NULL value panicked the read path
/// (`Row::get` unwraps the mismatched-width decode).
#[tokio::test]
async fn test_parity_recovery_metrics_roundtrip() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, tenant_id) = create_test_user(&repos).await;
        let data_source_id = seed_data_source(&repos, user_id, tenant_id).await;

        let metrics = StoredRecoveryMetrics {
            id: String::new(),
            user_id: user_id.to_string(),
            data_source_id,
            date: Utc::now().date_naive(),
            recovery_score: Some(85),
            readiness_score: Some(72),
            hrv_ms: None,
            hrv_rmssd: None,
            resting_heart_rate: Some(48),
            stress_score: Some(20),
            body_battery: None,
            spo2: None,
            respiratory_rate: Some(14.5),
            skin_temp_deviation: Some(0.3),
            source_name: "whoop".to_owned(),
            recorded_at: Utc::now(),
        };
        repos
            .recovery
            .upsert_recovery_metrics(&tenant_id, &metrics)
            .await
            .unwrap_or_else(|e| panic!("{backend}: recovery upsert must succeed: {e}"));

        let read = repos
            .recovery
            .get_latest_recovery(user_id, &tenant_id)
            .await
            .unwrap_or_else(|e| panic!("{backend}: get_latest_recovery must not error: {e}"))
            .unwrap_or_else(|| panic!("{backend}: upserted recovery metrics must be found"));

        assert_eq!(
            read.resting_heart_rate,
            Some(48),
            "{backend}: INT4 resting_heart_rate must round-trip"
        );
        assert_eq!(read.recovery_score, Some(85), "{backend}: recovery_score");
        assert_eq!(read.readiness_score, Some(72), "{backend}: readiness_score");
        assert_eq!(read.stress_score, Some(20), "{backend}: stress_score");
        assert_eq!(
            read.respiratory_rate,
            Some(14.5),
            "{backend}: respiratory_rate"
        );
        assert_eq!(
            read.skin_temp_deviation,
            Some(0.3),
            "{backend}: skin_temp_deviation"
        );
        assert_eq!(read.user_id, user_id.to_string(), "{backend}: user_id");
        assert_eq!(read.source_name, "whoop", "{backend}: source_name");
    }
}

/// The WHOOP shape: a provider id that is CONSTANT across dates.
///
/// `test_parity_health_snapshot_roundtrip` seeds `id: String::new()`, which
/// selects the one branch that could never fail — an empty id always minted a
/// fresh UUID, so the primary key could not collide. The real defect needed a
/// NON-EMPTY id repeating across two dates: dravr-enforme stamps
/// `whoop-body-{user}` for every date while the conflict arbiter keys on
/// `(user, tenant, provider, date)`, so the second date repeated the primary key
/// and the INSERT died before the arbiter was ever consulted. 108 failures in 14
/// days on dev, and WHOOP sync stuck permanently in `SyncStatus::Failed`.
///
/// Fixed by minting a surrogate id and reading the stored one back with
/// `RETURNING id`. This test is red against the old code and green against the
/// fix, on both backends.
#[tokio::test]
async fn test_parity_health_snapshot_constant_provider_id_across_dates() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, tenant_id) = create_test_user(&repos).await;
        let data_source_id = seed_data_source(&repos, user_id, tenant_id).await;

        // Captured once so a midnight rollover mid-test cannot make the two
        // upserts share a date and silently stop testing the collision.
        let today = Utc::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        let provider_id = format!("whoop-body-{user_id}");

        let make = |date, weight| StoredHealthMetrics {
            id: provider_id.clone(),
            user_id: user_id.to_string(),
            data_source_id: data_source_id.clone(),
            date,
            weight_kg: Some(weight),
            body_fat_pct: None,
            muscle_mass_kg: None,
            bmi: None,
            bone_mass_kg: None,
            water_pct: None,
            systolic_bp: None,
            diastolic_bp: None,
            blood_glucose: None,
            source_name: "whoop".to_owned(),
            recorded_at: Utc::now(),
        };

        let first = repos
            .health_snapshots
            .upsert_health_snapshot(&tenant_id, &make(yesterday, 70.0))
            .await
            .unwrap_or_else(|e| panic!("{backend}: first upsert must succeed: {e}"));

        // The assertion the old code failed: a DIFFERENT date carrying the SAME
        // provider id must land, not violate health_snapshots_pkey.
        let second = repos
            .health_snapshots
            .upsert_health_snapshot(&tenant_id, &make(today, 71.0))
            .await
            .unwrap_or_else(|e| {
                panic!("{backend}: a constant provider id on a new date must not collide: {e}")
            });

        assert_ne!(
            first, second,
            "{backend}: two dates are two rows, so they must carry distinct store-assigned ids"
        );

        // Re-syncing the same date updates in place and reports the SAME id —
        // this is what lets legacy rows heal without a backfill.
        let resynced = repos
            .health_snapshots
            .upsert_health_snapshot(&tenant_id, &make(today, 72.0))
            .await
            .unwrap_or_else(|e| panic!("{backend}: same-date re-sync must succeed: {e}"));
        assert_eq!(
            resynced, second,
            "{backend}: re-syncing a date must return the pre-existing row id, not mint a new one"
        );

        let latest = repos
            .health_snapshots
            .get_latest_health_snapshot(user_id, &tenant_id)
            .await
            .unwrap_or_else(|e| panic!("{backend}: get_latest must not error: {e}"))
            .unwrap_or_else(|| panic!("{backend}: a snapshot must be found"));
        assert_eq!(
            latest.weight_kg,
            Some(72.0),
            "{backend}: the re-sync must have overwritten the weight in place"
        );
    }
}

/// Same INT4 regression class for `health_snapshots.bp_systolic` /
/// `bp_diastolic`, which were decoded as `i64` and panicked when non-NULL.
#[tokio::test]
async fn test_parity_health_snapshot_roundtrip() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, tenant_id) = create_test_user(&repos).await;
        let data_source_id = seed_data_source(&repos, user_id, tenant_id).await;

        let snapshot = StoredHealthMetrics {
            id: String::new(),
            user_id: user_id.to_string(),
            data_source_id,
            date: Utc::now().date_naive(),
            weight_kg: Some(70.5),
            body_fat_pct: Some(12.5),
            muscle_mass_kg: None,
            bmi: None,
            bone_mass_kg: None,
            water_pct: None,
            systolic_bp: Some(120),
            diastolic_bp: Some(80),
            blood_glucose: Some(5.2),
            source_name: "whoop".to_owned(),
            recorded_at: Utc::now(),
        };
        repos
            .health_snapshots
            .upsert_health_snapshot(&tenant_id, &snapshot)
            .await
            .unwrap_or_else(|e| panic!("{backend}: snapshot upsert must succeed: {e}"));

        let read = repos
            .health_snapshots
            .get_latest_health_snapshot(user_id, &tenant_id)
            .await
            .unwrap_or_else(|e| panic!("{backend}: get_latest_health_snapshot must not error: {e}"))
            .unwrap_or_else(|| panic!("{backend}: upserted snapshot must be found"));

        assert_eq!(
            read.systolic_bp,
            Some(120),
            "{backend}: INT4 bp_systolic must round-trip"
        );
        assert_eq!(
            read.diastolic_bp,
            Some(80),
            "{backend}: INT4 bp_diastolic must round-trip"
        );
        assert_eq!(read.weight_kg, Some(70.5), "{backend}: weight_kg");
        assert_eq!(read.body_fat_pct, Some(12.5), "{backend}: body_fat_pct");
        assert_eq!(read.blood_glucose, Some(5.2), "{backend}: blood_glucose");
        assert_eq!(read.user_id, user_id.to_string(), "{backend}: user_id");
    }
}

/// Same INT4 regression class for `recipe_ingredients.fdc_id`: the domain
/// struct (dravr-cageux) types it `Option<i64>`, and an untyped `row.get`
/// decoded it as `i64` from the INTEGER column — panicking whenever a recipe
/// with a USDA-validated ingredient was read on `PostgreSQL`.
#[tokio::test]
async fn test_parity_recipe_ingredient_fdc_id_roundtrip() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, tenant_id) = create_test_user(&repos).await;

        let mut recipe = Recipe::new(user_id, "Parity Oat Bowl", 2);
        recipe.ingredients = vec![RecipeIngredient {
            fdc_id: Some(171_705),
            name: "Rolled oats".to_owned(),
            amount: 100.0,
            unit: IngredientUnit::Grams,
            grams: 100.0,
            preparation: None,
        }];
        recipe.instructions = vec!["Combine and serve.".to_owned()];

        let recipe_id = repos
            .recipes
            .create(user_id, tenant_id, &recipe)
            .await
            .unwrap_or_else(|e| panic!("{backend}: recipe create must succeed: {e}"));

        let read = repos
            .recipes
            .get_by_id(&recipe_id, user_id, tenant_id)
            .await
            .unwrap_or_else(|e| panic!("{backend}: recipe read must not error: {e}"))
            .unwrap_or_else(|| panic!("{backend}: created recipe must be found"));

        assert_eq!(read.ingredients.len(), 1, "{backend}: ingredient count");
        assert_eq!(
            read.ingredients[0].fdc_id,
            Some(171_705),
            "{backend}: INT4 fdc_id must round-trip"
        );
        assert_eq!(read.ingredients[0].name, "Rolled oats", "{backend}: name");
    }
}

/// `get_top_tools_analysis` averages `response_time_ms` (INTEGER): PG's
/// `AVG(int)` yields NUMERIC, which an `f64` `try_get(..).ok()` silently
/// dropped to `None` — so `average_response_time` was always 0 on `PostgreSQL`.
/// The query now casts to DOUBLE PRECISION; this pins the real average (and
/// the BIGINT count decodes) on both backends.
#[tokio::test]
async fn test_parity_api_key_top_tools_analysis() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, _tenant_id) = create_test_user(&repos).await;

        let api_key = ApiKey {
            id: Uuid::new_v4().to_string(),
            user_id,
            name: "parity-key".to_owned(),
            key_prefix: "pk_parity".to_owned(),
            key_hash: "parity-hash".to_owned(),
            description: None,
            tier: ApiKeyTier::Starter,
            rate_limit_requests: 1000,
            rate_limit_window_seconds: 3600,
            is_active: true,
            last_used_at: None,
            expires_at: None,
            created_at: Utc::now(),
        };
        repos
            .api_keys
            .create(&api_key)
            .await
            .unwrap_or_else(|e| panic!("{backend}: api key create must succeed: {e}"));

        for (response_time_ms, status_code) in [(90, 200), (100, 200), (110, 500)] {
            let usage = ApiKeyUsage {
                id: None,
                api_key_id: api_key.id.clone(),
                timestamp: Utc::now(),
                tool_name: "get_activities".to_owned(),
                response_time_ms: Some(response_time_ms),
                status_code,
                error_message: None,
                request_size_bytes: None,
                response_size_bytes: None,
                ip_address: None,
                user_agent: None,
            };
            repos
                .usage
                .record_api_key(&usage)
                .await
                .unwrap_or_else(|e| panic!("{backend}: usage record must succeed: {e}"));
        }

        let tools = repos
            .usage
            .get_top_tools_analysis(
                user_id,
                Utc::now() - Duration::hours(1),
                Utc::now() + Duration::hours(1),
            )
            .await
            .unwrap_or_else(|e| panic!("{backend}: top tools analysis must not error: {e}"));

        assert_eq!(tools.len(), 1, "{backend}: one tool expected");
        let tool = &tools[0];
        assert_eq!(tool.tool_name, "get_activities", "{backend}: tool_name");
        assert_eq!(tool.request_count, 3, "{backend}: request_count");
        assert!(
            (tool.average_response_time - 100.0).abs() < 1e-6,
            "{backend}: average_response_time must be the real mean, got {}",
            tool.average_response_time
        );
        assert!(
            (tool.success_rate - 200.0 / 3.0).abs() < 1e-6,
            "{backend}: success_rate 66.67% expected, got {}",
            tool.success_rate
        );
    }
}

/// `get_api_key_stats` aggregates `AVG(response_time_ms)` over the INTEGER
/// column, so PG returns NUMERIC. The `query_as` tuple decodes that slot as
/// `f64`, which sqlx rejects for NUMERIC — the whole call errored on PG until
/// the `::DOUBLE PRECISION` cast landed (the sibling `get_top_tools_analysis`
/// dropped it to a silent 0 via `try_get().ok()`; this path failed loud).
#[tokio::test]
async fn test_parity_api_key_stats() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, _tenant_id) = create_test_user(&repos).await;

        let api_key = ApiKey {
            id: Uuid::new_v4().to_string(),
            user_id,
            name: "parity-key".to_owned(),
            key_prefix: "pk_parity".to_owned(),
            key_hash: "parity-hash".to_owned(),
            description: None,
            tier: ApiKeyTier::Starter,
            rate_limit_requests: 1000,
            rate_limit_window_seconds: 3600,
            is_active: true,
            last_used_at: None,
            expires_at: None,
            created_at: Utc::now(),
        };
        repos
            .api_keys
            .create(&api_key)
            .await
            .unwrap_or_else(|e| panic!("{backend}: api key create must succeed: {e}"));

        for (response_time_ms, status_code) in [(90, 200), (100, 200), (110, 500)] {
            let usage = ApiKeyUsage {
                id: None,
                api_key_id: api_key.id.clone(),
                timestamp: Utc::now(),
                tool_name: "get_activities".to_owned(),
                response_time_ms: Some(response_time_ms),
                status_code,
                error_message: None,
                request_size_bytes: None,
                response_size_bytes: None,
                ip_address: None,
                user_agent: None,
            };
            repos
                .usage
                .record_api_key(&usage)
                .await
                .unwrap_or_else(|e| panic!("{backend}: usage record must succeed: {e}"));
        }

        let stats = repos
            .usage
            .get_api_key_stats(
                &api_key.id,
                Utc::now() - Duration::hours(1),
                Utc::now() + Duration::hours(1),
            )
            .await
            .unwrap_or_else(|e| panic!("{backend}: api key stats must not error: {e}"));

        assert_eq!(stats.total_requests, 3, "{backend}: total_requests");
        assert_eq!(
            stats.successful_requests, 2,
            "{backend}: successful_requests"
        );
        assert_eq!(stats.failed_requests, 1, "{backend}: failed_requests");
        assert_eq!(
            stats.total_response_time_ms, 300,
            "{backend}: total_response_time_ms"
        );

        let avg = stats.tool_usage["get_activities"]["avg_response_time_ms"]
            .as_f64()
            .unwrap_or_else(|| panic!("{backend}: avg_response_time_ms must be a number"));
        assert!(
            (avg - 100.0).abs() < 1e-6,
            "{backend}: avg_response_time_ms must be the real mean, got {avg}"
        );
    }
}

/// `get_usage_stats` aggregates `AVG(response_time_ms)` over the INTEGER column
/// (NUMERIC on PG), then `try_get::<Option<f64>>` — which sqlx rejects for
/// NUMERIC — so every call failed at decode on PG until the `::DOUBLE
/// PRECISION` cast landed. Mirrors `test_parity_api_key_stats` for the A2A path.
#[tokio::test]
async fn test_parity_a2a_usage_stats() {
    let (sqlite_db, pg_db) = create_both_databases().await;

    for (backend, db) in [("SQLite", &sqlite_db), ("PostgreSQL", &pg_db)] {
        let repos = db.repositories();
        let (user_id, _tenant_id) = create_test_user(&repos).await;

        let api_key = ApiKey {
            id: Uuid::new_v4().to_string(),
            user_id,
            name: "parity-a2a-key".to_owned(),
            key_prefix: "pk_a2a".to_owned(),
            key_hash: "parity-a2a-hash".to_owned(),
            description: None,
            tier: ApiKeyTier::Starter,
            rate_limit_requests: 1000,
            rate_limit_window_seconds: 3600,
            is_active: true,
            last_used_at: None,
            expires_at: None,
            created_at: Utc::now(),
        };
        repos
            .api_keys
            .create(&api_key)
            .await
            .unwrap_or_else(|e| panic!("{backend}: api key create must succeed: {e}"));

        let client = A2AClient {
            id: Uuid::new_v4().to_string(),
            name: "parity-client".to_owned(),
            description: "parity a2a client".to_owned(),
            public_key: "parity-public-key".to_owned(),
            user_id,
            capabilities: vec!["fitness-data-analysis".to_owned()],
            redirect_uris: vec!["https://test.example.com".to_owned()],
            permissions: vec!["read_activities".to_owned()],
            rate_limit_requests: 1000,
            rate_limit_window_seconds: 3600,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        repos
            .a2a
            .create_client(&client, "parity-secret", &api_key.id)
            .await
            .unwrap_or_else(|e| panic!("{backend}: a2a client create must succeed: {e}"));

        for (response_time_ms, status_code) in [(90, 200), (100, 200), (110, 500)] {
            let usage = A2AUsage {
                id: None,
                client_id: client.id.clone(),
                session_token: None,
                timestamp: Utc::now(),
                tool_name: "analyze".to_owned(),
                response_time_ms: Some(response_time_ms),
                status_code,
                error_message: None,
                request_size_bytes: Some(256),
                response_size_bytes: Some(512),
                ip_address: None,
                user_agent: None,
                protocol_version: "1.0".to_owned(),
                client_capabilities: vec!["analysis".to_owned()],
                granted_scopes: vec!["read".to_owned()],
            };
            repos
                .a2a
                .record_usage(&usage)
                .await
                .unwrap_or_else(|e| panic!("{backend}: a2a usage record must succeed: {e}"));
        }

        let stats = repos
            .a2a
            .get_usage_stats(
                &client.id,
                Utc::now() - Duration::hours(1),
                Utc::now() + Duration::hours(1),
            )
            .await
            .unwrap_or_else(|e| panic!("{backend}: a2a usage stats must not error: {e}"));

        assert_eq!(stats.total_requests, 3, "{backend}: total_requests");
        assert_eq!(
            stats.successful_requests, 2,
            "{backend}: successful_requests"
        );
        assert_eq!(stats.failed_requests, 1, "{backend}: failed_requests");
        assert_eq!(
            stats.avg_response_time_ms,
            Some(100),
            "{backend}: avg_response_time_ms must be the real mean"
        );
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create both `SQLite` and `PostgreSQL` test databases.
///
/// The `SQLite` side is opened explicitly — this file compares the two
/// dialects, so it is the one place a `PostgreSQL`-lane test opens `SQLite`
/// on purpose. The `PostgreSQL` side comes from the factory, which honours
/// `DATABASE_URL` and refuses `SQLite` on the lane that requires it.
async fn create_both_databases() -> (Arc<Database>, Arc<Database>) {
    let sqlite_db = create_sqlite_test_db()
        .await
        .expect("Failed to create SQLite test database");
    let pg_db = create_test_db()
        .await
        .expect("Failed to create PostgreSQL test database");
    (Arc::new(sqlite_db), Arc::new(pg_db))
}

/// Create a test user with an associated tenant.
///
/// Creates the user, then a tenant owned by that user, then calls
/// `update_tenant_id` to link them via the `tenant_users` junction table.
/// The tenant must exist before `update_tenant_id` because `tenant_users`
/// has a foreign key on `tenants(id)`.
async fn create_test_user(repos: &RepositoryRegistry) -> (Uuid, TenantId) {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("parity-test-{user_id}@example.com"),
        display_name: Some("Parity Test User".to_owned()),
        password_hash: "test_hash".to_owned(),
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

    repos
        .users
        .create(&user)
        .await
        .expect("Failed to create user");

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Parity Test Tenant for {user_id}"),
        slug: format!("parity-test-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    repos
        .tenants
        .create(&tenant)
        .await
        .expect("Failed to create tenant");

    repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .expect("Failed to link user to tenant via tenant_users");

    (user_id, tenant_id)
}
