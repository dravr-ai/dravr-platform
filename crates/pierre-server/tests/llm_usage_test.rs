// ABOUTME: Tests for the LLM usage tracking database module
// ABOUTME: Validates insert and record integrity for cost analysis data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect (valid in tests per CLAUDE.md guidelines)
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_statements
)]

use std::collections::HashMap;

use pierre_config::admin_types::{ConfigDataType, ConfigScope};
use pierre_core::models::{ConversationTurnId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::llm_usage::InsertLlmUsage;
use pierre_database::database::test_utils::create_test_db;
use pierre_llm::pricing::{
    calculate_cost_with_cache, is_not_per_token_metered, ModelPricing, PricingOverrideMap,
    PricingRegistry, TokenCounts,
};
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::admin::postgres_manager::PostgresAdminConfigManager;
use pierre_mcp_server::config::admin::repository::SetOverrideParams;
use pierre_mcp_server::config::admin::{AdminConfigManager, AdminConfigRepository};

#[tokio::test]
async fn test_insert_llm_usage() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let turn = ConversationTurnId::new();
    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: Some("conv-1"),
        turn_id: turn,
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 150,
        completion_tokens: 50,
        total_tokens: 200,
        cached_tokens: 0,
        cached_write_tokens: 0,
        reasoning_tokens: 0,
        call_type: "chat",
        tool_calls_count: 2,
        tools_called: "[\"get_activities\"]",
        execution_time_ms: Some(1500),
        cost_usd: 0.0,
        call_sequence: None,
    };

    let record = repos.llm_usage.insert_llm_usage(&params).await.unwrap();

    assert!(!record.id.is_empty());
    assert_eq!(record.tenant_id, "tenant-1");
    assert_eq!(record.user_id, "user-1");
    assert_eq!(record.conversation_id, Some("conv-1".to_owned()));
    assert_eq!(record.turn_id, turn);
    assert_eq!(record.provider, "google");
    assert_eq!(record.model, "gemini-2.0-flash-exp");
    assert_eq!(record.prompt_tokens, 150);
    assert_eq!(record.completion_tokens, 50);
    assert_eq!(record.total_tokens, 200);
    assert_eq!(record.call_type, "chat");
    assert_eq!(record.tool_calls_count, 2);
    assert_eq!(record.tools_called, "[\"get_activities\"]");
    assert_eq!(record.execution_time_ms, Some(1500));
    assert!(!record.created_at.is_empty());
}

#[tokio::test]
async fn test_insert_llm_usage_without_conversation() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: None,
        turn_id: ConversationTurnId::new(),
        provider: "openai",
        model: "gpt-4o",
        prompt_tokens: 300,
        completion_tokens: 100,
        total_tokens: 400,
        cached_tokens: 0,
        cached_write_tokens: 0,
        reasoning_tokens: 0,
        call_type: "insight",
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: None,
        cost_usd: 0.0,
        call_sequence: None,
    };

    let record = repos.llm_usage.insert_llm_usage(&params).await.unwrap();

    assert!(record.conversation_id.is_none());
    assert!(record.execution_time_ms.is_none());
    assert_eq!(record.provider, "openai");
    assert_eq!(record.call_type, "insight");
    assert_eq!(record.tools_called, "[]");
}

#[tokio::test]
async fn test_insert_multiple_llm_usage_records() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    for i in 0..3 {
        let params = InsertLlmUsage {
            tenant_id: "tenant-1",
            user_id: "user-1",
            conversation_id: None,
            turn_id: ConversationTurnId::new(),
            provider: "google",
            model: "gemini-2.0-flash-exp",
            prompt_tokens: 100 + i,
            completion_tokens: 50 + i,
            total_tokens: 150 + (2 * i),
            cached_tokens: 0,
            cached_write_tokens: 0,
            reasoning_tokens: 0,
            call_type: "chat",
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: None,
            cost_usd: 0.0,
            call_sequence: None,
        };
        repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    }

    // Verify fourth insert succeeds (proves multiple inserts work)
    let params = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: None,
        turn_id: ConversationTurnId::new(),
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 999,
        completion_tokens: 1,
        total_tokens: 1000,
        cached_tokens: 0,
        cached_write_tokens: 0,
        reasoning_tokens: 0,
        call_type: "chat",
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: None,
        cost_usd: 0.0,
        call_sequence: None,
    };
    let record = repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    assert_eq!(record.total_tokens, 1000);
}

