// ABOUTME: Integration tests for the Coach Notes Audit suppression flag
// ABOUTME: Proves admin POST /suppress excludes notes from chat-pipeline memory recall
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::database::{generate_encryption_key, Database as SqliteDatabase};
use pierre_database::repositories::{HarnessMemoryRepository, InsertCoachNoteParams};
use pierre_memory::scope::MemoryScope;
use sqlx::Executor;
use uuid::Uuid;

async fn open_in_memory_db() -> Result<SqliteDatabase> {
    let encryption_key = generate_encryption_key().to_vec();
    let db = SqliteDatabase::new("sqlite::memory:", encryption_key).await?;
    Ok(db)
}

/// Seed FK rows so `coach_notes` inserts succeed.
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
async fn suppressed_notes_are_excluded_from_memory_recall() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    let note = db
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
    let before = db.list_coach_notes(tenant, &user_id, &coach_id, 50).await?;
    assert_eq!(before.len(), 1);

    let changed = db
        .set_coach_note_suppressed(&note.id, tenant, true, "test-admin")
        .await?;
    assert!(changed, "first suppress should flip the flag");

    // After suppression: recall must NOT see the note.
    let after = db.list_coach_notes(tenant, &user_id, &coach_id, 50).await?;
    assert!(
        after.is_empty(),
        "suppressed notes must be excluded from memory recall"
    );

    // Audit listing still surfaces the row (suppressed=true) for review.
    let audit = db.list_coach_notes_for_tenant(tenant, 50).await?;
    assert_eq!(audit.len(), 1);
    assert!(
        audit[0].suppressed,
        "audit panel must still see suppressed rows"
    );

    Ok(())
}

#[tokio::test]
async fn suppress_then_unsuppress_restores_recall() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    let note = db
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

    db.set_coach_note_suppressed(&note.id, tenant, true, "admin")
        .await?;
    let after_suppress = db.list_coach_notes(tenant, &user_id, &coach_id, 50).await?;
    assert!(after_suppress.is_empty());

    let changed = db
        .set_coach_note_suppressed(&note.id, tenant, false, "admin")
        .await?;
    assert!(changed, "unsuppress should flip the flag back");

    let after_unsuppress = db.list_coach_notes(tenant, &user_id, &coach_id, 50).await?;
    assert_eq!(after_unsuppress.len(), 1, "recall returns the note again");
    assert!(!after_unsuppress[0].suppressed);

    Ok(())
}

#[tokio::test]
async fn set_suppressed_is_idempotent() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant, user_id, coach_id) = seed_user_tenant_coach(&db).await?;

    let note = db
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

    let first = db
        .set_coach_note_suppressed(&note.id, tenant, true, "admin")
        .await?;
    assert!(first);
    let second = db
        .set_coach_note_suppressed(&note.id, tenant, true, "admin")
        .await?;
    assert!(!second, "double-suppress is a no-op");

    Ok(())
}

#[tokio::test]
async fn set_suppressed_returns_false_for_missing_or_wrong_tenant() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant_a, user_a, coach_a) = seed_user_tenant_coach(&db).await?;
    let (tenant_b, _, _) = seed_user_tenant_coach(&db).await?;

    let note = db
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
    let cross = db
        .set_coach_note_suppressed(&note.id, tenant_b, true, "admin-b")
        .await?;
    assert!(!cross, "cross-tenant suppress must not match");

    let missing = db
        .set_coach_note_suppressed("nonexistent-id", tenant_a, true, "admin")
        .await?;
    assert!(!missing);

    // The original row's flag should still be false.
    let still_active = db.list_coach_notes(tenant_a, &user_a, &coach_a, 50).await?;
    assert_eq!(still_active.len(), 1);
    assert!(!still_active[0].suppressed);

    Ok(())
}
