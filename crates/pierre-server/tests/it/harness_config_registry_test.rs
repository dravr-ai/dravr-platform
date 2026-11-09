// ABOUTME: Integration tests for the harness config registry — DB load, install, runtime projections
// ABOUTME: Proves that admin PUT changes flow into the chat pipeline's compaction + guardrails inputs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;
use std::sync::Arc;

use pierre_contremaitre::harness_config_document::{
    HarnessConfigDocument, HARNESS_CONFIG_SETTING_KEY,
};
use pierre_contremaitre::harness_config_registry::{
    HarnessConfigRegistry, HarnessConfigSnapshot, HarnessConfigSource, HarnessReloadOutcome,
};
use pierre_contremaitre::text_guardrails::LocaleGuardrails;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_test_support::db::create_test_db_with_key;

const ENCRYPTION_KEY: &[u8; 32] = b"test_encryption_key_32_bytes_lng";

/// Open a fresh database for a single-test scope.
async fn fresh_database() -> Database {
    create_test_db_with_key(ENCRYPTION_KEY.to_vec())
        .await
        .expect("test database")
}

#[tokio::test]
async fn from_database_with_no_row_returns_compiled_in_defaults() {
    let db = fresh_database().await;
    let registry = HarnessConfigRegistry::from_database(&db).await;

    let snapshot: Arc<HarnessConfigSnapshot> = registry.current();
    assert_eq!(snapshot.source, HarnessConfigSource::Defaults);
    assert_eq!(snapshot.compaction.window_tokens, 1_000_000);
    assert!((snapshot.compaction.warn_threshold - 0.0896).abs() < f32::EPSILON);
    assert_eq!(snapshot.guardrails.blocked_topics, Vec::<String>::new());
    // Default ships triggers for every supported locale; sanity-check
    // both the English and the French entries so the regression case
    // (English-only triggers) cannot reappear silently.
    assert!(snapshot.guardrails.locales.contains_key("en"));
    assert!(snapshot.guardrails.locales.contains_key("fr"));
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
    doc.guardrails.blocked_topics = vec!["politics".to_owned()];
    let mut custom = HashMap::new();
    custom.insert(
        "en".to_owned(),
        LocaleGuardrails {
            disclaimer_triggers: vec!["surgery".to_owned()],
        },
    );
    doc.guardrails.locales = custom;

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

    assert_eq!(snapshot.guardrails.blocked_topics, vec!["politics"]);
    let en = snapshot
        .guardrails
        .locales
        .get("en")
        .expect("en locale must round-trip");
    assert!(
        en.triggers_re
            .as_ref()
            .is_some_and(|re| re.is_match("scheduled for SURGERY next week")),
        "compiled trigger regex must match the surgery keyword case-insensitively"
    );
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
    assert_eq!(snapshot.compaction.window_tokens, 1_000_000);
}

#[test]
fn install_swaps_runtime_projections_atomically() {
    let registry = HarnessConfigRegistry::bootstrap();
    let baseline = registry.current_compaction();
    assert_eq!(baseline.window_tokens, 1_000_000);
    let baseline_guardrails = registry.current_guardrails();
    assert_eq!(baseline_guardrails.blocked_topics, Vec::<String>::new());

    let mut next = HarnessConfigDocument::default();
    next.compaction.window_tokens = 32_000;
    next.compaction.warn_threshold = 0.4;
    next.compaction.emergency_threshold = 0.9;
    next.guardrails.blocked_topics = vec!["doping".to_owned()];

    registry.install(next, HarnessConfigSource::AdminUpdate);

    let updated = registry.current_compaction();
    assert_eq!(updated.window_tokens, 32_000);
    assert!((updated.warn_threshold - 0.4).abs() < f32::EPSILON);
    assert!((updated.emergency_threshold - 0.9).abs() < f32::EPSILON);

    let guardrails = registry.current_guardrails();
    assert_eq!(guardrails.blocked_topics, vec!["doping"]);

    // The `Arc` returned by `current_guardrails` must reflect the new
    // snapshot — a stale clone from before `install` would prove the
    // registry leaks the previous projection across a swap.
    assert_ne!(
        baseline_guardrails.blocked_topics,
        guardrails.blocked_topics
    );
}

#[test]
fn current_compaction_feeds_into_compactor_constructor() {
    use pierre_services::conversation_compaction::ConversationCompactor;

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
    use pierre_contremaitre::text_guardrails::{GuardrailOutcome, GuardrailRejection};

    let registry = HarnessConfigRegistry::bootstrap();

    // Default guardrails have no blocked topics.
    let baseline = registry.current_guardrails();
    let baseline_outcome = baseline.apply("we should discuss politics openly", "en");
    assert!(
        !matches!(baseline_outcome, GuardrailOutcome::Rejected(_)),
        "default config has no blocked topics"
    );

    // After install, the same input must trip the new blocked-topic rule.
    let mut doc = HarnessConfigDocument::default();
    doc.guardrails.blocked_topics = vec!["politics".to_owned()];
    registry.install(doc, HarnessConfigSource::AdminUpdate);

    let updated = registry.current_guardrails();
    let outcome = updated.apply("we should discuss politics openly", "en");
    match outcome {
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            assert_eq!(topic, "politics");
        }
        other => panic!("expected BlockedTopic rejection, got {other:?}"),
    }
}

