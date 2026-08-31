// ABOUTME: Integration tests for the Coach Notes Audit suppression flag
// ABOUTME: Proves admin POST /suppress excludes notes from chat-pipeline memory recall
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::InsertCoachNoteParams;
use pierre_memory::scope::MemoryScope;
use uuid::Uuid;

/// Open the database the lane names through the test factory.
async fn open_db() -> Result<Database> {
    Ok(create_test_db().await?)
}

/// Seed the user, tenant, and coach rows the `coach_notes` foreign keys
/// resolve against, through the repositories so both backends accept them.
async fn seed_user_tenant_coach(db: &Database) -> Result<(TenantId, String, String)> {
    let repos = db.repositories();
    let user = User::new(
        format!("{}@test.local", Uuid::new_v4()),
        "hash".to_owned(),
        Some("Test User".to_owned()),
    );
    repos.users.create(&user).await?;
    let tenant = Tenant::new(
        "Test Tenant".to_owned(),
        format!("tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user.id,
    );
    repos.tenants.create(&tenant).await?;
    let coach = repos
        .coaches
        .create(
            user.id,
            tenant.id,
            &CreateCoachRequest {
                title: "Test Coach".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
                category: CoachCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await?;
    Ok((tenant.id, user.id.to_string(), coach.id.to_string()))
}

#[tokio::test]
async fn suppressed_notes_are_excluded_from_memory_recall() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    let note = memory
        .insert_coach_note(&InsertCoachNoteParams {
            tenant_id: tenant,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: None,
            scope: MemoryScope::User,
            content: "user mentioned they want to qualify for Boston in 2027",
            embedding: None,
        })
        .await?;
    assert!(!note.suppressed);

    // Before suppression: recall returns the note.
    let before = memory
        .list_coach_notes(tenant, &user_id, &coach_id, 50)
        .await?;
    assert_eq!(before.len(), 1);

    let changed = memory
        .set_coach_note_suppressed(&note.id, tenant, true, "test-admin")
        .await?;
    assert!(changed, "first suppress should flip the flag");

    // After suppression: recall must NOT see the note.
    let after = memory
        .list_coach_notes(tenant, &user_id, &coach_id, 50)
        .await?;
    assert!(
        after.is_empty(),
        "suppressed notes must be excluded from memory recall"
    );

    // Audit listing still surfaces the row (suppressed=true) for review.
    let audit = memory.list_coach_notes_for_tenant(tenant, 50).await?;
    assert_eq!(audit.len(), 1);
    assert!(
        audit[0].suppressed,
        "audit panel must still see suppressed rows"
    );

    Ok(())
}

#[tokio::test]
async fn suppress_then_unsuppress_restores_recall() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    let note = memory
        .insert_coach_note(&InsertCoachNoteParams {
            tenant_id: tenant,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: None,
            scope: MemoryScope::User,
            content: "marathon time goal: sub-3:00",
            embedding: None,
        })
        .await?;

    memory
        .set_coach_note_suppressed(&note.id, tenant, true, "admin")
        .await?;
    let after_suppress = memory
        .list_coach_notes(tenant, &user_id, &coach_id, 50)
        .await?;
    assert!(after_suppress.is_empty());

    let changed = memory
        .set_coach_note_suppressed(&note.id, tenant, false, "admin")
        .await?;
    assert!(changed, "unsuppress should flip the flag back");

    let after_unsuppress = memory
        .list_coach_notes(tenant, &user_id, &coach_id, 50)
        .await?;
    assert_eq!(after_unsuppress.len(), 1, "recall returns the note again");
    assert!(!after_unsuppress[0].suppressed);

    Ok(())
}

#[tokio::test]
async fn set_suppressed_is_idempotent() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    let note = memory
        .insert_coach_note(&InsertCoachNoteParams {
            tenant_id: tenant,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: None,
            scope: MemoryScope::User,
            content: "needs hydration reminders before long runs",
            embedding: None,
        })
        .await?;

    let first = memory
        .set_coach_note_suppressed(&note.id, tenant, true, "admin")
        .await?;
    assert!(first);
    let second = memory
        .set_coach_note_suppressed(&note.id, tenant, true, "admin")
        .await?;
    assert!(!second, "double-suppress is a no-op");

    Ok(())
}

#[tokio::test]
async fn set_suppressed_returns_false_for_missing_or_wrong_tenant() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant_a, user_a, coach_a) = seed_user_tenant_coach(&db).await?;
    let (tenant_b, _, _) = seed_user_tenant_coach(&db).await?;

    let note = memory
        .insert_coach_note(&InsertCoachNoteParams {
            tenant_id: tenant_a,
            user_id: &user_a,
            coach_id: &coach_a,
            conversation_id: None,
            scope: MemoryScope::User,
            content: "tenant a only",
            embedding: None,
        })
        .await?;

    // Wrong tenant: must not flip the row (cross-tenant safety).
    let cross = memory
        .set_coach_note_suppressed(&note.id, tenant_b, true, "admin-b")
        .await?;
    assert!(!cross, "cross-tenant suppress must not match");

    let missing = memory
        .set_coach_note_suppressed("nonexistent-id", tenant_a, true, "admin")
        .await?;
    assert!(!missing);

    // The original row's flag should still be false.
    let still_active = memory
        .list_coach_notes(tenant_a, &user_a, &coach_a, 50)
        .await?;
    assert_eq!(still_active.len(), 1);
    assert!(!still_active[0].suppressed);

    Ok(())
}