#[tokio::test]
async fn test_find_llm_usage_by_turn_id_returns_empty_when_missing() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let missing = ConversationTurnId::new();
    let rows = repos
        .llm_usage
        .find_llm_usage_by_turn_id(missing)
        .await
        .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn test_find_llm_usage_by_turn_id_returns_all_matching_rows_in_order() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let turn = ConversationTurnId::new();
    for i in 0..3 {
        let params = InsertLlmUsage {
            tenant_id: "tenant-1",
            user_id: "user-1",
            conversation_id: Some("conv-1"),
            turn_id: turn,
            provider: "google",
            model: "gemini-2.0-flash-exp",
            prompt_tokens: 100 + i,
            completion_tokens: 10 + i,
            total_tokens: 110 + (2 * i),
            cached_tokens: 0,
            cached_write_tokens: 0,
            reasoning_tokens: 0,
            call_type: "chat",
            tool_calls_count: 1,
            tools_called: "[\"get_activities\"]",
            execution_time_ms: Some(500 + i),
            cost_usd: 0.0,
            call_sequence: None,
        };
        repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    }

    // Insert an unrelated record with a different turn id to prove isolation
    let other = InsertLlmUsage {
        tenant_id: "tenant-1",
        user_id: "user-1",
        conversation_id: Some("conv-1"),
        turn_id: ConversationTurnId::new(),
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 1,
        completion_tokens: 1,
        total_tokens: 2,
        cached_tokens: 0,
        cached_write_tokens: 0,
        reasoning_tokens: 0,
        call_type: "chat",
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: None,
        cost_usd: 0.0,
        call_sequence: None,
    };
    repos.llm_usage.insert_llm_usage(&other).await.unwrap();

    let rows = repos
        .llm_usage
        .find_llm_usage_by_turn_id(turn)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.turn_id, turn);
        #[allow(clippy::cast_possible_wrap)]
        let expected_prompt = 100i64 + i as i64;
        assert_eq!(row.prompt_tokens, expected_prompt);
    }
}

#[tokio::test]
async fn test_llm_usage_cost_and_cache() {
    // Phase 1 — verifies that cost_usd, cached_tokens, and call_sequence
    // are persisted on every llm_usage row and that the cached-token
    // discount is applied correctly by the pricing module.
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let turn = ConversationTurnId::new();
    let prompt = 1_000i64;
    let cached = 400i64;
    let completion = 500i64;
    let cost_usd =
        calculate_cost_with_cache("gemini", "gemini-2.0-flash", prompt, cached, completion);
    assert!(cost_usd > 0.0, "cost should be non-zero for known pricing");
    let params = InsertLlmUsage {
        tenant_id: "tenant-cost",
        user_id: "user-cost",
        conversation_id: Some("conv-cost"),
        turn_id: turn,
        provider: "gemini",
        model: "gemini-2.0-flash",
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt + completion,
        cached_tokens: cached,
        cached_write_tokens: 0,
        reasoning_tokens: 0,
        call_type: "chat",
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: Some(123),
        cost_usd,
        call_sequence: Some(1),
    };
    let record = repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    assert_eq!(record.cached_tokens, cached);
    assert_eq!(record.call_sequence, Some(1));
    assert!((record.cost_usd - cost_usd).abs() < 1e-12);

    // Verify the cached-token discount actually applied (cached portion
    // bills at 25% vs full input rate).
    let no_cache_cost =
        calculate_cost_with_cache("gemini", "gemini-2.0-flash", prompt, 0, completion);
    assert!(
        cost_usd < no_cache_cost,
        "cache discount should reduce billed cost"
    );
}

