// ABOUTME: Integration tests for the myth-busting → harness blocked-topics feedback loop
// ABOUTME: Proves promote_topic mutates the registry so chat replies start tripping guardrails
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_database::backends::factory::Database;
use pierre_mcp_server::config::text_guardrails::{GuardrailOutcome, GuardrailRejection};
use pierre_mcp_server::harness_config_registry::HarnessConfigRegistry;
use pierre_mcp_server::routes::admin::harness_config::HARNESS_CONFIG_SETTING_KEY;
use pierre_mcp_server::routes::admin::myth_busting::promote_topic;

const ENCRYPTION_KEY: &[u8; 32] = b"test_encryption_key_32_bytes_lng";

#[cfg(not(feature = "postgresql"))]
async fn fresh_database() -> Database {
    Database::new("sqlite::memory:", ENCRYPTION_KEY.to_vec())
        .await
        .expect("in-memory sqlite database")
}

#[cfg(feature = "postgresql")]
async fn fresh_database() -> Database {
    use pierre_mcp_server::config::environment::PostgresPoolConfig;
    Database::new(
        "sqlite::memory:",
        ENCRYPTION_KEY.to_vec(),
        &PostgresPoolConfig::default(),
    )
    .await
    .expect("in-memory sqlite database")
}

#[tokio::test]
async fn promote_topic_blocks_subsequent_replies() {
    let db = fresh_database().await;
    let registry = Arc::new(HarnessConfigRegistry::from_database(&db).await);

    // Default snapshot has no blocked_topics.
    let baseline = registry.current_guardrails();
    assert!(baseline.blocked_topics.is_empty());

    // Replies that mention "creatine causes kidney damage" pass before
    // promotion (only the medical-disclaimer triggers fire, and only on
    // certain other keywords).
    let baseline_outcome =
        baseline.apply("Some say creatine causes kidney damage in healthy adults.");
    assert!(matches!(baseline_outcome, GuardrailOutcome::Allowed(_)));

    // Promote the claim. The handler-shared helper runs the full
    // read-mutate-validate-persist-install cycle.
    let response = promote_topic(&db, &registry, "creatine causes kidney damage")
        .await
        .expect("promote_topic should succeed");
    assert!(response.added);
    assert_eq!(response.total_blocked_topics, 1);

    // After promotion the registry's runtime guardrails reject the same input.
    let updated = registry.current_guardrails();
    assert_eq!(updated.blocked_topics.len(), 1);
    let outcome = updated.apply("Some say creatine causes kidney damage in healthy adults.");
    match outcome {
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            assert_eq!(topic, "creatine causes kidney damage");
        }
        other => panic!("expected BlockedTopic rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn promote_topic_persists_to_system_settings() {
    let db = fresh_database().await;
    let registry = Arc::new(HarnessConfigRegistry::from_database(&db).await);

    promote_topic(&db, &registry, "homeopathy cures diabetes")
        .await
        .expect("promote should succeed");

    // Re-read the persisted row directly — proves we wrote to storage,
    // not just to the in-memory registry. A registry-only swap would
    // be lost on the next restart.
    let row = db
        .get_system_setting(HARNESS_CONFIG_SETTING_KEY)
        .await
        .expect("get_system_setting")
        .expect("row should exist after promote");
    assert!(row.value.contains("homeopathy cures diabetes"));
}

#[tokio::test]
async fn promote_topic_is_idempotent() {
    let db = fresh_database().await;
    let registry = Arc::new(HarnessConfigRegistry::from_database(&db).await);

    let first = promote_topic(&db, &registry, "stretching prevents injury")
        .await
        .expect("first promote");
    assert!(first.added);

    // Casing differs but comparison is case-insensitive — the second
    // call should be a no-op (added=false), avoiding registry churn.
    let second = promote_topic(&db, &registry, "Stretching Prevents Injury")
        .await
        .expect("second promote");
    assert!(!second.added);
    assert_eq!(second.total_blocked_topics, 1);

    let snapshot = registry.current();
    assert_eq!(snapshot.guardrails.blocked_topics.len(), 1);
}

#[tokio::test]
async fn promote_topic_rejects_empty_input() {
    let db = fresh_database().await;
    let registry = Arc::new(HarnessConfigRegistry::from_database(&db).await);

    let err = promote_topic(&db, &registry, "   ").await.unwrap_err();
    assert!(
        format!("{err}").contains("non-empty"),
        "expected non-empty error, got: {err}"
    );
}

#[tokio::test]
async fn promote_topic_rejects_overlong_input() {
    let db = fresh_database().await;
    let registry = Arc::new(HarnessConfigRegistry::from_database(&db).await);

    let too_long = "x".repeat(250);
    let err = promote_topic(&db, &registry, &too_long).await.unwrap_err();
    assert!(
        format!("{err}").contains("exceeds"),
        "expected length error, got: {err}"
    );
}

#[tokio::test]
async fn promote_topic_survives_round_trip_through_from_database() {
    let db = fresh_database().await;
    let registry = Arc::new(HarnessConfigRegistry::from_database(&db).await);

    promote_topic(&db, &registry, "vegan diets cause anemia")
        .await
        .expect("promote should succeed");

    // Drop the registry and rebuild from the database — the topic should
    // still be in the loaded guardrails. Proves promote writes durable
    // state, not just in-memory state.
    let revived = HarnessConfigRegistry::from_database(&db).await;
    let topics = &revived.current_guardrails().blocked_topics;
    assert!(
        topics.iter().any(|t| t == "vegan diets cause anemia"),
        "promoted topic should survive a registry rebuild"
    );
}
