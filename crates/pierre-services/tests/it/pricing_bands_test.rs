// ABOUTME: A model priced by prompt size costs the same from its prompt-size bands as from its calls
// ABOUTME: Mixed short and long Haiku 5.5 prompts, priced per row and through cost_of_usage_group, in every scope
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Claude Haiku 5.5 bills a call by its own prompt: the base card up to
//! 100K prompt tokens, the long-prompt card above. An aggregate row cannot be
//! priced from its sums, so [`cost_of_usage_group`] reads the rows behind it
//! summed per side of that threshold. These cases pin that the bands price to
//! exactly what pricing each call on its own does.

#![allow(clippy::unwrap_used, missing_docs)]

use chrono::{Duration, Utc};
use pierre_core::models::usage::{InsertLlmUsage, LlmUsageRecord};
use pierre_core::models::ConversationTurnId;
use pierre_database::repositories::LlmUsageScope;
use pierre_database::RepositoryRegistry;
use pierre_services::pricing::{cost_for_record, cost_of_usage_group};
use pierre_test_support::db::create_test_db;

/// One call's tokens: prompt, completion, cache read, cache write, reasoning.
type Call = (i64, i64, i64, i64, i64);

/// Short and long prompts on both sides of the 100K threshold, the threshold
/// itself included (it bills on the base card), with cache and reasoning tokens.
const MIXED: [Call; 5] = [
    (40_000, 1_000, 10_000, 5_000, 0),
    (100_000, 2_000, 0, 0, 0),
    (90_000, 500, 30_000, 0, 200),
    (150_000, 3_000, 40_000, 10_000, 0),
    (250_000, 500, 0, 0, 100),
];

async fn insert(
    repos: &RepositoryRegistry,
    tenant: &str,
    user: &str,
    call_type: &str,
    (prompt, completion, cached, written, reasoning): Call,
) -> LlmUsageRecord {
    repos
        .llm_usage
        .insert_llm_usage(&InsertLlmUsage {
            tenant_id: tenant,
            user_id: user,
            conversation_id: None,
            turn_id: ConversationTurnId::new(),
            provider: "anthropic",
            model: "claude-haiku-5-5",
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            cached_tokens: cached,
            cached_write_tokens: written,
            cached_write_1h_tokens: 0,
            reasoning_tokens: reasoning,
            call_type,
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: Some(100),
            cost_usd: 0.0,
            call_sequence: None,
        })
        .await
        .unwrap()
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-12
}

#[tokio::test]
async fn prompt_size_bands_price_to_the_sum_of_their_calls_in_every_scope() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let since = (Utc::now() - Duration::days(1)).to_rfc3339();

    let mut alice = Vec::new();
    for call in MIXED {
        alice.push(insert(repos, "tenant-a", "alice", "chat", call).await);
    }
    // Alice in another tenant, Bob beside her, and another call type: each must
    // stay out of the scopes and the group that do not cover it.
    let elsewhere = insert(repos, "tenant-b", "alice", "chat", MIXED[3]).await;
    let bob = insert(repos, "tenant-a", "bob", "chat", MIXED[4]).await;
    insert(repos, "tenant-a", "alice", "memory_extraction", MIXED[4]).await;

    let priced = |records: &[&LlmUsageRecord]| records.iter().map(|r| cost_for_record(r)).sum();
    let alice_rows: Vec<&LlmUsageRecord> = alice.iter().collect();
    let tenant_rows: Vec<&LlmUsageRecord> = alice.iter().chain([&bob]).collect();
    let user_rows: Vec<&LlmUsageRecord> = alice.iter().chain([&elsewhere]).collect();
    let all_rows: Vec<&LlmUsageRecord> = alice.iter().chain([&bob, &elsewhere]).collect();

    let group = repos
        .llm_usage
        .get_llm_usage_aggregates(Some("tenant-a"), &since)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.call_type == "chat")
        .unwrap();

    for (scope, rows) in [
        (
            LlmUsageScope::UserInTenant {
                user_id: "alice",
                tenant_id: "tenant-a",
            },
            &alice_rows,
        ),
        (LlmUsageScope::Tenant("tenant-a"), &tenant_rows),
        (LlmUsageScope::User("alice"), &user_rows),
        (LlmUsageScope::AllTenants, &all_rows),
    ] {
        let expected: f64 = priced(rows);
        let cost = cost_of_usage_group(repos.llm_usage.as_ref(), scope, &group, &since)
            .await
            .unwrap();
        assert!(
            close(cost, expected),
            "bug: {scope:?} priced at {cost}, its calls at {expected}"
        );
    }
}
