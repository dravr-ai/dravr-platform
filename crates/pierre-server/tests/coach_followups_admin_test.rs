// ABOUTME: Sprint C7 — integration tests for tenant-wide followup listing and cancel
// ABOUTME: Covers list_pending_followups_for_tenant + cancel_followup semantics on SQLite
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::{Duration, Utc};
use pierre_core::models::TenantId;
use pierre_database::database::{generate_encryption_key, Database as SqliteDatabase};
use pierre_database::repositories::{HarnessMemoryRepository, InsertCoachFollowupParams};
use sqlx::Executor;
use uuid::Uuid;

async fn open_in_memory_db() -> Result<SqliteDatabase> {
    let encryption_key = generate_encryption_key().to_vec();
    let db = SqliteDatabase::new("sqlite::memory:", encryption_key).await?;
    Ok(db)
}

/// Seed the minimum rows required so `coach_followups` foreign keys resolve.
/// Returns `(tenant_id, coach_id)` the caller will use for all followup ops.
async fn seed_user_tenant_coach(db: &SqliteDatabase) -> Result<(TenantId, String)> {
    let user_id = Uuid::new_v4().to_string();
    let tenant_uuid = Uuid::new_v4();
    let tenant_id_str = tenant_uuid.to_string();
    let coach_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let pool = db.pool();

    pool.execute(
        sqlx::query(
            r"
            INSERT INTO users (
                id, email, password_hash, created_at, last_active
            )
            VALUES ($1, $2, $3, $4, $4)
            ",
        )
        .bind(&user_id)
        .bind(format!("{user_id}@test.local"))
        .bind("hash")
        .bind(&now),
    )
    .await?;

    pool.execute(
        sqlx::query(
            r"
            INSERT INTO tenants (
                id, name, slug, owner_user_id, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $5)
            ",
        )
        .bind(&tenant_id_str)
        .bind("Test Tenant")
        .bind(format!("tenant-{tenant_uuid}"))
        .bind(&user_id)
        .bind(&now),
    )
    .await?;

    pool.execute(
        sqlx::query(
            r"
            INSERT INTO coaches (
                id, user_id, tenant_id, title, system_prompt,
                category, created_at, updated_at
            )
            VALUES ($1, $2, $3, 'Test Coach', 'You are helpful', 'custom', $4, $4)
            ",
        )
        .bind(&coach_id)
        .bind(&user_id)
        .bind(&tenant_id_str)
        .bind(&now),
    )
    .await?;

    Ok((TenantId::from(tenant_uuid), coach_id))
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
    let db = open_in_memory_db().await?;
    let (tenant, coach_id) = seed_user_tenant_coach(&db).await?;
    let now = Utc::now();

    db.insert_coach_followup(&followup_params(
        tenant,
        "user-a",
        &coach_id,
        "check achilles",
        Some(now + Duration::hours(4)),
    ))
    .await?;
    db.insert_coach_followup(&followup_params(
        tenant,
        "user-b",
        &coach_id,
        "taper week reminder",
        Some(now + Duration::days(2)),
    ))
    .await?;
    db.insert_coach_followup(&followup_params(
        tenant,
        "user-c",
        &coach_id,
        "hydration check",
        None,
    ))
    .await?;

    let rows = db.list_pending_followups_for_tenant(tenant, 100).await?;

    assert_eq!(rows.len(), 3);
    // Ordered by due_at ASC NULLS LAST → achilles, taper, hydration.
    assert_eq!(rows[0].content, "check achilles");
    assert_eq!(rows[1].content, "taper week reminder");
    assert_eq!(rows[2].content, "hydration check");

    Ok(())
}

#[tokio::test]
async fn cancel_followup_transitions_pending_once() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, coach_id) = seed_user_tenant_coach(&db).await?;

    let followup = db
        .insert_coach_followup(&followup_params(
            tenant,
            "user-a",
            &coach_id,
            "check achilles",
            None,
        ))
        .await?;

    let cancelled = db.cancel_followup(&followup.id, tenant).await?;
    assert!(
        cancelled,
        "first cancel should transition pending → cancelled"
    );

    // Second cancel is a no-op: the row is no longer pending.
    let cancelled_again = db.cancel_followup(&followup.id, tenant).await?;
    assert!(!cancelled_again, "double-cancel should return false");

    // Cancelled followups should no longer appear in the pending list.
    let rows = db.list_pending_followups_for_tenant(tenant, 100).await?;
    assert!(rows.is_empty());

    Ok(())
}

#[tokio::test]
async fn pending_list_is_tenant_scoped() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant_a, coach_a) = seed_user_tenant_coach(&db).await?;
    let (tenant_b, coach_b) = seed_user_tenant_coach(&db).await?;

    db.insert_coach_followup(&followup_params(
        tenant_a,
        "user-a",
        &coach_a,
        "alpha followup",
        None,
    ))
    .await?;
    db.insert_coach_followup(&followup_params(
        tenant_b,
        "user-b",
        &coach_b,
        "beta followup",
        None,
    ))
    .await?;

    let rows_a = db.list_pending_followups_for_tenant(tenant_a, 100).await?;
    let rows_b = db.list_pending_followups_for_tenant(tenant_b, 100).await?;

    assert_eq!(rows_a.len(), 1);
    assert_eq!(rows_a[0].content, "alpha followup");
    assert_eq!(rows_b.len(), 1);
    assert_eq!(rows_b[0].content, "beta followup");

    Ok(())
}