fn topic_document(topic: &str) -> HarnessConfigDocument {
    let mut document = HarnessConfigDocument::default();
    document.guardrails.blocked_topics = vec![topic.to_owned()];
    document
}

async fn write_row(db: &Database, document: &HarnessConfigDocument) {
    db.set_system_setting(
        HARNESS_CONFIG_SETTING_KEY,
        &serde_json::to_string(document).expect("document serializes"),
    )
    .await
    .expect("row written");
}

/// A save lands on one instance. The other hears about it through its poll tick.
#[tokio::test]
async fn two_instances_over_one_database_converge_through_the_poll_tick() {
    let db = fresh_database().await;
    let serving = HarnessConfigRegistry::from_database(&db).await;
    let other = HarnessConfigRegistry::from_database(&db).await;

    write_row(&db, &topic_document("doping")).await;
    serving.install(topic_document("doping"), HarnessConfigSource::AdminUpdate);

    assert_eq!(
        other.current_guardrails().blocked_topics,
        Vec::<String>::new()
    );
    assert_eq!(
        other.reload_from_database(&db).await,
        HarnessReloadOutcome::Installed
    );
    assert_eq!(other.current_guardrails().blocked_topics, ["doping"]);
    assert_eq!(other.current().source, HarnessConfigSource::Database);

    assert_eq!(
        other.reload_from_database(&db).await,
        HarnessReloadOutcome::Unchanged
    );
    assert_eq!(
        serving.reload_from_database(&db).await,
        HarnessReloadOutcome::Unchanged,
        "the instance that took the save already holds the row"
    );
}

/// A parse-failed row must never reload an instance into the defaults: that would
/// silently lift every blocked topic on a running fleet.
#[tokio::test]
async fn a_corrupt_row_keeps_the_values_in_force_until_it_is_repaired() {
    let db = fresh_database().await;
    write_row(&db, &topic_document("doping")).await;
    let registry = HarnessConfigRegistry::from_database(&db).await;

    db.set_system_setting(HARNESS_CONFIG_SETTING_KEY, "{not json")
        .await
        .expect("row written");
    assert_eq!(
        registry.reload_from_database(&db).await,
        HarnessReloadOutcome::KeptOnInvalidRow
    );
    assert_eq!(registry.current_guardrails().blocked_topics, ["doping"]);
    assert!(registry.current().load_failure.is_some());

    write_row(&db, &topic_document("doping")).await;
    assert_eq!(
        registry.reload_from_database(&db).await,
        HarnessReloadOutcome::Installed,
        "repairing the row to the document in force still clears the mark"
    );
    assert!(registry.current().load_failure.is_none());
}

/// A save validates and a tick must too: a hand-edited row that parses but breaks a
/// rule used to be installed, with warn above emergency or a summary that can never
/// run. It is treated like a row that does not parse.
#[tokio::test]
async fn a_row_a_save_would_refuse_keeps_the_values_in_force() {
    let db = fresh_database().await;
    write_row(&db, &topic_document("doping")).await;
    let registry = HarnessConfigRegistry::from_database(&db).await;

    let mut invalid = HarnessConfigDocument::default();
    invalid.guardrails.blocked_topics = vec!["other".to_owned()];
    invalid.compaction.summarize_oldest_n = 0;
    write_row(&db, &invalid).await;

    assert_eq!(
        registry.reload_from_database(&db).await,
        HarnessReloadOutcome::KeptOnInvalidRow
    );
    assert_eq!(registry.current_guardrails().blocked_topics, ["doping"]);
    let failure = registry.current().load_failure.clone();
    assert!(
        failure
            .as_deref()
            .is_some_and(|reason| reason.contains("summarize_oldest_n")),
        "the mark names the rule, got {failure:?}"
    );
}

#[tokio::test]
async fn a_read_failure_on_a_tick_keeps_the_values() {
    let db = fresh_database().await;
    write_row(&db, &topic_document("doping")).await;
    let registry = HarnessConfigRegistry::from_database(&db).await;

    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlite.pool().close().await,
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => postgres.pool().close().await,
    }

    assert_eq!(
        registry.reload_from_database(&db).await,
        HarnessReloadOutcome::KeptOnReadFailure
    );
    assert_eq!(registry.current_guardrails().blocked_topics, ["doping"]);
}

#[tokio::test]
async fn a_corrupt_row_at_boot_runs_the_defaults_and_heals_on_a_tick() {
    let db = fresh_database().await;
    db.set_system_setting(HARNESS_CONFIG_SETTING_KEY, "{not json")
        .await
        .expect("row written");

    let registry = HarnessConfigRegistry::from_database(&db).await;
    assert_eq!(registry.current().source, HarnessConfigSource::Defaults);
    assert!(registry.current().load_failure.is_some());

    write_row(&db, &topic_document("doping")).await;
    assert_eq!(
        registry.reload_from_database(&db).await,
        HarnessReloadOutcome::Installed
    );
    assert_eq!(registry.current_guardrails().blocked_topics, ["doping"]);
    assert!(registry.current().load_failure.is_none());
}
