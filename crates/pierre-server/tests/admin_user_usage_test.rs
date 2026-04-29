// ABOUTME: Phase 2 — admin per-user usage routes integration tests
// ABOUTME: Asserts get_llm_usage_aggregates_by_user / get_llm_usage_daily_series_by_user
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_statements
)]

use chrono::{Duration, Utc};
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

#[tokio::test]
async fn aggregates_by_user_isolates_users_in_same_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let tenant_id = TenantId::new();
    let user_a = Uuid::new_v4();
    let user_b = Uuid::new_v4();
    let since = (Utc::now() - Duration::days(7)).to_rfc3339();

    // Seed two distinct users in the same tenant.
    for (uid, tokens) in [(user_a, 1_000_i32), (user_b, 5_000_i32)] {
        for _ in 0..3_i32 {
            let turn = ConversationTurnId::new();
            let params = InsertLlmUsage {
                tenant_id: &tenant_id.to_string(),
                user_id: &uid.to_string(),
                conversation_id: None,
                turn_id: turn,
                provider: "google",
                model: "gemini-2.5-flash",
                prompt_tokens: i64::from(tokens),
                completion_tokens: i64::from(tokens) / 2,
                total_tokens: i64::from(tokens) * 3 / 2,
                cached_tokens: 0,
                call_type: "chat",
                tool_calls_count: 0,
                tools_called: "[]",
                execution_time_ms: Some(100),
                cost_usd: 0.001,
                call_sequence: None,
            };
            repos.llm_usage.insert_llm_usage(&params).await.unwrap();
        }
    }

    let agg_a = repos
        .llm_usage
        .get_llm_usage_aggregates_by_user(&user_a.to_string(), &since)
        .await
        .unwrap();
    let agg_b = repos
        .llm_usage
        .get_llm_usage_aggregates_by_user(&user_b.to_string(), &since)
        .await
        .unwrap();

    assert_eq!(agg_a.len(), 1, "user A: one (provider, model, call_type)");
    assert_eq!(agg_b.len(), 1, "user B: one (provider, model, call_type)");
    assert_eq!(agg_a[0].calls, 3, "user A: 3 calls");
    assert_eq!(agg_b[0].calls, 3, "user B: 3 calls");
    // user_b row tokens should be 5x user_a's (5000 vs 1000 prompt scaling).
    assert!(agg_b[0].total_tokens > agg_a[0].total_tokens);
}

#[tokio::test]
async fn daily_series_by_user_filters_to_owner_only() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let tenant_id = TenantId::new();
    let owner = Uuid::new_v4();
    let other = Uuid::new_v4();
    let since = (Utc::now() - Duration::days(2)).to_rfc3339();

    // One row per user.
    for uid in [owner, other] {
        let turn = ConversationTurnId::new();
        let params = InsertLlmUsage {
            tenant_id: &tenant_id.to_string(),
            user_id: &uid.to_string(),
            conversation_id: None,
            turn_id: turn,
            provider: "google",
            model: "gemini-2.5-flash",
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cached_tokens: 0,
            call_type: "chat",
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: Some(50),
            cost_usd: 0.001,
            call_sequence: None,
        };
        repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    }

    let series = repos
        .llm_usage
        .get_llm_usage_daily_series_by_user(&owner.to_string(), &since)
        .await
        .unwrap();
    let total_calls: i64 = series.iter().map(|d| d.calls).sum();
    assert_eq!(
        total_calls, 1,
        "owner only owns one row; the other user's row must not bleed in"
    );
}
