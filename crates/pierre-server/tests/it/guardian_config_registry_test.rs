// ABOUTME: Guardian config registry tests — defaults ← document ← env resolution, install hot-swap, DB load, poll convergence
// ABOUTME: Env-free via with_env_overrides; a corrupt row and a read failure are each kept out; a v1 row still loads

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

use std::sync::Arc;
use std::time::Duration;

use common::create_test_database;
use pierre_config::environment::GuardianSettings;
use pierre_contremaitre::harness_config_document::{
    HarnessConfigDocument, HARNESS_CONFIG_SETTING_KEY,
};
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::{settings_poll_interval, start_settings_poll, SettingsPollTargets};
use pierre_tool_runtime::guardian::{
    parse_persisted_document, validate_document, validate_settings, GuardianConfigDocument,
    GuardianConfigRegistry, GuardianConfigSource, GuardianEnvOverrides, GuardianFieldSource,
    GuardianMode, GuardianPolicy, GuardianReloadOutcome, GUARDIAN_CONFIG_SCHEMA_VERSION,
    GUARDIAN_CONFIG_SETTING_KEY,
};
use tokio::time::{sleep, timeout};

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
    assert_eq!(policy.max_destructive_per_turn, 3);
    assert_eq!(policy.max_writes_per_turn, 50);
    assert_eq!(
        policy.max_destructive_per_turn,
        defaults.max_destructive_per_turn
    );
    assert_eq!(policy.max_writes_per_turn, defaults.max_writes_per_turn);

    let sources = snapshot.field_sources;
    assert_eq!(sources.mode, GuardianFieldSource::Default);
    assert_eq!(
        sources.max_destructive_per_turn,
        GuardianFieldSource::Default
    );
    assert_eq!(sources.max_writes_per_turn, GuardianFieldSource::Default);
    assert!(
        sources.env_pinned().is_empty(),
        "expected empty, got {:?}",
        sources.env_pinned()
    );
    assert_eq!(snapshot.source, GuardianConfigSource::Defaults);
}

#[test]
fn document_fields_override_defaults_and_mark_sources() {
    let doc = GuardianConfigDocument {
        mode: Some(GuardianMode::Observe),
        max_writes_per_turn: Some(10),
        ..GuardianConfigDocument::default()
    };
    let registry = env_free(doc, GuardianConfigSource::Database);
    let snapshot = registry.current();
    let policy = snapshot.guardian.policy();

    assert_eq!(policy.mode, GuardianMode::Observe);
    assert_eq!(policy.max_writes_per_turn, 10);
    // Unset fields still follow the compiled-in defaults.
    assert_eq!(policy.max_destructive_per_turn, 3);

    let sources = snapshot.field_sources;
    assert_eq!(sources.mode, GuardianFieldSource::Database);
    assert_eq!(sources.max_writes_per_turn, GuardianFieldSource::Database);
    assert_eq!(
        sources.max_destructive_per_turn,
        GuardianFieldSource::Default
    );
}

#[test]
fn env_overrides_beat_document_and_report_pinned() {
    let env = GuardianEnvOverrides {
        mode: Some(GuardianMode::Enforce),
        max_destructive_per_turn: Some(7),
        ..GuardianEnvOverrides::default()
    };
    let doc = GuardianConfigDocument {
        mode: Some(GuardianMode::Observe),
        max_destructive_per_turn: Some(2),
        max_writes_per_turn: Some(20),
        ..GuardianConfigDocument::default()
    };
    let registry =
        GuardianConfigRegistry::with_env_overrides(env, doc, GuardianConfigSource::Database);
    let snapshot = registry.current();
    let policy = snapshot.guardian.policy();

    // Env wins over the document for the fields it pins; the document still wins
    // for the field env leaves unset.
    assert_eq!(policy.mode, GuardianMode::Enforce);
    assert_eq!(policy.max_destructive_per_turn, 7);
    assert_eq!(policy.max_writes_per_turn, 20);

    let sources = snapshot.field_sources;
    assert_eq!(sources.mode, GuardianFieldSource::Env);
    assert_eq!(sources.max_destructive_per_turn, GuardianFieldSource::Env);
    assert_eq!(sources.max_writes_per_turn, GuardianFieldSource::Database);
    assert_eq!(
        sources.env_pinned(),
        vec!["mode", "max_destructive_per_turn"]
    );
}

