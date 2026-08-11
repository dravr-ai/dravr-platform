// ABOUTME: Guardian config registry tests — defaults ← document ← env resolution, install hot-swap, DB load
// ABOUTME: Env-free via with_env_overrides except one combined fn that owns all GUARDIAN_* env mutation

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Tests for [`GuardianConfigRegistry`] resolution and hot-reload.
//!
//! Every test but one builds registries through `with_env_overrides` with an
//! explicit (usually empty) [`GuardianEnvOverrides`], so they never read the
//! process env and stay parallel-safe. The single env-touching fn owns ALL
//! `GUARDIAN_*` mutation in this binary — including the `from_database` loads,
//! which capture env at construction — so no other test can observe a
//! half-set environment.

mod common;

use std::env;
use std::iter;

use common::create_test_database;
use pierre_tool_runtime::guardian::{
    validate_document, validate_env, ExternalSendAllowlist, GuardianConfigDocument,
    GuardianConfigRegistry, GuardianConfigSource, GuardianEnvOverrides, GuardianFieldSource,
    GuardianMode, GuardianPolicy, PlanMode, TaintedDestructive, GUARDIAN_CONFIG_SCHEMA_VERSION,
    GUARDIAN_CONFIG_SETTING_KEY,
};
use uuid::Uuid;

fn env_free(doc: GuardianConfigDocument, source: GuardianConfigSource) -> GuardianConfigRegistry {
    GuardianConfigRegistry::with_env_overrides(GuardianEnvOverrides::default(), doc, source)
}

#[test]
fn empty_document_resolves_to_compiled_defaults() {
    let registry = env_free(
        GuardianConfigDocument::default(),
        GuardianConfigSource::Defaults,
    );
    let snapshot = registry.current();
    let policy = snapshot.guardian.policy();
    let defaults = GuardianPolicy::default();

    assert_eq!(policy.mode, GuardianMode::Enforce);
    assert_eq!(
        policy.max_destructive_per_turn,
        defaults.max_destructive_per_turn
    );
    assert_eq!(policy.max_writes_per_turn, defaults.max_writes_per_turn);
    assert!(!policy.external_send.allows(Some(Uuid::new_v4())));
    assert_eq!(policy.tainted_destructive, TaintedDestructive::Log);
    assert_eq!(policy.plan_mode, PlanMode::Off);

    let sources = snapshot.field_sources;
    assert_eq!(sources.mode, GuardianFieldSource::Default);
    assert_eq!(sources.tainted_destructive, GuardianFieldSource::Default);
    assert!(sources.env_pinned().is_empty());
    assert_eq!(snapshot.source, GuardianConfigSource::Defaults);
}

#[test]
fn document_fields_override_defaults_and_mark_sources() {
    let doc = GuardianConfigDocument {
        mode: Some(GuardianMode::Observe),
        max_writes_per_turn: Some(10),
        tainted_destructive: Some(TaintedDestructive::Deny),
        ..GuardianConfigDocument::default()
    };
    let registry = env_free(doc, GuardianConfigSource::Database);
    let snapshot = registry.current();
    let policy = snapshot.guardian.policy();

    assert_eq!(policy.mode, GuardianMode::Observe);
    assert_eq!(policy.max_writes_per_turn, 10);
    assert_eq!(policy.tainted_destructive, TaintedDestructive::Deny);
    // Unset fields still follow the compiled-in defaults.
    assert_eq!(policy.max_destructive_per_turn, 1);
    assert_eq!(policy.plan_mode, PlanMode::Off);

    let sources = snapshot.field_sources;
    assert_eq!(sources.mode, GuardianFieldSource::Database);
    assert_eq!(sources.max_writes_per_turn, GuardianFieldSource::Database);
    assert_eq!(sources.tainted_destructive, GuardianFieldSource::Database);
    assert_eq!(
        sources.max_destructive_per_turn,
        GuardianFieldSource::Default
    );
}

