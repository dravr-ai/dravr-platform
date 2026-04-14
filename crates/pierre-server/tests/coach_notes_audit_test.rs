// ABOUTME: Sprint C8 — integration tests for tenant-wide coach_notes audit listing
// ABOUTME: Covers list_coach_notes_for_tenant ordering, clamp, and tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::database::{generate_encryption_key, Database as SqliteDatabase};
use pierre_database::repositories::{HarnessMemoryRepository, InsertCoachNoteParams};
use pierre_memory::MemoryScope;
use sqlx::Executor;
use tokio::time::sleep;
use uuid::Uuid;

async fn open_in_memory_db() -> Result<SqliteDatabase> {
    let encryption_key = generate_encryption_key().to_vec();
    let db = SqliteDatabase::new("sqlite::memory:", encryption_key).await?;
    Ok(db)
}

/// Seed minimal user/tenant/coach rows so the `coach_notes` foreign keys resolve.
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
        embedding: None,
    }
}

#[tokio::test]
async fn tenant_audit_returns_notes_newest_first() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, coach_id) = seed_user_tenant_coach(&db).await?;

    db.insert_coach_note(&note_params(tenant, "user-a", &coach_id, "first note"))
        .await?;
    // Tiny gap so RFC3339 strings sort deterministically.
    sleep(Duration::from_millis(5)).await;
    db.insert_coach_note(&note_params(tenant, "user-b", &coach_id, "second note"))
        .await?;
    sleep(Duration::from_millis(5)).await;
    db.insert_coach_note(&note_params(tenant, "user-c", &coach_id, "third note"))
        .await?;

    let rows = db.list_coach_notes_for_tenant(tenant, 100).await?;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].content, "third note");
    assert_eq!(rows[1].content, "second note");
    assert_eq!(rows[2].content, "first note");

    Ok(())
}

#[tokio::test]
async fn tenant_audit_clamps_to_limit() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, coach_id) = seed_user_tenant_coach(&db).await?;

    for i in 0..5 {
        db.insert_coach_note(&note_params(
            tenant,
            "user-a",
            &coach_id,
            &format!("note {i}"),
        ))
        .await?;
        sleep(Duration::from_millis(2)).await;
    }

    let rows = db.list_coach_notes_for_tenant(tenant, 2).await?;
    assert_eq!(rows.len(), 2);

    Ok(())
}

#[tokio::test]
async fn tenant_audit_is_tenant_scoped() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant_a, coach_a) = seed_user_tenant_coach(&db).await?;
    let (tenant_b, coach_b) = seed_user_tenant_coach(&db).await?;

    db.insert_coach_note(&note_params(tenant_a, "user-a", &coach_a, "alpha note"))
        .await?;
    db.insert_coach_note(&note_params(tenant_b, "user-b", &coach_b, "beta note"))
        .await?;

    let rows_a = db.list_coach_notes_for_tenant(tenant_a, 100).await?;
    let rows_b = db.list_coach_notes_for_tenant(tenant_b, 100).await?;

    assert_eq!(rows_a.len(), 1);
    assert_eq!(rows_a[0].content, "alpha note");
    assert_eq!(rows_b.len(), 1);
    assert_eq!(rows_b[0].content, "beta note");

    Ok(())
}