#[test]
fn install_swaps_the_effective_policy_for_subsequent_readers() {
    let registry = env_free(
        GuardianConfigDocument::default(),
        GuardianConfigSource::Defaults,
    );
    let before = registry.current_guardian();
    assert_eq!(before.policy().mode, GuardianMode::Enforce);

    registry.install(
        GuardianConfigDocument {
            mode: Some(GuardianMode::Observe),
            max_destructive_per_turn: Some(4),
            ..GuardianConfigDocument::default()
        },
        GuardianConfigSource::AdminUpdate,
    );

    let after = registry.current_guardian();
    assert_eq!(after.policy().mode, GuardianMode::Observe);
    assert_eq!(after.policy().max_destructive_per_turn, 4);
    assert_eq!(registry.current().source, GuardianConfigSource::AdminUpdate);
    // The snapshot handed out before the install keeps the OLD policy — a
    // mid-dispatch reader is never mutated underneath.
    assert_eq!(before.policy().mode, GuardianMode::Enforce);
}

#[test]
fn document_serde_covers_the_wire_forms() {
    // Enum + keyword forms, exactly what the admin PUT / CLI send.
    let json = r#"{
        "schema_version": 2,
        "mode": "observe",
        "max_destructive_per_turn": 4,
        "max_writes_per_turn": 9
    }"#;
    let doc: GuardianConfigDocument = serde_json::from_str(json).expect("wire form parses");
    assert_eq!(doc.mode, Some(GuardianMode::Observe));
    assert_eq!(doc.max_destructive_per_turn, Some(4));
    assert_eq!(doc.max_writes_per_turn, Some(9));

    // Round-trips, and an unset field stays unset rather than freezing a default.
    let serialized = serde_json::to_string(&GuardianConfigDocument {
        mode: Some(GuardianMode::Enforce),
        ..GuardianConfigDocument::default()
    })
    .expect("serializes");
    assert!(
        !serialized.contains("max_destructive_per_turn"),
        "{serialized}"
    );
    let back: GuardianConfigDocument = serde_json::from_str(&serialized).expect("round-trips");
    assert_eq!(back.mode, Some(GuardianMode::Enforce));

    // Unknown keyword is rejected, not silently degraded.
    let bad = r#"{"schema_version": 2, "mode": "everyone"}"#;
    assert!(serde_json::from_str::<GuardianConfigDocument>(bad).is_err());

    // A bare `{}` deserializes to the current schema with no overrides.
    let minimal: GuardianConfigDocument = serde_json::from_str("{}").expect("minimal parses");
    assert_eq!(minimal.schema_version, GUARDIAN_CONFIG_SCHEMA_VERSION);
    // A persisted value: saved rows carry this number, so moving it is a decision.
    assert_eq!(
        GUARDIAN_CONFIG_SCHEMA_VERSION, 2,
        "rows on disk carry this version"
    );
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
        ..GuardianSettings::default()
    };

    let overrides = GuardianEnvOverrides::from_settings(&settings);
    assert_eq!(overrides.mode, Some(GuardianMode::Observe));
    assert_eq!(overrides.max_writes_per_turn, Some(7));
    assert_eq!(
        overrides.max_destructive_per_turn, None,
        "unset stays unset"
    );
    assert_eq!(validate_settings(&settings), []);

    // The removed `off` posture is a typo now, like any other word.
    for not_a_mode in ["enfroce", "off"] {
        settings.mode = Some(not_a_mode.to_owned());
        assert_eq!(
            GuardianEnvOverrides::from_settings(&settings).mode,
            None,
            "{not_a_mode} is no override"
        );
        assert_eq!(
            validate_settings(&settings),
            vec![("GUARDIAN_MODE".to_owned(), not_a_mode.to_owned())]
        );
    }

    // The strict boot gate reports exactly the malformed var, and the lenient
    // capture drops it.
    settings.mode = None;
    settings.max_destructive_per_turn = Some("lots".to_owned());
    assert_eq!(
        validate_settings(&settings),
        vec![(
            "GUARDIAN_MAX_DESTRUCTIVE_PER_TURN".to_owned(),
            "lots".to_owned()
        )]
    );
    assert_eq!(
        GuardianEnvOverrides::from_settings(&settings).max_destructive_per_turn,
        None
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
        mode: Some(GuardianMode::Observe),
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
    assert_eq!(snapshot.guardian.policy().mode, GuardianMode::Observe);
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
        overridden.guardian.policy().mode,
        GuardianMode::Observe,
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

fn observe_document() -> GuardianConfigDocument {
    GuardianConfigDocument {
        mode: Some(GuardianMode::Observe),
        max_writes_per_turn: Some(9),
        ..GuardianConfigDocument::default()
    }
}

async fn write_row(db: &Database, document: &GuardianConfigDocument) {
    db.set_system_setting(
        GUARDIAN_CONFIG_SETTING_KEY,
        &serde_json::to_string(document).expect("document serializes"),
    )
    .await
    .expect("row written");
}

/// Close the pool, so every later read on this database fails.
async fn break_reads(db: &Database) {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlite.pool().close().await,
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => postgres.pool().close().await,
    }
}

