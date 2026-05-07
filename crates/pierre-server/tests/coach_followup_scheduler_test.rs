// ABOUTME: Integration tests for the coach followup scheduler — overdue rows fire and transition delivered
// ABOUTME: Proves due_at metadata becomes real: tick processes overdue rows once and skips fresh ones
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
use pierre_mcp_server::services::coach_followup_scheduler::tick;
use sqlx::Executor;
use uuid::Uuid;

async fn open_in_memory_db() -> Result<SqliteDatabase> {
    let encryption_key = generate_encryption_key().to_vec();
    let db = SqliteDatabase::new("sqlite::memory:", encryption_key).await?;
    Ok(db)
}

/// Seed the minimum FK rows required so `coach_followups` inserts succeed.
async fn seed_user_tenant_coach(db: &SqliteDatabase) -> Result<(TenantId, String, String)> {
    let user_id = Uuid::new_v4().to_string();
    let tenant_uuid = Uuid::new_v4();
    let tenant_id_str = tenant_uuid.to_string();
    let coach_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let pool = db.pool();

    pool.execute(
        sqlx::query(
            r"INSERT INTO users (id, email, password_hash, created_at, last_active)
              VALUES ($1, $2, $3, $4, $4)",
        )
        .bind(&user_id)
        .bind(format!("{user_id}@test.local"))
        .bind("hash")
        .bind(&now),
    )
    .await?;

    pool.execute(
        sqlx::query(
            r"INSERT INTO tenants (id, name, slug, owner_user_id, created_at, updated_at)
              VALUES ($1, $2, $3, $4, $5, $5)",
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
            r"INSERT INTO coaches (id, user_id, tenant_id, title, system_prompt,
                                    category, created_at, updated_at)
              VALUES ($1, $2, $3, 'Test Coach', 'You are helpful', 'custom', $4, $4)",
        )
        .bind(&coach_id)
        .bind(&user_id)
        .bind(&tenant_id_str)
        .bind(&now),
    )
    .await?;

    Ok((TenantId::from(tenant_uuid), user_id, coach_id))
}

#[tokio::test]
async fn tick_processes_overdue_followup_and_marks_delivered() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    let due_in_past = Utc::now() - Duration::hours(1);
    let inserted = db
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
        &db,
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
    let pending = db.list_pending_followups_for_tenant(tenant, 100).await?;
    assert!(
        pending.is_empty(),
        "delivered followup should leave the pending queue"
    );

    Ok(())
}

#[tokio::test]
async fn tick_skips_followups_with_due_at_in_the_future() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    db.insert_coach_followup(&InsertCoachFollowupParams {
        tenant_id: tenant,
        user_id: &user_id,
        coach_id: &coach_id,
        conversation_id: None,
        content: "future check",
        due_at: Some(Utc::now() + Duration::hours(2)),
    })
    .await?;

    let outcome = tick(
        &db,
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(outcome.processed, 0);
    assert_eq!(outcome.marked_delivered, 0);

    // The row is still pending after the tick.
    let pending = db.list_pending_followups_for_tenant(tenant, 100).await?;
    assert_eq!(pending.len(), 1);

    Ok(())
}

#[tokio::test]
async fn tick_skips_followups_with_no_due_at() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    db.insert_coach_followup(&InsertCoachFollowupParams {
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
        &db,
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(outcome.processed, 0);

    let pending = db.list_pending_followups_for_tenant(tenant, 100).await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "no specific time");

    Ok(())
}

#[tokio::test]
async fn second_tick_does_not_re_process_delivered_row() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    db.insert_coach_followup(&InsertCoachFollowupParams {
        tenant_id: tenant,
        user_id: &user_id,
        coach_id: &coach_id,
        conversation_id: None,
        content: "check once",
        due_at: Some(Utc::now() - Duration::minutes(10)),
    })
    .await?;

    let first = tick(
        &db,
        #[cfg(feature = "client-notifications")]
        None,
        Utc::now(),
        100,
    )
    .await?;
    assert_eq!(first.processed, 1);

    let second = tick(
        &db,
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
    let db = open_in_memory_db().await?;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    for i in 0..5 {
        db.insert_coach_followup(&InsertCoachFollowupParams {
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
        &db,
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