#[tokio::test]
async fn test_subscription_and_self_hosted_providers_zero_cost_without_undercount_warning() {
    // P2-8 — self-hosted (ollama/vllm/local) and flat-rate subscription CLI
    // runners (claude-code/cursor-agent/copilot/...) bill $0 per token *by
    // design*. They must (a) resolve to exactly $0 and (b) be classified as
    // not-per-token-metered so the cost path does NOT emit the misleading
    // "No pricing data ... undercount" warning that real misses get.
    let registry = PricingRegistry::new();
    let subscription_and_self_hosted = [
        "ollama",
        "vllm",
        "local",
        "localai",
        "claude-code",
        "cursor-agent",
        "copilot",
        "opencode",
        "codex",
        "goose",
        "cline",
        "continue",
        "warp_cli",
        "kiro",
        "kilo",
    ];
    for provider in subscription_and_self_hosted {
        assert!(
            is_not_per_token_metered(provider),
            "{provider} must be classified as not-per-token-metered (no undercount warning)"
        );
        // A real model under each provider resolves to $0 — the correct cost,
        // not a missing-price fallback.
        let cost = registry.calculate_cost(
            None,
            provider,
            "some-model-x",
            &TokenCounts::new(10_000, 10_000),
        );
        assert!(
            cost.abs() < f64::EPSILON,
            "{provider} should bill $0 per token by design; got {cost}"
        );
    }

    // A genuinely metered provider must NOT be classified as zero-by-design.
    assert!(
        !is_not_per_token_metered("openai_api"),
        "metered API providers must not be treated as subscription/self-hosted"
    );
    assert!(
        !is_not_per_token_metered("gemini"),
        "metered API providers must not be treated as subscription/self-hosted"
    );
}

#[tokio::test]
async fn test_known_priced_provider_resolves_to_its_price() {
    // P2-8 — a known-priced provider/model must resolve to a non-zero,
    // exactly-computed cost. OpenRouter's default slug is now in the table,
    // closing the silent-undercount gap for the gateway's common models.
    let registry = PricingRegistry::new();

    // OpenRouter default model: input 0.12/M, output 0.30/M.
    let cost = registry.calculate_cost(
        None,
        "openrouter",
        "meta-llama/llama-3.3-70b-instruct",
        &TokenCounts::new(1_000_000, 1_000_000),
    );
    let expected = 0.12 + 0.30;
    assert!(
        (cost - expected).abs() < 1e-9,
        "OpenRouter default model should be priced; expected {expected}, got {cost}"
    );
    assert!(
        !is_not_per_token_metered("openrouter"),
        "OpenRouter is metered per model, not subscription/self-hosted"
    );

    // A direct-API priced provider still resolves to its price.
    let gemini = registry.calculate_cost(
        None,
        "gemini",
        "gemini-2.0-flash",
        &TokenCounts::new(1_000_000, 0),
    );
    assert!(
        (gemini - 0.075).abs() < 1e-9,
        "Gemini flash input price should resolve; got {gemini}"
    );
}

#[tokio::test]
async fn test_admin_pricing_override() {
    // Phase 1 — PricingRegistry global override layer wins over the
    // compile-time PRICING_TABLE for the same (provider, model_prefix).
    let registry = PricingRegistry::new();

    // Without overrides the registry falls through to the compile-time
    // table — Gemini flash @ ~$0.075/M input, $0.30/M output.
    let baseline = registry.calculate_cost(
        None,
        "gemini",
        "gemini-2.0-flash",
        &TokenCounts::new(1_000, 1_000),
    );
    assert!(baseline > 0.0);

    // Install a global override that triples both rates.
    let mut overrides: PricingOverrideMap = HashMap::new();
    overrides.insert(
        ("gemini".to_owned(), "gemini-2.0-flash".to_owned()),
        ModelPricing::new(0.225, 0.9),
    );
    registry.replace_global(overrides);
    let after_override = registry.calculate_cost(
        None,
        "gemini",
        "gemini-2.0-flash",
        &TokenCounts::new(1_000, 1_000),
    );
    assert!(
        3.0_f64.mul_add(-baseline, after_override).abs() < 1e-6,
        "override should triple the cost; baseline={baseline} after={after_override}"
    );
}