/// A save lands on one instance. The other hears about it through its poll tick,
/// with no restart and no second save.
#[tokio::test]
async fn two_instances_over_one_database_converge_through_the_poll_tick() {
    let db = create_test_database().await.expect("test database");
    let serving = GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    let other = GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    assert_ne!(serving.instance_nonce(), other.instance_nonce());

    // What the PUT handler does on the instance the request landed on.
    write_row(&db, &observe_document()).await;
    serving.install(observe_document(), GuardianConfigSource::AdminUpdate);

    assert_eq!(
        other.current_guardian().policy().mode,
        GuardianMode::Enforce,
        "before its tick the other instance still runs the old policy"
    );
    assert_eq!(
        other.reload_from_database(&db).await,
        GuardianReloadOutcome::Installed
    );
    let snapshot = other.current();
    assert_eq!(snapshot.guardian.policy().mode, GuardianMode::Observe);
    assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 9);
    assert_eq!(snapshot.source, GuardianConfigSource::Database);

    // Nothing changed since: neither instance swaps again, so a tick on an idle
    // fleet costs one read.
    assert_eq!(
        other.reload_from_database(&db).await,
        GuardianReloadOutcome::Unchanged
    );
    assert_eq!(
        serving.reload_from_database(&db).await,
        GuardianReloadOutcome::Unchanged,
        "the instance that took the save already holds the row"
    );
}

