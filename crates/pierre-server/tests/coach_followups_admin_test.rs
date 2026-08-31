// ABOUTME: Sprint C7 — integration tests for tenant-wide followup listing and cancel
// ABOUTME: Covers list_pending_followups_for_tenant + cancel_followup semantics on the lane's database
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
use uuid::Uuid;

/// Open the database the lane names through the test factory.
async fn open_db() -> Result<Database> {
    Ok(create_test_db().await?)
}

/// Seed the user, tenant, and coach rows the `coach_followups` foreign keys
/// resolve against, through the repositories so both backends accept them.
/// Returns `(tenant_id, coach_id)` the caller will use for all followup ops.
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

fn followup_params<'a>(
    tenant_id: TenantId,
    user_id: &'a str,
    coach_id: &'a str,
    content: &'a str,
    due_at: Option<chrono::DateTime<Utc>>,
) -> InsertCoachFollowupParams<'a> {
    InsertCoachFollowupParams {
        tenant_id,
        user_id,
        coach_id,
        conversation_id: None,
        content,
        due_at,
    }
}

#[tokio::test]
async fn tenant_wide_list_orders_by_due_date_nulls_last() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, coach_id) = seed_user_tenant_coach(&db).await?;
    let now = Utc::now();

    memory
        .insert_coach_followup(&followup_params(
            tenant,
            "user-a",
            &coach_id,
            "check achilles",
            Some(now + Duration::hours(4)),
        ))
        .await?;
    memory
        .insert_coach_followup(&followup_params(
            tenant,
            "user-b",
            &coach_id,
            "taper week reminder",
            Some(now + Duration::days(2)),
        ))
        .await?;
    memory
        .insert_coach_followup(&followup_params(
            tenant,
            "user-c",
            &coach_id,
            "hydration check",
            None,
        ))
        .await?;

    let rows = memory
        .list_pending_followups_for_tenant(tenant, 100)
        .await?;

    assert_eq!(rows.len(), 3);
    // Ordered by due_at ASC NULLS LAST → achilles, taper, hydration.
    assert_eq!(rows[0].content, "check achilles");
    assert_eq!(rows[1].content, "taper week reminder");
    assert_eq!(rows[2].content, "hydration check");

    Ok(())
}

#[tokio::test]
async fn cancel_followup_transitions_pending_once() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant, coach_id) = seed_user_tenant_coach(&db).await?;

    let followup = memory
        .insert_coach_followup(&followup_params(
            tenant,
            "user-a",
            &coach_id,
            "check achilles",
            None,
        ))
        .await?;

    let cancelled = memory.cancel_followup(&followup.id, tenant).await?;
    assert!(
        cancelled,
        "first cancel should transition pending → cancelled"
    );

    // Second cancel is a no-op: the row is no longer pending.
    let cancelled_again = memory.cancel_followup(&followup.id, tenant).await?;
    assert!(!cancelled_again, "double-cancel should return false");

    // Cancelled followups should no longer appear in the pending list.
    let rows = memory
        .list_pending_followups_for_tenant(tenant, 100)
        .await?;
    assert!(rows.is_empty());

    Ok(())
}

#[tokio::test]
async fn pending_list_is_tenant_scoped() -> Result<()> {
    let db = open_db().await?;
    let memory = db.repositories().memory;
    let (tenant_a, coach_a) = seed_user_tenant_coach(&db).await?;
    let (tenant_b, coach_b) = seed_user_tenant_coach(&db).await?;

    memory
        .insert_coach_followup(&followup_params(
            tenant_a,
            "user-a",
            &coach_a,
            "alpha followup",
            None,
        ))
        .await?;
    memory
        .insert_coach_followup(&followup_params(
            tenant_b,
            "user-b",
            &coach_b,
            "beta followup",
            None,
        ))
        .await?;

    let rows_a = memory
        .list_pending_followups_for_tenant(tenant_a, 100)
        .await?;
    let rows_b = memory
        .list_pending_followups_for_tenant(tenant_b, 100)
        .await?;

    assert_eq!(rows_a.len(), 1);
    assert_eq!(rows_a[0].content, "alpha followup");
    assert_eq!(rows_b.len(), 1);
    assert_eq!(rows_b[0].content, "beta followup");

    Ok(())
}