#[tokio::test]
async fn test_admin_pricing_loader_round_trip() {
    // Phase 2 — write a cat_llm_pricing override row, run the loader, then
    // verify GLOBAL_PRICING_REGISTRY returns the overridden price.
    use pierre_llm::pricing::GLOBAL_PRICING_REGISTRY;
    use pierre_services::pricing_loader;

    let db = create_test_db().await.unwrap();

    // FK target: admin_config_overrides.created_by references users(id), so
    // the override is attributed to a persisted admin.
    let admin = User::new(
        format!("admin-{}@test.local", uuid::Uuid::new_v4()),
        "x".to_owned(),
        None,
    );
    let admin_id = admin.id.to_string();
    db.repositories().users.create(&admin).await.unwrap();

    // The row exactly as `PUT /api/admin/config` writes it: a system-wide
    // (tenant-less) override whose value is the pricing payload as JSON text.
    let payload = serde_json::json!({"input_per_million": 0.999, "output_per_million": 9.99});
    let repo: Box<dyn AdminConfigRepository> = match &db {
        Database::SQLite(sqlite) => Box::new(AdminConfigManager::new(sqlite.pool().clone())),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => Box::new(PostgresAdminConfigManager::new(pg.pool().clone())),
    };
    repo.set_override(SetOverrideParams {
        category: "cat_llm_pricing",
        key: "gemini.gemini-2.0-flash",
        value: &payload,
        data_type: ConfigDataType::String,
        admin_user_id: &admin_id,
        scope: ConfigScope::Global,
        reason: Some("pricing loader round trip"),
    })
    .await
    .unwrap();

    pricing_loader::load_pricing_overrides(db.repositories().llm_credentials.as_ref()).await;

    // Compute under the registry: with our override the input rate is 0.999/M,
    // so 1_000 prompt tokens at full rate ≈ 0.999/1000 = 0.000999.
    let cost = GLOBAL_PRICING_REGISTRY.calculate_cost(
        None,
        "gemini",
        "gemini-2.0-flash",
        &TokenCounts::new(1_000, 0),
    );
    assert!(
        (cost - 0.000_999).abs() < 1e-9,
        "expected override to apply; got cost={cost}"
    );
}

#[tokio::test]
async fn cache_write_and_reasoning_counts_survive_a_round_trip() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let turn = ConversationTurnId::new();
    let params = InsertLlmUsage {
        tenant_id: "tenant-rt",
        user_id: "user-rt",
        conversation_id: Some("conv-rt"),
        turn_id: turn,
        provider: "copilot_headless",
        model: "claude-opus-4",
        prompt_tokens: 27_862,
        completion_tokens: 4,
        total_tokens: 27_866,
        // The real ACP payload captured 2026-08-27.
        cached_tokens: 15_320,
        cached_write_tokens: 12_540,
        reasoning_tokens: 640,
        call_type: "chat",
        tool_calls_count: 1,
        tools_called: "[\"get_activities\"]",
        execution_time_ms: Some(13_500),
        cost_usd: 0.0,
        call_sequence: Some(1),
    };
    repos.llm_usage.insert_llm_usage(&params).await.unwrap();

    let rows = repos
        .llm_usage
        .get_recent_llm_calls_admin(10)
        .await
        .unwrap();
    let row = rows
        .iter()
        .find(|r| r.user_id == "user-rt")
        .expect("inserted row should come back from the admin query");

    assert_eq!(
        row.cached_tokens, 15_320,
        "cache-read count did not persist"
    );
    assert_eq!(
        row.cached_write_tokens, 12_540,
        "cache-write count did not persist"
    );
    assert_eq!(row.reasoning_tokens, 640, "reasoning count did not persist");
}

/// The aggregate query sums the new columns rather than dropping them, so the
/// "is the prefix churning?" question is answerable from the usage tables.
#[tokio::test]
async fn aggregates_sum_cache_write_and_reasoning_counts() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    for _ in 0..3 {
        let params = InsertLlmUsage {
            tenant_id: "tenant-agg-cw",
            user_id: "user-agg-cw",
            conversation_id: None,
            turn_id: ConversationTurnId::new(),
            provider: "copilot_headless",
            model: "claude-opus-4",
            prompt_tokens: 1_000,
            completion_tokens: 10,
            total_tokens: 1_010,
            cached_tokens: 100,
            cached_write_tokens: 200,
            reasoning_tokens: 30,
            call_type: "chat",
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: Some(10),
            cost_usd: 0.0,
            call_sequence: None,
        };
        repos.llm_usage.insert_llm_usage(&params).await.unwrap();
    }

    let aggregates = repos
        .llm_usage
        .get_llm_usage_aggregates("tenant-agg-cw", "1970-01-01T00:00:00Z")
        .await
        .unwrap();
    let row = aggregates
        .iter()
        .find(|r| r.model == "claude-opus-4")
        .expect("aggregate row for the inserted model");

    assert_eq!(row.calls, 3);
    assert_eq!(row.cached_tokens, 300, "3 x 100 cache reads");
    assert_eq!(row.cached_write_tokens, 600, "3 x 200 cache writes");
    assert_eq!(row.reasoning_tokens, 90, "3 x 30 reasoning tokens");
}