/// The spawned loop, not only its tick: the wiring that makes the convergence
/// above happen on its own.
#[tokio::test]
async fn the_started_poll_converges_both_registries_on_its_own() {
    let db = create_test_database().await.expect("test database");
    let guardian =
        Arc::new(GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await);
    let harness = Arc::new(HarnessConfigRegistry::from_database(&db).await);
    start_settings_poll(
        SettingsPollTargets {
            database: Arc::clone(&db),
            guardian: Arc::clone(&guardian),
            harness: Arc::clone(&harness),
        },
        Duration::from_millis(20),
    );

    write_row(&db, &observe_document()).await;
    let mut topics = HarnessConfigDocument::default();
    topics.guardrails.blocked_topics = vec!["doping".to_owned()];
    db.set_system_setting(
        HARNESS_CONFIG_SETTING_KEY,
        &serde_json::to_string(&topics).expect("document serializes"),
    )
    .await
    .expect("row written");

    timeout(Duration::from_secs(10), async {
        while guardian.current_guardian().policy().mode != GuardianMode::Observe
            || harness.current_guardrails().blocked_topics.is_empty()
        {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the poll installs both rows without being asked");

    assert_eq!(
        settings_poll_interval(None),
        Duration::from_secs(30),
        "the default interval"
    );
    assert_eq!(settings_poll_interval(Some(5)), Duration::from_secs(5));
}

/// A row that goes bad under a running instance must not take the policy with it.
#[tokio::test]
async fn a_corrupt_row_keeps_the_policy_in_force_until_it_is_repaired() {
    let db = create_test_database().await.expect("test database");
    write_row(&db, &observe_document()).await;
    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    assert!(registry.current().degraded_reason().is_none());

    // `[]` is what serde would otherwise read as a struct of defaults.
    for corrupt in ["{not json", "[]", r#"{"mode": "enfroce"}"#] {
        db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, corrupt)
            .await
            .expect("row written");
        assert_eq!(
            registry.reload_from_database(&db).await,
            GuardianReloadOutcome::KeptOnInvalidRow,
            "{corrupt} is not a document"
        );
        let snapshot = registry.current();
        assert_eq!(
            snapshot.guardian.policy().mode,
            GuardianMode::Observe,
            "{corrupt} must not move the policy"
        );
        assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 9);
        assert!(snapshot
            .degraded_reason()
            .expect("the snapshot says so")
            .contains("does not parse"));
    }

    // Repairing the row to the very document in force still clears the mark.
    write_row(&db, &observe_document()).await;
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Installed
    );
    assert!(registry.current().degraded_reason().is_none());
}

/// A pod that came up over a bad row runs the defaults, says so, and heals.
#[tokio::test]
async fn a_corrupt_row_at_boot_runs_the_defaults_marked_degraded() {
    let db = create_test_database().await.expect("test database");
    db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, "{not json")
        .await
        .expect("row written");

    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    let snapshot = registry.current();
    assert_eq!(snapshot.guardian.policy().mode, GuardianMode::Enforce);
    assert!(snapshot
        .degraded_reason()
        .expect("marked degraded")
        .contains("does not parse"));

    write_row(&db, &observe_document()).await;
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Installed
    );
    assert_eq!(
        registry.current_guardian().policy().mode,
        GuardianMode::Observe
    );
    assert!(registry.current().degraded_reason().is_none());
}

/// A failed read says nothing about the row, so it changes nothing, and in
/// particular does not put a loaded instance back on the defaults.
#[tokio::test]
async fn a_read_failure_on_a_tick_keeps_the_policy() {
    let db = create_test_database().await.expect("test database");
    write_row(&db, &observe_document()).await;
    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;

    break_reads(&db).await;

    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::KeptOnReadFailure
    );
    assert_eq!(
        registry.current_guardian().policy().mode,
        GuardianMode::Observe
    );
}

#[tokio::test]
async fn a_read_failure_at_boot_runs_the_defaults_marked_degraded() {
    let db = create_test_database().await.expect("test database");
    write_row(&db, &observe_document()).await;
    break_reads(&db).await;

    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    let snapshot = registry.current();
    assert_eq!(snapshot.guardian.policy().mode, GuardianMode::Enforce);
    assert!(snapshot
        .degraded_reason()
        .expect("marked degraded")
        .contains("could not be read"));
}

