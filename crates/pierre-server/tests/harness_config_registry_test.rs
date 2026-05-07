// ABOUTME: Integration tests for the harness config registry — DB load, install, runtime projections
// ABOUTME: Proves that admin PUT changes flow into the chat pipeline's compaction + guardrails inputs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_database::backends::factory::Database;
use pierre_mcp_server::harness_config_registry::{
    HarnessConfigRegistry, HarnessConfigSnapshot, HarnessConfigSource,
};
use pierre_mcp_server::routes::admin::harness_config::{
    HarnessConfigDocument, HARNESS_CONFIG_SETTING_KEY,
};

const ENCRYPTION_KEY: &[u8; 32] = b"test_encryption_key_32_bytes_lng";

/// Build an in-memory `SQLite` database for a single-test scope.
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
async fn from_database_with_no_row_returns_compiled_in_defaults() {
    let db = fresh_database().await;
    let registry = HarnessConfigRegistry::from_database(&db).await;

    let snapshot: HarnessConfigSnapshot = registry.current();
    assert_eq!(snapshot.source, HarnessConfigSource::Defaults);
    assert_eq!(snapshot.compaction.window_tokens, 128_000);
    assert!((snapshot.compaction.warn_threshold - 0.70).abs() < f32::EPSILON);
    assert_eq!(snapshot.guardrails.max_response_chars, 5_000);
    assert!(snapshot
        .guardrails
        .disclaimer_triggers
        .iter()
        .any(|t| t == "injury"));
}

#[tokio::test]
async fn from_database_loads_persisted_row() {
    let db = fresh_database().await;

    let mut doc = HarnessConfigDocument::default();
    doc.compaction.window_tokens = 64_000;
    doc.compaction.warn_threshold = 0.5;
    doc.compaction.emergency_threshold = 0.8;
    doc.compaction.summarize_oldest_n = 4;
    doc.compaction.sliding_drop_n = 2;
    doc.guardrails.max_response_chars = 1_500;
    doc.guardrails.blocked_topics = vec!["politics".to_owned()];
    doc.guardrails.disclaimer_triggers = vec!["surgery".to_owned()];
    doc.guardrails.disclaimer_text = "**Test disclaimer.**".to_owned();

    let serialized = serde_json::to_string(&doc).expect("serialize doc");
    db.set_system_setting(HARNESS_CONFIG_SETTING_KEY, &serialized)
        .await
        .expect("persist row");

    let registry = HarnessConfigRegistry::from_database(&db).await;
    let snapshot = registry.current();

    assert_eq!(snapshot.source, HarnessConfigSource::Database);
    assert_eq!(snapshot.compaction.window_tokens, 64_000);
    assert!((snapshot.compaction.warn_threshold - 0.5).abs() < f32::EPSILON);
    assert!((snapshot.compaction.emergency_threshold - 0.8).abs() < f32::EPSILON);
    assert_eq!(snapshot.compaction.summarize_oldest_n, 4);
    assert_eq!(snapshot.compaction.sliding_drop_n, 2);

    assert_eq!(snapshot.guardrails.max_response_chars, 1_500);
    assert_eq!(snapshot.guardrails.blocked_topics, vec!["politics"]);
    assert_eq!(snapshot.guardrails.disclaimer_triggers, vec!["surgery"]);
    assert_eq!(snapshot.guardrails.disclaimer_text, "**Test disclaimer.**");
}

#[tokio::test]
async fn from_database_with_corrupt_row_falls_back_to_defaults() {
    let db = fresh_database().await;
    db.set_system_setting(HARNESS_CONFIG_SETTING_KEY, "{not valid json")
        .await
        .expect("persist row");

    let registry = HarnessConfigRegistry::from_database(&db).await;
    let snapshot = registry.current();

    assert_eq!(snapshot.source, HarnessConfigSource::Defaults);
    assert_eq!(snapshot.compaction.window_tokens, 128_000);
}

#[test]
fn install_swaps_runtime_projections_atomically() {
    let registry = HarnessConfigRegistry::bootstrap();
    let baseline = registry.current_compaction();
    assert_eq!(baseline.window_tokens, 128_000);
    let baseline_guardrails = registry.current_guardrails();
    assert_eq!(baseline_guardrails.max_response_chars, 5_000);

    let mut next = HarnessConfigDocument::default();
    next.compaction.window_tokens = 32_000;
    next.compaction.warn_threshold = 0.4;
    next.compaction.emergency_threshold = 0.9;
    next.guardrails.max_response_chars = 250;
    next.guardrails.blocked_topics = vec!["doping".to_owned()];

    registry.install(next, HarnessConfigSource::AdminUpdate);

    let updated = registry.current_compaction();
    assert_eq!(updated.window_tokens, 32_000);
    assert!((updated.warn_threshold - 0.4).abs() < f32::EPSILON);
    assert!((updated.emergency_threshold - 0.9).abs() < f32::EPSILON);

    let guardrails = registry.current_guardrails();
    assert_eq!(guardrails.max_response_chars, 250);
    assert_eq!(guardrails.blocked_topics, vec!["doping"]);

    // The `Arc` returned by `current_guardrails` must reflect the new
    // snapshot — a stale clone from before `install` would prove the
    // registry leaks the previous projection across a swap.
    assert_ne!(
        baseline_guardrails.max_response_chars,
        guardrails.max_response_chars
    );
}

#[test]
fn current_compaction_feeds_into_compactor_constructor() {
    use pierre_mcp_server::services::conversation_compaction::ConversationCompactor;

    let registry = HarnessConfigRegistry::bootstrap();
    let mut doc = HarnessConfigDocument::default();
    doc.compaction.window_tokens = 10_000;
    doc.compaction.warn_threshold = 0.5;
    doc.compaction.emergency_threshold = 0.95;
    doc.compaction.summarize_oldest_n = 3;
    doc.compaction.sliding_drop_n = 2;
    registry.install(doc, HarnessConfigSource::AdminUpdate);

    // The chat pipeline does exactly this read on every turn — if the
    // compactor cannot be constructed from the registry's projection, the
    // dispatch wiring is broken.
    let _compactor = ConversationCompactor::new(registry.current_compaction());

    let cfg = registry.current_compaction();
    assert_eq!(cfg.window_tokens, 10_000);
    assert_eq!(cfg.summarize_oldest_n, 3);
    assert_eq!(cfg.sliding_drop_n, 2);
}

#[test]
fn current_guardrails_blocks_a_topic_after_install() {
    use pierre_mcp_server::config::text_guardrails::{GuardrailOutcome, GuardrailRejection};

    let registry = HarnessConfigRegistry::bootstrap();

    // Default guardrails have no blocked topics.
    let baseline = registry.current_guardrails();
    let baseline_outcome = baseline.apply("we should discuss politics openly");
    match baseline_outcome {
        GuardrailOutcome::Allowed(_) => {}
        GuardrailOutcome::Rejected(_) => panic!("default config has no blocked topics"),
    }

    // After install, the same input must trip the new blocked-topic rule.
    let mut doc = HarnessConfigDocument::default();
    doc.guardrails.blocked_topics = vec!["politics".to_owned()];
    registry.install(doc, HarnessConfigSource::AdminUpdate);

    let updated = registry.current_guardrails();
    let outcome = updated.apply("we should discuss politics openly");
    match outcome {
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            assert_eq!(topic, "politics");
        }
        other => panic!("expected BlockedTopic rejection, got {other:?}"),
    }
}
