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

use pierre_core::models::usage::InsertEmbeddingUsage;
use pierre_core::models::ConversationTurnId;
use pierre_database::database::llm_usage::InsertLlmUsage;
use pierre_database::database::test_utils::create_test_db;
use pierre_llm::pricing::{
    calculate_cost_with_cache, ModelPricing, PricingOverrideMap, PricingRegistry,
};
use sqlx::Row;

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
async fn test_embedding_usage_insert() {
    // Phase 1 — embedding_usage rows persist independently of llm_usage
    // so embedding volume never inflates chat-token billing aggregates.
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let params = InsertEmbeddingUsage {
        tenant_id: "tenant-emb",
        user_id: "user-emb",
        provider: "gemini",
        model: "text-embedding-004",
        input_tokens: 64,
        cost_usd: 0.000_4,
    };
    let record = repos
        .llm_usage
        .insert_embedding_usage(&params)
        .await
        .unwrap();
    assert_eq!(record.tenant_id, "tenant-emb");
    assert_eq!(record.user_id, "user-emb");
    assert_eq!(record.provider, "gemini");
    assert_eq!(record.model, "text-embedding-004");
    assert_eq!(record.input_tokens, 64);
    assert!((record.cost_usd - 0.000_4).abs() < 1e-9);
    assert!(!record.id.is_empty());
    assert!(!record.created_at.is_empty());
}

#[tokio::test]
async fn test_admin_pricing_override() {
    // Phase 1 — PricingRegistry global override layer wins over the
    // compile-time PRICING_TABLE for the same (provider, model_prefix).
    let registry = PricingRegistry::new();

    // Without overrides the registry falls through to the compile-time
    // table — Gemini flash @ ~$0.075/M input, $0.30/M output.
    let baseline = registry.calculate_cost(None, "gemini", "gemini-2.0-flash", 1_000, 0, 1_000);
    assert!(baseline > 0.0);

    // Install a global override that triples both rates.
    let mut overrides: PricingOverrideMap = HashMap::new();
    overrides.insert(
        ("gemini".to_owned(), "gemini-2.0-flash".to_owned()),
        ModelPricing {
            input_per_million: 0.225,
            output_per_million: 0.9,
        },
    );
    registry.replace_global(overrides);
    let after_override =
        registry.calculate_cost(None, "gemini", "gemini-2.0-flash", 1_000, 0, 1_000);
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
    use pierre_mcp_server::services::pricing_loader;

    let db = create_test_db().await.unwrap();
    let pool = db.sqlite_pool().expect("test db is sqlite");

    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let admin_id = uuid::Uuid::new_v4().to_string();
    // FK target: users(id). Insert a minimal admin row so the override
    // satisfies the CHECK + FK constraints.
    sqlx::query(
        r"INSERT INTO users (id, email, password_hash, tier, created_at, last_active)
          VALUES (?1, ?2, 'x', 'enterprise', ?3, ?3)",
    )
    .bind(&admin_id)
    .bind(format!("admin-{admin_id}@test.local"))
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r"INSERT INTO admin_config_overrides
            (id, category, config_key, config_value, data_type, tenant_id, created_by, created_at, updated_at)
          VALUES (?1, 'cat_llm_pricing', 'gemini.gemini-2.0-flash', ?2, 'string', NULL, ?3, ?4, ?4)",
    )
    .bind(&id)
    .bind(r#"{"input_per_million": 0.999, "output_per_million": 9.99}"#)
    .bind(&admin_id)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();

    pricing_loader::load_pricing_overrides(db.repositories().llm_credentials.as_ref()).await;

    // Compute under the registry: with our override the input rate is 0.999/M,
    // so 1_000 prompt tokens at full rate ≈ 0.999/1000 = 0.000999.
    let cost =
        GLOBAL_PRICING_REGISTRY.calculate_cost(None, "gemini", "gemini-2.0-flash", 1_000, 0, 0);
    assert!(
        (cost - 0.000_999).abs() < 1e-9,
        "expected override to apply; got cost={cost}"
    );
}

#[tokio::test]
async fn test_embedding_sink_injection_records_via_instrumented_provider() {
    // Phase 1+3 — build an InstrumentedEmbeddingProvider over a stub inner
    // provider + the RepositoryEmbeddingSink and verify embed_for() persists
    // a row into embedding_usage.
    use async_trait::async_trait;
    use pierre_core::errors::AppResult;
    use pierre_llm::embeddings::{
        EmbeddingDim, EmbeddingProvider, EmbeddingUsageSink, InstrumentedEmbeddingProvider,
    };
    use pierre_mcp_server::services::embedding_sink::RepositoryEmbeddingSink;
    use std::sync::Arc;

    struct StubProvider;

    #[async_trait]
    impl EmbeddingProvider for StubProvider {
        fn name(&self) -> &'static str {
            "gemini"
        }
        fn model(&self) -> &'static str {
            "text-embedding-004"
        }
        fn dimensions(&self) -> EmbeddingDim {
            8
        }
        async fn embed(&self, _text: &str) -> AppResult<Vec<f32>> {
            Ok(vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8])
        }
    }

    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let sink: Arc<dyn EmbeddingUsageSink> =
        Arc::new(RepositoryEmbeddingSink::new(Arc::clone(&repos.llm_usage)));
    let provider = InstrumentedEmbeddingProvider::new(Box::new(StubProvider), sink);

    let v = provider
        .embed_for("tenant-emb-x", "user-emb-x", "the quick brown fox")
        .await
        .unwrap();
    assert_eq!(v.len(), 8);

    // The sink wrote a row — we don't have a list-by-tenant query, but a
    // direct count via the pool confirms persistence.
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM embedding_usage WHERE tenant_id = ? AND user_id = ?",
    )
    .bind("tenant-emb-x")
    .bind("user-emb-x")
    .fetch_one(db.sqlite_pool().expect("test db is sqlite"))
    .await
    .unwrap();
    let count: i64 = row.get("n");
    assert_eq!(count, 1);
}

// Outbound-queue user_id round-trip lives in
// `messaging_repository_test.rs::test_enqueue_outbound_round_trips_user_id`
// — it reuses that file's existing FK-chain fixture (tenant + user +
// session + message) instead of hand-rolling another one here.