#[test]
fn env_overrides_beat_document_and_report_pinned() {
    let env = GuardianEnvOverrides {
        mode: Some(GuardianMode::Enforce),
        plan_mode: Some(PlanMode::Enforce),
        ..GuardianEnvOverrides::default()
    };
    let doc = GuardianConfigDocument {
        mode: Some(GuardianMode::Off),
        max_writes_per_turn: Some(20),
        ..GuardianConfigDocument::default()
    };
    let registry =
        GuardianConfigRegistry::with_env_overrides(env, doc, GuardianConfigSource::Database);
    let snapshot = registry.current();
    let policy = snapshot.guardian.policy();

    // Env wins over the document for mode; the document still wins for the
    // field env leaves unset.
    assert_eq!(policy.mode, GuardianMode::Enforce);
    assert_eq!(policy.plan_mode, PlanMode::Enforce);
    assert_eq!(policy.max_writes_per_turn, 20);

    let sources = snapshot.field_sources;
    assert_eq!(sources.mode, GuardianFieldSource::Env);
    assert_eq!(sources.plan_mode, GuardianFieldSource::Env);
    assert_eq!(sources.max_writes_per_turn, GuardianFieldSource::Database);
    assert_eq!(sources.env_pinned(), vec!["mode", "plan_mode"]);
}

#[test]
fn install_swaps_the_effective_policy_for_subsequent_readers() {
    let registry = env_free(
        GuardianConfigDocument::default(),
        GuardianConfigSource::Defaults,
    );
    let before = registry.current_guardian();
    assert_eq!(before.policy().tainted_destructive, TaintedDestructive::Log);

    registry.install(
        GuardianConfigDocument {
            tainted_destructive: Some(TaintedDestructive::Deny),
            max_destructive_per_turn: Some(3),
            ..GuardianConfigDocument::default()
        },
        GuardianConfigSource::AdminUpdate,
    );

    let after = registry.current_guardian();
    assert_eq!(after.policy().tainted_destructive, TaintedDestructive::Deny);
    assert_eq!(after.policy().max_destructive_per_turn, 3);
    assert_eq!(registry.current().source, GuardianConfigSource::AdminUpdate);
    // The snapshot handed out before the install keeps the OLD policy — a
    // mid-dispatch reader is never mutated underneath.
    assert_eq!(before.policy().tainted_destructive, TaintedDestructive::Log);
}

#[test]
fn document_serde_covers_the_wire_forms() {
    // Enum + keyword forms, exactly what the admin PUT / CLI send.
    let json = r#"{
        "schema_version": 1,
        "mode": "observe",
        "tainted_destructive": "confirm",
        "plan_mode": "enforce",
        "external_send": "all"
    }"#;
    let doc: GuardianConfigDocument = serde_json::from_str(json).expect("wire form parses");
    assert_eq!(doc.mode, Some(GuardianMode::Observe));
    assert_eq!(doc.tainted_destructive, Some(TaintedDestructive::Confirm));
    assert_eq!(doc.plan_mode, Some(PlanMode::Enforce));
    assert!(matches!(
        doc.external_send,
        Some(ExternalSendAllowlist::All)
    ));

    // Tenant-list form round-trips (sorted, deterministic).
    let tenant = Uuid::new_v4();
    let doc = GuardianConfigDocument {
        external_send: Some(ExternalSendAllowlist::Only(iter::once(tenant).collect())),
        ..GuardianConfigDocument::default()
    };
    let serialized = serde_json::to_string(&doc).expect("serializes");
    assert!(serialized.contains(&tenant.to_string()));
    let back: GuardianConfigDocument = serde_json::from_str(&serialized).expect("round-trips");
    match back.external_send {
        Some(ExternalSendAllowlist::Only(set)) => assert!(set.contains(&tenant)),
        other => panic!("expected Only allowlist, got {other:?}"),
    }

    // Unknown keyword is rejected, not silently degraded.
    let bad = r#"{"schema_version": 1, "external_send": "everyone"}"#;
    assert!(serde_json::from_str::<GuardianConfigDocument>(bad).is_err());

    // Empty tenant array normalizes to None (deny-all).
    let empty = r#"{"schema_version": 1, "external_send": []}"#;
    let doc: GuardianConfigDocument = serde_json::from_str(empty).expect("empty array parses");
    assert!(matches!(
        doc.external_send,
        Some(ExternalSendAllowlist::None)
    ));

    // A bare `{}` deserializes to the current schema with no overrides.
    let minimal: GuardianConfigDocument = serde_json::from_str("{}").expect("minimal parses");
    assert_eq!(minimal.schema_version, GUARDIAN_CONFIG_SCHEMA_VERSION);
    assert!(minimal.mode.is_none());
}

