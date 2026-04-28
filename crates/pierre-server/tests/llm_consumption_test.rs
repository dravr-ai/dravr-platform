// ABOUTME: Tests for LLM consumption analytics aggregation queries
// ABOUTME: Validates grouping, date range filtering, cost estimation, and tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap (valid in tests per CLAUDE.md guidelines)
#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::ConversationTurnId;
use pierre_database::database::llm_usage::InsertLlmUsage;
use pierre_database::database::repositories::LlmUsageRepository;
use pierre_database::database::test_utils::create_test_db;
use pierre_mcp_server::llm::pricing::calculate_cost;

/// Parameters for inserting test LLM usage data
struct TestUsageParams<'a> {
    tenant_id: &'a str,
    user_id: &'a str,
    provider: &'a str,
    model: &'a str,
    call_type: &'a str,
    prompt_tokens: i64,
    completion_tokens: i64,
}

/// Insert test usage data for a given tenant/user with specified provider, model, and `call_type`
async fn insert_test_usage(db: &dyn LlmUsageRepository, params: &TestUsageParams<'_>) {
    let insert = InsertLlmUsage {
        tenant_id: params.tenant_id,
        user_id: params.user_id,
        conversation_id: None,
        turn_id: ConversationTurnId::new(),
        provider: params.provider,
        model: params.model,
        prompt_tokens: params.prompt_tokens,
        completion_tokens: params.completion_tokens,
        total_tokens: params.prompt_tokens + params.completion_tokens,
        cached_tokens: 0,
        call_type: params.call_type,
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: Some(500),
        cost_usd: 0.0,
        call_sequence: None,
    };
    db.insert_llm_usage(&insert).await.unwrap();
}

#[tokio::test]
async fn test_aggregates_empty_when_no_data() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let aggregates = repos
        .llm_usage
        .get_llm_usage_aggregates("tenant-1", "2020-01-01T00:00:00+00:00")
        .await
        .unwrap();
    assert!(
        aggregates.is_empty(),
        "Expected empty aggregates for no data"
    );

    let daily = repos
        .llm_usage
        .get_llm_usage_daily_series("tenant-1", "2020-01-01T00:00:00+00:00")
        .await
        .unwrap();
    assert!(daily.is_empty(), "Expected empty daily series for no data");
}

#[tokio::test]
async fn test_aggregates_single_provider() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    // Insert 3 chat calls from the same provider/model
    let params = TestUsageParams {
        tenant_id: "t1",
        user_id: "u1",
        provider: "gemini",
        model: "gemini-2.5-flash",
        call_type: "chat",
        prompt_tokens: 100,
        completion_tokens: 50,
    };
    for _ in 0..3 {
        insert_test_usage(repos.llm_usage.as_ref(), &params).await;
    }

    let aggregates = repos
        .llm_usage
        .get_llm_usage_aggregates("t1", "2020-01-01T00:00:00+00:00")
        .await
        .unwrap();

    assert_eq!(aggregates.len(), 1, "Should have 1 aggregate group");
    let row = &aggregates[0];
    assert_eq!(row.provider, "gemini");
    assert_eq!(row.model, "gemini-2.5-flash");
    assert_eq!(row.call_type, "chat");
    assert_eq!(row.total_tokens, 450); // 3 * (100 + 50)
    assert_eq!(row.prompt_tokens, 300); // 3 * 100
    assert_eq!(row.completion_tokens, 150); // 3 * 50
    assert_eq!(row.calls, 3);
}

#[tokio::test]
async fn test_aggregates_multiple_providers_and_call_types() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    // Gemini chat calls
    insert_test_usage(
        repos.llm_usage.as_ref(),
        &TestUsageParams {
            tenant_id: "t1",
            user_id: "u1",
            provider: "gemini",
            model: "gemini-2.5-flash",
            call_type: "chat",
            prompt_tokens: 200,
            completion_tokens: 100,
        },
    )
    .await;
    insert_test_usage(
        repos.llm_usage.as_ref(),
        &TestUsageParams {
            tenant_id: "t1",
            user_id: "u1",
            provider: "gemini",
            model: "gemini-2.5-flash",
            call_type: "chat",
            prompt_tokens: 300,
            completion_tokens: 150,
        },
    )
    .await;

    // Gemini insight calls
    insert_test_usage(
        repos.llm_usage.as_ref(),
        &TestUsageParams {
            tenant_id: "t1",
            user_id: "u1",
            provider: "gemini",
            model: "gemini-2.5-flash",
            call_type: "insight",
            prompt_tokens: 500,
            completion_tokens: 200,
        },
    )
    .await;

    // Groq chat calls
    insert_test_usage(
        repos.llm_usage.as_ref(),
        &TestUsageParams {
            tenant_id: "t1",
            user_id: "u1",
            provider: "groq",
            model: "llama-3.3-70b",
            call_type: "chat",
            prompt_tokens: 1000,
            completion_tokens: 500,
        },
    )
    .await;

    let aggregates = repos
        .llm_usage
        .get_llm_usage_aggregates("t1", "2020-01-01T00:00:00+00:00")
        .await
        .unwrap();

    // Should have 3 groups: (gemini, gemini-2.5-flash, chat), (gemini, gemini-2.5-flash, insight), (groq, llama-3.3-70b, chat)
    assert_eq!(aggregates.len(), 3, "Should have 3 aggregate groups");

    // Results are ordered by total_tokens DESC
    // groq chat: 1500 tokens
    // gemini insight: 700 tokens
    // gemini chat: 750 tokens
    let groq = aggregates.iter().find(|r| r.provider == "groq").unwrap();
    assert_eq!(groq.total_tokens, 1500);
    assert_eq!(groq.calls, 1);

    let gemini_chat = aggregates
        .iter()
        .find(|r| r.provider == "gemini" && r.call_type == "chat")
        .unwrap();
    assert_eq!(gemini_chat.total_tokens, 750);
    assert_eq!(gemini_chat.calls, 2);

    let gemini_insight = aggregates
        .iter()
        .find(|r| r.provider == "gemini" && r.call_type == "insight")
        .unwrap();
    assert_eq!(gemini_insight.total_tokens, 700);
    assert_eq!(gemini_insight.calls, 1);
}