/// An absent row is a real answer, unlike a failed read: it means the defaults,
/// as it does at boot.
#[tokio::test]
async fn a_row_that_is_gone_resolves_to_the_defaults_on_a_tick() {
    let db = create_test_database().await.expect("test database");
    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Unchanged,
        "no row and already on the defaults"
    );

    registry.install(observe_document(), GuardianConfigSource::AdminUpdate);
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Installed
    );
    let snapshot = registry.current();
    assert_eq!(snapshot.guardian.policy().mode, GuardianMode::Enforce);
    assert_eq!(snapshot.source, GuardianConfigSource::Defaults);
}

#[test]
fn a_stored_zero_write_budget_resolves_to_the_default_and_says_so() {
    let registry = env_free(
        GuardianConfigDocument {
            max_writes_per_turn: Some(0),
            // A zero destructive budget is a posture a super admin can mean.
            max_destructive_per_turn: Some(0),
            ..GuardianConfigDocument::default()
        },
        GuardianConfigSource::Database,
    );
    let snapshot = registry.current();
    let policy = snapshot.guardian.policy();
    assert_eq!(
        policy.max_writes_per_turn,
        GuardianPolicy::default().max_writes_per_turn
    );
    assert_eq!(
        snapshot.field_sources.max_writes_per_turn,
        GuardianFieldSource::Default
    );
    assert_eq!(policy.max_destructive_per_turn, 0);
    assert!(snapshot
        .degraded_reason()
        .expect("marked degraded")
        .contains("max_writes_per_turn=0"));
}

#[test]
fn the_boot_gate_refuses_a_zero_write_budget_and_allows_a_zero_destructive_one() {
    let zero_writes = GuardianSettings {
        max_writes_per_turn: Some("0".to_owned()),
        ..GuardianSettings::default()
    };
    assert_eq!(
        validate_settings(&zero_writes),
        vec![("GUARDIAN_MAX_WRITES_PER_TURN".to_owned(), "0".to_owned())]
    );
    assert_eq!(
        GuardianEnvOverrides::from_settings(&zero_writes).max_writes_per_turn,
        None,
        "outside the server's gate a zero budget is no override"
    );

    let zero_destructive = GuardianSettings {
        max_destructive_per_turn: Some("0".to_owned()),
        ..GuardianSettings::default()
    };
    assert_eq!(validate_settings(&zero_destructive), []);
    assert_eq!(
        GuardianEnvOverrides::from_settings(&zero_destructive).max_destructive_per_turn,
        Some(0)
    );
}

