// ABOUTME: Sprint C8 — integration tests for tenant-wide coach_notes audit listing
// ABOUTME: Covers list_coach_notes_for_tenant ordering, clamp, and tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use anyhow::Result;
use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::InsertCoachNoteParams;
use pierre_memory::MemoryScope;
use tokio::time::sleep;
use uuid::Uuid;

/// Open the database the lane names through the test factory.
async fn open_db() -> Result<Database> {
    Ok(create_test_db().await?)
}

/// Seed the user, tenant, and coach rows the `coach_notes` foreign keys
/// resolve against, through the repositories so both backends accept them.
async fn seed_user_tenant_coach(db: &Database) -> Result<(TenantId, String)> {
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
    Ok((tenant.id, coach.id.to_string()))
}

fn note_params<'a>(
    tenant_id: TenantId,
    user_id: &'a str,
    coach_id: &'a str,
    content: &'a str,
) -> InsertCoachNoteParams<'a> {
    InsertCoachNoteParams {
        tenant_id,
        user_id,
        coach_id,
        conversation_id: None,
        scope: MemoryScope::User,
        content,
    }
}

#[tokio::test]
async fn tenant_audit_returns_notes_newest_first() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, coach_id) = seed_user_tenant_coach(&db).await?;

    memory
        .insert_coach_note(&note_params(tenant, "user-a", &coach_id, "first note"))
        .await?;
    // Tiny gap so RFC3339 strings sort deterministically.
    sleep(Duration::from_millis(5)).await;
    memory
        .insert_coach_note(&note_params(tenant, "user-b", &coach_id, "second note"))
        .await?;
    sleep(Duration::from_millis(5)).await;
    memory
        .insert_coach_note(&note_params(tenant, "user-c", &coach_id, "third note"))
        .await?;

    let rows = memory.list_coach_notes_for_tenant(tenant, 100).await?;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].content, "third note");
    assert_eq!(rows[1].content, "second note");
    assert_eq!(rows[2].content, "first note");

    Ok(())
}

#[tokio::test]
async fn tenant_audit_clamps_to_limit() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, coach_id) = seed_user_tenant_coach(&db).await?;

    for i in 0..5 {
        memory
            .insert_coach_note(&note_params(
                tenant,
                "user-a",
                &coach_id,
                &format!("note {i}"),
            ))
            .await?;
        sleep(Duration::from_millis(2)).await;
    }

    let rows = memory.list_coach_notes_for_tenant(tenant, 2).await?;
    assert_eq!(rows.len(), 2);

    Ok(())
}

#[tokio::test]
async fn tenant_audit_is_tenant_scoped() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant_a, coach_a) = seed_user_tenant_coach(&db).await?;
    let (tenant_b, coach_b) = seed_user_tenant_coach(&db).await?;

    memory
        .insert_coach_note(&note_params(tenant_a, "user-a", &coach_a, "alpha note"))
        .await?;
    memory
        .insert_coach_note(&note_params(tenant_b, "user-b", &coach_b, "beta note"))
        .await?;

    let rows_a = memory.list_coach_notes_for_tenant(tenant_a, 100).await?;
    let rows_b = memory.list_coach_notes_for_tenant(tenant_b, 100).await?;

    assert_eq!(rows_a.len(), 1);
    assert_eq!(rows_a[0].content, "alpha note");
    assert_eq!(rows_b.len(), 1);
    assert_eq!(rows_b[0].content, "beta note");

    Ok(())
}
