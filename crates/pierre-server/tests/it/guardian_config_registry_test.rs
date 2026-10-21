// ABOUTME: Guardian config registry tests — defaults ← document ← env resolution, install hot-swap, DB load
// ABOUTME: Env-free via with_env_overrides except one combined fn that owns all GUARDIAN_* env mutation

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Tests for [`GuardianConfigRegistry`] resolution and hot-reload.
//!
//! Every test builds registries over explicit [`GuardianEnvOverrides`] — the
//! `GUARDIAN_*` values the server's config captured at startup, parsed — so
//! none reads the process environment.

use crate::common;

use std::iter;

use common::create_test_database;
use pierre_config::environment::GuardianSettings;
use pierre_tool_runtime::guardian::{
    validate_document, validate_settings, ExternalSendAllowlist, GuardianConfigDocument,
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
    assert_eq!(sources.env_pinned(), [] as [&str; 0]);
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

/// The `GUARDIAN_*` values startup captured parse leniently into the
/// registry's overrides, while the strict boot gate names exactly the value it
/// does not recognize.
#[test]
fn captured_settings_parse_into_overrides_and_validation() {
    let mut settings = GuardianSettings {
        mode: Some("observe".to_owned()),
        max_writes_per_turn: Some(" 7 ".to_owned()),
        external_send_tenants: Some("not-a-uuid,also-not-a-uuid".to_owned()),
        ..GuardianSettings::default()
    };

    let overrides = GuardianEnvOverrides::from_settings(&settings);
    assert_eq!(overrides.mode, Some(GuardianMode::Observe));
    assert_eq!(overrides.max_writes_per_turn, Some(7));
    assert_eq!(
        overrides.max_destructive_per_turn, None,
        "unset stays unset"
    );
    // The lenient capture fail-closes the malformed allowlist to deny-all…
    assert!(matches!(
        overrides.external_send,
        Some(ExternalSendAllowlist::None)
    ));
    // …while the strict boot gate reports exactly the malformed var.
    let bad = validate_settings(&settings);
    assert_eq!(
        bad,
        vec![(
            "GUARDIAN_EXTERNAL_SEND_TENANTS".to_owned(),
            "not-a-uuid,also-not-a-uuid".to_owned()
        )]
    );

    settings.external_send_tenants = Some("all".to_owned());
    assert_eq!(validate_settings(&settings), []);

    settings.mode = Some("enfroce".to_owned());
    assert_eq!(
        GuardianEnvOverrides::from_settings(&settings).mode,
        None,
        "a typo is no override"
    );
    assert_eq!(
        validate_settings(&settings),
        vec![("GUARDIAN_MODE".to_owned(), "enfroce".to_owned())]
    );
}

/// The registry loads its document from the database at startup, under the
/// overrides it is handed.
#[tokio::test]
async fn the_database_load_runs_under_the_overrides_it_is_handed() {
    let db = create_test_database().await.expect("test database");

    // No row → compiled defaults.
    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
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

    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    let snapshot = registry.current();
    assert_eq!(snapshot.source, GuardianConfigSource::Database);
    assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 12);
    assert_eq!(snapshot.guardian.policy().plan_mode, PlanMode::Enforce);
    assert_eq!(
        snapshot.field_sources.max_writes_per_turn,
        GuardianFieldSource::Database
    );

    // An operator override wins over the stored row, field by field.
    let overridden = GuardianConfigRegistry::from_database(
        &db,
        GuardianEnvOverrides::from_settings(&GuardianSettings {
            max_writes_per_turn: Some("3".to_owned()),
            ..GuardianSettings::default()
        }),
    )
    .await
    .current();
    assert_eq!(overridden.guardian.policy().max_writes_per_turn, 3);
    assert_eq!(
        overridden.field_sources.max_writes_per_turn,
        GuardianFieldSource::Env
    );
    assert_eq!(
        overridden.guardian.policy().plan_mode,
        PlanMode::Enforce,
        "a field the overrides leave unset keeps the stored value"
    );

    // Corrupted row → server still boots on compiled defaults.
    db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, "{not json")
        .await
        .expect("row written");
    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    assert_eq!(registry.current().source, GuardianConfigSource::Defaults);
    assert_eq!(
        registry.current_guardian().policy().max_writes_per_turn,
        GuardianPolicy::default().max_writes_per_turn
    );
}