#[test]
fn a_foreign_row_parses_to_the_current_version_and_reports_what_it_held() {
    let persisted = parse_persisted_document(
        r#"{"schema_version": 7, "mode": "observe", "shiny_new_rule": true}"#,
    )
    .expect("the fields this binary reads are valid");
    assert_eq!(
        persisted.document.schema_version,
        GUARDIAN_CONFIG_SCHEMA_VERSION
    );
    assert_eq!(persisted.document.mode, Some(GuardianMode::Observe));
    assert_eq!(persisted.stored_schema_version, Some(7));
    assert_eq!(persisted.unknown_fields, ["shiny_new_rule"]);

    let current = parse_persisted_document(r#"{"schema_version": 2, "mode": "observe"}"#).unwrap();
    assert_eq!(current.stored_schema_version, None);
    assert_eq!(current.unknown_fields, Vec::<String>::new());
}

/// A row written before schema 2 is what every deployed instance holds, so it
/// loads rather than degrading the policy: the fields this binary still reads
/// apply, and what it no longer reads is reported, never silently dropped.
#[test]
fn a_v1_row_parses_with_its_known_fields_and_reports_the_removed_ones() {
    let bare = parse_persisted_document(r#"{"schema_version":1}"#).expect("a v1 row parses");
    assert_eq!(bare.stored_schema_version, Some(1));
    assert_eq!(bare.unknown_fields, Vec::<String>::new());
    assert_eq!(bare.document, GuardianConfigDocument::default());

    let persisted = parse_persisted_document(
        r#"{"schema_version":1,"plan_mode":"enforce","tainted_destructive":"deny","external_send":"all","max_writes_per_turn":7,"mode":"observe"}"#,
    )
    .expect("the removed fields are reported, not refused");
    assert_eq!(persisted.stored_schema_version, Some(1));
    assert_eq!(
        persisted.document.schema_version,
        GUARDIAN_CONFIG_SCHEMA_VERSION
    );
    assert_eq!(persisted.document.mode, Some(GuardianMode::Observe));
    assert_eq!(persisted.document.max_writes_per_turn, Some(7));
    let mut unknown = persisted.unknown_fields;
    unknown.sort();
    assert_eq!(
        unknown,
        ["external_send", "plan_mode", "tainted_destructive"]
    );
}

/// The same rows through the registry, at boot and on a poll tick: each is a
/// healthy load, never a degraded one.
#[tokio::test]
async fn a_v1_row_loads_at_boot_and_on_a_tick_without_degrading() {
    let db = create_test_database().await.expect("test database");

    db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, r#"{"schema_version":1}"#)
        .await
        .expect("row written");
    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;
    let snapshot = registry.current();
    assert_eq!(snapshot.source, GuardianConfigSource::Database);
    assert_eq!(snapshot.guardian.policy().mode, GuardianMode::Enforce);
    assert_eq!(snapshot.guardian.policy().max_destructive_per_turn, 3);
    assert!(snapshot.degraded_reason().is_none());
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Unchanged,
        "a v1 row reads as what the snapshot already holds, not as a change each tick"
    );

    db.set_system_setting(
        GUARDIAN_CONFIG_SETTING_KEY,
        r#"{"schema_version":1,"plan_mode":"enforce","tainted_destructive":"deny","external_send":"all","mode":"observe","max_destructive_per_turn":2}"#,
    )
    .await
    .expect("row written");
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Installed
    );
    let snapshot = registry.current();
    assert_eq!(snapshot.guardian.policy().mode, GuardianMode::Observe);
    assert_eq!(snapshot.guardian.policy().max_destructive_per_turn, 2);
    assert_eq!(
        snapshot.field_sources.max_destructive_per_turn,
        GuardianFieldSource::Database
    );
    assert!(snapshot.degraded_reason().is_none());

    let booted = GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default())
        .await
        .current();
    assert_eq!(booted.source, GuardianConfigSource::Database);
    assert_eq!(booted.guardian.policy().mode, GuardianMode::Observe);
    assert_eq!(booted.guardian.policy().max_destructive_per_turn, 2);
    assert!(booted.degraded_reason().is_none());
}

/// `off` was removed rather than aliased, so a row still holding it is unreadable.
/// The policy in force stays and the admin surface says why, until the row is fixed.
#[tokio::test]
async fn a_row_with_mode_off_keeps_the_policy_in_force_and_marks_it_degraded() {
    for row in [r#"{"mode":"off"}"#, r#"{"schema_version":1,"mode":"off"}"#] {
        assert!(parse_persisted_document(row).is_err(), "{row}");
    }

    let db = create_test_database().await.expect("test database");
    write_row(&db, &observe_document()).await;
    let registry =
        GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default()).await;

    db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, r#"{"mode":"off"}"#)
        .await
        .expect("row written");
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::KeptOnInvalidRow
    );
    let snapshot = registry.current();
    assert_eq!(snapshot.guardian.policy().mode, GuardianMode::Observe);
    assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 9);
    assert!(snapshot
        .degraded_reason()
        .expect("the snapshot says so")
        .contains("does not parse"));

    // A pod that boots over the same row runs the defaults, not `off`.
    let booted = GuardianConfigRegistry::from_database(&db, GuardianEnvOverrides::default())
        .await
        .current();
    assert_eq!(booted.guardian.policy().mode, GuardianMode::Enforce);
    assert!(booted
        .degraded_reason()
        .expect("marked degraded")
        .contains("does not parse"));
}
