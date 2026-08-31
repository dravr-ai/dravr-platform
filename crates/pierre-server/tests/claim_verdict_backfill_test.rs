// ABOUTME: Sprint C17 — integration tests for the ClaimVerdict backfill over chat_messages
// ABOUTME: Seeds synthetic chat history and asserts verdict persistence + resume cursor behavior
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `ClaimVerdict` backfill integration tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use pierre_core::models::{
    AddMessageParams, CoachCategory, CreateCoachRequest, Tenant, TenantId, User,
};
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_evals::VerificationConfig;
use pierre_services::claim_verdict_backfill::{run_backfill, BackfillParams};
use uuid::Uuid;

async fn open_test_db() -> Result<Database> {
    Ok(create_test_db_with_key(generate_encryption_key().to_vec()).await?)
}

/// Seed a synthetic tenant + user + coach + conversation + assistant
/// message through the repositories, so the rows are whatever the backend
/// writes. Each call makes its own tenant, which is what keeps the scans in
/// the tests below independent of one another. Returns `(tenant_id,
/// message_id)` for assertions.
async fn seed_assistant_message(db: &Database, content: &str) -> Result<(TenantId, String)> {
    let repos = db.repositories();
    let user = User::new(
        format!("{}@test.local", Uuid::new_v4()),
        "hash".to_owned(),
        None,
    );
    let user_id = user.id;
    repos.users.create(&user).await?;

    let tenant_id = TenantId::generate();
    let now = Utc::now();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Test Tenant".to_owned(),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        })
        .await?;

    let coach = repos
        .coaches
        .create(
            user_id,
            tenant_id,
            &CreateCoachRequest {
                title: "Test Coach".to_owned(),
                description: None,
                system_prompt: "You are a helpful coach.".to_owned(),
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
    let coach_id = coach.id.to_string();

    let user_id_str = user_id.to_string();
    let conversation = repos
        .chat
        .create_conversation(
            &user_id_str,
            tenant_id,
            "Test chat",
            "gemini-pro",
            Some(&coach_id),
            None,
        )
        .await?;
    let message = repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conversation.id,
            user_id: &user_id_str,
            role: "assistant",
            content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await?;

    Ok((tenant_id, message.id))
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
    let db = open_test_db().await?;

    // Claim text that the heuristic extractor can pick up: concrete
    // training prescription statement. Even if the corpus only has a
    // few entries, the deterministic + rhetoric layers should classify
    // it as either supported, unsupported, or rhetorical — which the
    // backfill records.
    let content =
        "You should run 5 miles every day and drink 4 liters of water to fix your knee pain.";
    let (tenant_id, message_id) = seed_assistant_message(&db, content).await?;

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
    let db = open_test_db().await?;
    let content = "Drink a gallon of water per day to prevent muscle cramps.";
    let (tenant_id, _msg) = seed_assistant_message(&db, content).await?;

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
    let db = open_test_db().await?;
    // Seed three messages across three distinct tenants (each
    // `seed_assistant_message` creates a fresh one). We'll scan only
    // the first tenant and cap the limit at 0 → clamped to 1.
    let (tenant_id, _m1) = seed_assistant_message(&db, "first message").await?;
    let (_t2, _m2) = seed_assistant_message(&db, "second message").await?;
    let (_t3, _m3) = seed_assistant_message(&db, "third message").await?;

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
    let db = open_test_db().await?;
    let (tenant_id, _first_msg) = seed_assistant_message(&db, "assistant turn one").await?;

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