#[tokio::test]
async fn test_tenant_isolation() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    // Insert data for two tenants
    insert_test_usage(
        repos.llm_usage.as_ref(),
        &TestUsageParams {
            tenant_id: "tenant-a",
            user_id: "u1",
            provider: "gemini",
            model: "gemini-2.5-flash",
            call_type: "chat",
            prompt_tokens: 100,
            completion_tokens: 50,
        },
    )
    .await;
    insert_test_usage(
        repos.llm_usage.as_ref(),
        &TestUsageParams {
            tenant_id: "tenant-b",
            user_id: "u2",
            provider: "gemini",
            model: "gemini-2.5-flash",
            call_type: "chat",
            prompt_tokens: 200,
            completion_tokens: 100,
        },
    )
    .await;

    // Query tenant-a — should only see tenant-a's data
    let agg_a = repos
        .llm_usage
        .get_llm_usage_aggregates("tenant-a", "2020-01-01T00:00:00+00:00")
        .await
        .unwrap();
    assert_eq!(agg_a.len(), 1);
    assert_eq!(agg_a[0].total_tokens, 150);

    // Query tenant-b — should only see tenant-b's data
    let agg_b = repos
        .llm_usage
        .get_llm_usage_aggregates("tenant-b", "2020-01-01T00:00:00+00:00")
        .await
        .unwrap();
    assert_eq!(agg_b.len(), 1);
    assert_eq!(agg_b[0].total_tokens, 300);
}

#[tokio::test]
async fn test_daily_series_groups_by_date() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    // Insert multiple records (all on the same day since created_at is "now")
    let params = TestUsageParams {
        tenant_id: "t1",
        user_id: "u1",
        provider: "gemini",
        model: "gemini-2.5-flash",
        call_type: "chat",
        prompt_tokens: 100,
        completion_tokens: 50,
    };
    for _ in 0..5 {
        insert_test_usage(repos.llm_usage.as_ref(), &params).await;
    }

    let daily = repos
        .llm_usage
        .get_llm_usage_daily_series("t1", "2020-01-01T00:00:00+00:00")
        .await
        .unwrap();

    // Should have 1 day since all inserts happen at "now"
    assert_eq!(daily.len(), 1, "Should have 1 day in the series");
    assert_eq!(daily[0].tokens, 750); // 5 * 150
    assert_eq!(daily[0].calls, 5);
    assert!(!daily[0].date.is_empty(), "Date should be populated");
}

#[tokio::test]
async fn test_date_range_filtering() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    // Insert a record "now"
    insert_test_usage(
        repos.llm_usage.as_ref(),
        &TestUsageParams {
            tenant_id: "t1",
            user_id: "u1",
            provider: "gemini",
            model: "gemini-2.5-flash",
            call_type: "chat",
            prompt_tokens: 100,
            completion_tokens: 50,
        },
    )
    .await;

    // Query with a future since date — should get no results
    let future_since = "2099-01-01T00:00:00+00:00";
    let aggregates = repos
        .llm_usage
        .get_llm_usage_aggregates("t1", future_since)
        .await
        .unwrap();
    assert!(
        aggregates.is_empty(),
        "Future since date should return no results"
    );

    // Query with a past since date — should get results
    let past_since = "2020-01-01T00:00:00+00:00";
    let aggregates = repos
        .llm_usage
        .get_llm_usage_aggregates("t1", past_since)
        .await
        .unwrap();
    assert_eq!(aggregates.len(), 1, "Past since date should return results");
}

#[tokio::test]
async fn test_cost_calculation_matches_pricing_module() {
    // Verify that the cost for known models matches the pricing module
    let cost = calculate_cost("gemini", "gemini-2.5-flash", 1_000_000, 1_000_000);
    // input: 1M * 0.15/1M = $0.15, output: 1M * 0.60/1M = $0.60
    let expected = 0.75;
    assert!(
        (cost - expected).abs() < 0.001,
        "Cost {cost} should be close to {expected}"
    );

    // Unknown model returns 0.0
    let unknown_cost = calculate_cost("unknown", "unknown-model", 1_000_000, 1_000_000);
    assert!(
        (unknown_cost - 0.0).abs() < 0.001,
        "Unknown model cost should be 0.0"
    );
}

#[tokio::test]
async fn test_group_by_provider() {
    use pierre_database::database::llm_usage::LlmUsageGroupBy;

    let group = LlmUsageGroupBy::from_str_param("provider");
    assert_eq!(group, Some(LlmUsageGroupBy::Provider));

    let group = LlmUsageGroupBy::from_str_param("model");
    assert_eq!(group, Some(LlmUsageGroupBy::Model));

    let group = LlmUsageGroupBy::from_str_param("call_type");
    assert_eq!(group, Some(LlmUsageGroupBy::CallType));

    let group = LlmUsageGroupBy::from_str_param("invalid");
    assert_eq!(group, None);
}
