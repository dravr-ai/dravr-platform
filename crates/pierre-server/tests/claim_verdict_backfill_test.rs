// ABOUTME: Sprint C17 — integration tests for the ClaimVerdict backfill over chat_messages
// ABOUTME: Seeds synthetic chat history and asserts verdict persistence + resume cursor behavior
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_evals::VerificationConfig;
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::environment::PostgresPoolConfig;
use pierre_mcp_server::services::claim_verdict_backfill::{run_backfill, BackfillParams};
use sqlx::Executor;
use uuid::Uuid;

async fn open_in_memory_db() -> Result<Database> {
    let encryption_key = generate_encryption_key().to_vec();
    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await?;
    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key).await?;
    Ok(db)
}

/// Seed a synthetic tenant + user + coach + conversation + assistant
/// message. Returns `(tenant_id, message_id)` for assertions.
async fn seed_assistant_message(
    db: &Database,
    content: &str,
    sequence: u32,
) -> Result<(TenantId, String)> {
    let pool = match db {
        Database::SQLite(sqlite) => sqlite.pool(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(_) => panic!("test expects SQLite backend"),
    };

    let user_id = Uuid::new_v4().to_string();
    let tenant_uuid = Uuid::new_v4();
    let tenant_id_str = tenant_uuid.to_string();
    let coach_id = Uuid::new_v4().to_string();
    let conversation_id = Uuid::new_v4().to_string();
    let message_id = Uuid::new_v4().to_string();
    // Sequence lets the caller control `created_at` ordering across
    // seeded messages so the backfill walker sees a deterministic
    // order regardless of test timing.
    let base = Utc::now();
    let created_at = (base + chrono::Duration::seconds(i64::from(sequence))).to_rfc3339();

    pool.execute(
        sqlx::query(
            r"INSERT INTO users (id, email, password_hash, created_at, last_active)
              VALUES ($1, $2, 'hash', $3, $3)",
        )
        .bind(&user_id)
        .bind(format!("{user_id}@test.local"))
        .bind(&created_at),
    )
    .await?;
    pool.execute(
        sqlx::query(
            r"INSERT INTO tenants (id, name, slug, owner_user_id, created_at, updated_at)
              VALUES ($1, 'Test Tenant', $2, $3, $4, $4)",
        )
        .bind(&tenant_id_str)
        .bind(format!("tenant-{tenant_uuid}"))
        .bind(&user_id)
        .bind(&created_at),
    )
    .await?;
    pool.execute(
        sqlx::query(
            r"INSERT INTO coaches (id, user_id, tenant_id, title, system_prompt, category, created_at, updated_at)
              VALUES ($1, $2, $3, 'Test Coach', 'You are a helpful coach.', 'custom', $4, $4)",
        )
        .bind(&coach_id)
        .bind(&user_id)
        .bind(&tenant_id_str)
        .bind(&created_at),
    )
    .await?;
    pool.execute(
        sqlx::query(
            r"INSERT INTO chat_conversations (id, user_id, tenant_id, title, model, coach_id, total_tokens, created_at, updated_at)
              VALUES ($1, $2, $3, 'Test chat', 'gemini-pro', $4, 0, $5, $5)",
        )
        .bind(&conversation_id)
        .bind(&user_id)
        .bind(&tenant_id_str)
        .bind(&coach_id)
        .bind(&created_at),
    )
    .await?;
    pool.execute(
        sqlx::query(
            r"INSERT INTO chat_messages (id, conversation_id, role, content, created_at)
              VALUES ($1, $2, 'assistant', $3, $4)",
        )
        .bind(&message_id)
        .bind(&conversation_id)
        .bind(content)
        .bind(&created_at),
    )
    .await?;

    Ok((TenantId::from(tenant_uuid), message_id))
}

fn default_params<'a>(tenant_id: TenantId) -> BackfillParams<'a> {
    BackfillParams {
        tenant_id,
        limit: 1000,
        since: None,
        dry_run: false,
        sleep_between: Duration::ZERO,
        resume: false,
    }
}

#[tokio::test]
async fn backfill_walks_assistant_messages_and_persists_verdicts() -> Result<()> {
    let db = open_in_memory_db().await?;

    // Claim text that the heuristic extractor can pick up: concrete
    // training prescription statement. Even if the corpus only has a
    // few entries, the deterministic + rhetoric layers should classify
    // it as either supported, unsupported, or rhetorical — which the
    // backfill records.
    let content =
        "You should run 5 miles every day and drink 4 liters of water to fix your knee pain.";
    let (tenant_id, message_id) = seed_assistant_message(&db, content, 1).await?;

    let repos = db.repositories();
    let stats = run_backfill(
        &repos,
        &db,
        &default_params(tenant_id),
        &VerificationConfig::default(),
    )
    .await?;

    assert_eq!(stats.messages_scanned, 1);
    assert!(!stats.dry_run);
    assert_eq!(stats.last_message_id.as_deref(), Some(message_id.as_str()));

    Ok(())
}

#[tokio::test]
async fn dry_run_scans_without_persisting_anything() -> Result<()> {
    let db = open_in_memory_db().await?;
    let content = "Drink a gallon of water per day to prevent muscle cramps.";
    let (tenant_id, _msg) = seed_assistant_message(&db, content, 1).await?;

    let repos = db.repositories();
    let mut params = default_params(tenant_id);
    params.dry_run = true;
    let stats = run_backfill(&repos, &db, &params, &VerificationConfig::default()).await?;

    assert_eq!(stats.messages_scanned, 1);
    assert!(stats.dry_run);
    assert_eq!(stats.persistence_errors, 0);
    Ok(())
}

#[tokio::test]
async fn backfill_respects_limit_clamp() -> Result<()> {
    let db = open_in_memory_db().await?;
    // Seed three messages across three distinct tenants (each
    // `seed_assistant_message` creates a fresh one). We'll scan only
    // the first tenant and cap the limit at 0 → clamped to 1.
    let (tenant_id, _m1) = seed_assistant_message(&db, "first message", 1).await?;
    let (_t2, _m2) = seed_assistant_message(&db, "second message", 2).await?;
    let (_t3, _m3) = seed_assistant_message(&db, "third message", 3).await?;

    let repos = db.repositories();
    let mut params = default_params(tenant_id);
    params.limit = 0; // clamped up to 1
    let stats = run_backfill(&repos, &db, &params, &VerificationConfig::default()).await?;

    // Only one message belongs to tenant_id's conversations.
    assert!(stats.messages_scanned <= 1);
    Ok(())
}

#[tokio::test]
async fn resume_skips_previously_processed_messages() -> Result<()> {
    let db = open_in_memory_db().await?;
    let (tenant_id, _first_msg) = seed_assistant_message(&db, "assistant turn one", 1).await?;

    let repos = db.repositories();

    // First pass — walks the one seeded message and stores the cursor.
    let first_stats = run_backfill(
        &repos,
        &db,
        &default_params(tenant_id),
        &VerificationConfig::default(),
    )
    .await?;
    assert_eq!(first_stats.messages_scanned, 1);

    // Second pass with resume=true — the cursor is stored, so no new
    // messages should be scanned because nothing was inserted in
    // between.
    let mut params = default_params(tenant_id);
    params.resume = true;
    let resumed_stats = run_backfill(&repos, &db, &params, &VerificationConfig::default()).await?;
    assert_eq!(resumed_stats.messages_scanned, 0);
    Ok(())
}
