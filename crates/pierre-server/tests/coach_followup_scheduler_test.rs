// ABOUTME: Integration tests for the coach followup scheduler — overdue rows fire and transition delivered
// ABOUTME: Proves due_at metadata becomes real: tick processes overdue rows once and skips fresh ones
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::{Duration, Utc};
use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::InsertCoachFollowupParams;
use pierre_services::coach_followup_scheduler::tick;
use uuid::Uuid;

/// Open the database the lane names through the test factory.
async fn open_db() -> Result<Database> {
    Ok(create_test_db().await?)
}

/// Seed the user, tenant, and coach rows the `coach_followups` foreign keys
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
async fn tick_processes_overdue_followup_and_marks_delivered() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    let due_in_past = Utc::now() - Duration::hours(1);
    let inserted = memory
        .insert_coach_followup(&InsertCoachFollowupParams {
            tenant_id: tenant,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: None,
            content: "check Achilles after 24h",
            due_at: Some(due_in_past),
        })
        .await?;
    assert_eq!(inserted.status.as_str(), "pending");

    // Tick at "now" — the row's due_at is in the past so it should fire.
    let outcome = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;

    assert_eq!(
        outcome.processed, 1,
        "one overdue followup should be picked up"
    );
    assert_eq!(
        outcome.marked_delivered, 1,
        "row should transition to delivered"
    );
    assert_eq!(
        outcome.errors, 0,
        "no errors expected without notification service"
    );

    // Re-list pending — the delivered row should not show up.
    let pending = memory
        .list_pending_followups_for_tenant(tenant, 100)
        .await?;
    assert!(
        pending.is_empty(),
        "delivered followup should leave the pending queue"
    );

    Ok(())
}

#[tokio::test]
async fn tick_skips_followups_with_due_at_in_the_future() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    memory
        .insert_coach_followup(&InsertCoachFollowupParams {
            tenant_id: tenant,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: None,
            content: "future check",
            due_at: Some(Utc::now() + Duration::hours(2)),
        })
        .await?;

    let outcome = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(outcome.processed, 0);
    assert_eq!(outcome.marked_delivered, 0);

    // The row is still pending after the tick.
    let pending = memory
        .list_pending_followups_for_tenant(tenant, 100)
        .await?;
    assert_eq!(pending.len(), 1);

    Ok(())
}

#[tokio::test]
async fn tick_skips_followups_with_no_due_at() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    memory
        .insert_coach_followup(&InsertCoachFollowupParams {
            tenant_id: tenant,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: None,
            content: "no specific time",
            due_at: None,
        })
        .await?;

    // due_at IS NULL → never picked up by the scheduler. The in-prompt
    // injection path handles those followups instead.
    let outcome = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(outcome.processed, 0);

    let pending = memory
        .list_pending_followups_for_tenant(tenant, 100)
        .await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "no specific time");

    Ok(())
}

#[tokio::test]
async fn second_tick_does_not_re_process_delivered_row() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    memory
        .insert_coach_followup(&InsertCoachFollowupParams {
            tenant_id: tenant,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: None,
            content: "check once",
            due_at: Some(Utc::now() - Duration::minutes(10)),
        })
        .await?;

    let first = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(first.processed, 1);

    let second = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(
        second.processed, 0,
        "second tick must not re-pick a delivered row"
    );

    Ok(())
}

#[tokio::test]
async fn tick_processes_multiple_overdue_in_one_batch() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    for i in 0..5 {
        memory
            .insert_coach_followup(&InsertCoachFollowupParams {
                tenant_id: tenant,
                user_id: &user_id,
                coach_id: &coach_id,
                conversation_id: None,
                content: "overdue",
                due_at: Some(Utc::now() - Duration::minutes(30 + i)),
            })
            .await?;
    }

    let outcome = tick(
        memory.as_ref(),
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(outcome.processed, 5);
    assert_eq!(outcome.marked_delivered, 5);
    assert_eq!(outcome.errors, 0);

    Ok(())
}