#[test]
fn validate_document_rejects_bad_schema_and_zero_write_budget() {
    let wrong_schema = GuardianConfigDocument {
        schema_version: 99,
        ..GuardianConfigDocument::default()
    };
    let err = validate_document(&wrong_schema).expect_err("schema 99 must be rejected");
    assert!(err.to_string().contains("schema_version"));

    let zero_writes = GuardianConfigDocument {
        max_writes_per_turn: Some(0),
        ..GuardianConfigDocument::default()
    };
    let err = validate_document(&zero_writes).expect_err("0 write budget must be rejected");
    assert!(err.to_string().contains("max_writes_per_turn"));

    assert!(validate_document(&GuardianConfigDocument::default()).is_ok());
}

/// The one fn that touches process env AND constructs env-reading registries
/// (`from_database` captures `GuardianEnvOverrides::from_env`): keeping both
/// halves sequential in a single test means no parallel test in this binary
/// can observe the mutated environment.
#[tokio::test]
async fn env_capture_validation_and_database_load() {
    // --- Env half -----------------------------------------------------------
    env::set_var("GUARDIAN_MODE", "observe");
    env::set_var("GUARDIAN_MAX_WRITES_PER_TURN", "7");
    env::set_var(
        "GUARDIAN_EXTERNAL_SEND_TENANTS",
        "not-a-uuid,also-not-a-uuid",
    );

    let overrides = GuardianEnvOverrides::from_env();
    assert_eq!(overrides.mode, Some(GuardianMode::Observe));
    assert_eq!(overrides.max_writes_per_turn, Some(7));
    // The lenient capture fail-closes the malformed allowlist to deny-all…
    assert!(matches!(
        overrides.external_send,
        Some(ExternalSendAllowlist::None)
    ));
    // …while the strict boot gate reports exactly the malformed var.
    let bad = validate_env();
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].0, "GUARDIAN_EXTERNAL_SEND_TENANTS");

    env::set_var("GUARDIAN_EXTERNAL_SEND_TENANTS", "all");
    assert!(validate_env().is_empty());

    env::remove_var("GUARDIAN_MODE");
    env::remove_var("GUARDIAN_MAX_WRITES_PER_TURN");
    env::remove_var("GUARDIAN_EXTERNAL_SEND_TENANTS");

    // --- Database half (env now clean) --------------------------------------
    let db = create_test_database().await.expect("test database");

    // No row → compiled defaults.
    let registry = GuardianConfigRegistry::from_database(&db).await;
    assert_eq!(registry.current().source, GuardianConfigSource::Defaults);
    assert_eq!(
        registry.current_guardian().policy().mode,
        GuardianMode::Enforce
    );

    // Persisted row → loaded, effective, and marked Database-sourced.
    let doc = GuardianConfigDocument {
        max_writes_per_turn: Some(12),
        plan_mode: Some(PlanMode::Enforce),
        ..GuardianConfigDocument::default()
    };
    let json = serde_json::to_string(&doc).expect("doc serializes");
    db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, &json)
        .await
        .expect("row written");

    let registry = GuardianConfigRegistry::from_database(&db).await;
    let snapshot = registry.current();
    assert_eq!(snapshot.source, GuardianConfigSource::Database);
    assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 12);
    assert_eq!(snapshot.guardian.policy().plan_mode, PlanMode::Enforce);
    assert_eq!(
        snapshot.field_sources.max_writes_per_turn,
        GuardianFieldSource::Database
    );

    // Corrupted row → server still boots on compiled defaults.
    db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, "{not json")
        .await
        .expect("row written");
    let registry = GuardianConfigRegistry::from_database(&db).await;
    assert_eq!(registry.current().source, GuardianConfigSource::Defaults);
    assert_eq!(
        registry.current_guardian().policy().max_writes_per_turn,
        GuardianPolicy::default().max_writes_per_turn
    );
}
