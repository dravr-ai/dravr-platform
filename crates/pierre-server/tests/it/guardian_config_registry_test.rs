// ABOUTME: Guardian config registry tests — defaults ← stored document resolution, install hot-swap, DB load, poll convergence
// ABOUTME: A corrupt row and a read failure are each kept out; a row in an earlier binary's shape still loads

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Tests for [`GuardianConfigRegistry`] resolution and hot-reload.

use crate::common;

use std::sync::Arc;
use std::time::Duration;

use common::create_test_database;
use pierre_contremaitre::harness_config_document::{
    HarnessConfigDocument, HARNESS_CONFIG_SETTING_KEY,
};
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_mcp_server::{settings_poll_interval, start_settings_poll, SettingsPollTargets};
use pierre_tool_runtime::guardian::{
    parse_persisted_document, validate_document, GuardianConfigDocument, GuardianConfigRegistry,
    GuardianPolicy, GuardianReloadOutcome, GUARDIAN_CONFIG_SETTING_KEY,
};
use tokio::time::{sleep, timeout};

/// The caps a stored row sets in these tests: both away from the defaults, so a
/// test can tell the row in force from the defaults.
fn stored_document() -> GuardianConfigDocument {
    GuardianConfigDocument {
        max_destructive_per_turn: Some(1),
        max_writes_per_turn: Some(9),
    }
}

fn destructive_cap(registry: &GuardianConfigRegistry) -> u16 {
    registry
        .current_guardian()
        .policy()
        .max_destructive_per_turn
}

#[test]
fn empty_document_resolves_to_compiled_defaults() {
    let registry = GuardianConfigRegistry::bootstrap();
    let snapshot = registry.current();
    let policy = snapshot.guardian.policy();

    assert_eq!(policy.max_destructive_per_turn, 3);
    assert_eq!(policy.max_writes_per_turn, 50);
    let defaults = GuardianPolicy::default();
    assert_eq!(
        policy.max_destructive_per_turn,
        defaults.max_destructive_per_turn
    );
    assert_eq!(policy.max_writes_per_turn, defaults.max_writes_per_turn);
    assert!(snapshot.degraded_reason().is_none());
}

#[test]
fn document_fields_override_defaults_field_by_field() {
    let registry = GuardianConfigRegistry::with_document(GuardianConfigDocument {
        max_writes_per_turn: Some(10),
        ..GuardianConfigDocument::default()
    });
    let policy = registry.current_guardian();

    assert_eq!(policy.policy().max_writes_per_turn, 10);
    // An unset field still follows the compiled-in default.
    assert_eq!(policy.policy().max_destructive_per_turn, 3);
}

#[test]
fn install_swaps_the_effective_policy_for_subsequent_readers() {
    let registry = GuardianConfigRegistry::bootstrap();
    let before = registry.current_guardian();
    assert_eq!(before.policy().max_destructive_per_turn, 3);

    registry.install(GuardianConfigDocument {
        max_destructive_per_turn: Some(4),
        ..GuardianConfigDocument::default()
    });

    assert_eq!(destructive_cap(&registry), 4);
    // The snapshot handed out before the install keeps the OLD policy — a
    // mid-dispatch reader is never mutated underneath.
    assert_eq!(before.policy().max_destructive_per_turn, 3);
}

#[test]
fn document_serde_covers_the_wire_forms() {
    let json = r#"{"max_destructive_per_turn": 4, "max_writes_per_turn": 9}"#;
    let doc: GuardianConfigDocument = serde_json::from_str(json).expect("wire form parses");
    assert_eq!(doc.max_destructive_per_turn, Some(4));
    assert_eq!(doc.max_writes_per_turn, Some(9));

    // An unset field stays unset rather than freezing a default.
    let serialized = serde_json::to_string(&GuardianConfigDocument {
        max_writes_per_turn: Some(9),
        ..GuardianConfigDocument::default()
    })
    .expect("serializes");
    assert_eq!(serialized, r#"{"max_writes_per_turn":9}"#);

    // A value of the wrong type is refused, not silently degraded.
    let bad = r#"{"max_writes_per_turn": "lots"}"#;
    assert!(serde_json::from_str::<GuardianConfigDocument>(bad).is_err());

    let minimal: GuardianConfigDocument = serde_json::from_str("{}").expect("minimal parses");
    assert_eq!(minimal, GuardianConfigDocument::default());
}

#[test]
fn validate_document_rejects_a_zero_write_budget() {
    let zero_writes = GuardianConfigDocument {
        max_writes_per_turn: Some(0),
        ..GuardianConfigDocument::default()
    };
    let err = validate_document(&zero_writes).expect_err("0 write budget must be rejected");
    assert!(err.to_string().contains("max_writes_per_turn"));

    // A zero destructive budget is a posture a super admin can mean.
    let zero_destructive = GuardianConfigDocument {
        max_destructive_per_turn: Some(0),
        ..GuardianConfigDocument::default()
    };
    assert!(validate_document(&zero_destructive).is_ok());
    assert!(validate_document(&GuardianConfigDocument::default()).is_ok());
}

#[tokio::test]
async fn the_database_load_applies_the_stored_row() {
    let db = create_test_database().await.expect("test database");

    // No row → compiled defaults.
    let registry = GuardianConfigRegistry::from_database(&db).await;
    assert_eq!(destructive_cap(&registry), 3);
    assert!(registry.current().degraded_reason().is_none());

    // Persisted row → loaded and effective.
    write_row(&db, &stored_document()).await;
    let registry = GuardianConfigRegistry::from_database(&db).await;
    let snapshot = registry.current();
    assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 9);
    assert_eq!(snapshot.guardian.policy().max_destructive_per_turn, 1);

    // Corrupted row → server still boots on compiled defaults.
    db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, "{not json")
        .await
        .expect("row written");
    let registry = GuardianConfigRegistry::from_database(&db).await;
    assert_eq!(
        registry.current_guardian().policy().max_writes_per_turn,
        GuardianPolicy::default().max_writes_per_turn
    );
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
    let serving = GuardianConfigRegistry::from_database(&db).await;
    let other = GuardianConfigRegistry::from_database(&db).await;
    assert_ne!(serving.instance_nonce(), other.instance_nonce());

    // What the PUT handler does on the instance the request landed on.
    write_row(&db, &stored_document()).await;
    serving.install(stored_document());

    assert_eq!(
        destructive_cap(&other),
        3,
        "before its tick the other instance still runs the old policy"
    );
    assert_eq!(
        other.reload_from_database(&db).await,
        GuardianReloadOutcome::Installed
    );
    let snapshot = other.current();
    assert_eq!(snapshot.guardian.policy().max_destructive_per_turn, 1);
    assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 9);

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
    let guardian = Arc::new(GuardianConfigRegistry::from_database(&db).await);
    let harness = Arc::new(HarnessConfigRegistry::from_database(&db).await);
    start_settings_poll(
        SettingsPollTargets {
            database: Arc::clone(&db),
            guardian: Arc::clone(&guardian),
            harness: Arc::clone(&harness),
        },
        Duration::from_millis(20),
    );

    write_row(&db, &stored_document()).await;
    let mut topics = HarnessConfigDocument::default();
    topics.guardrails.blocked_topics = vec!["doping".to_owned()];
    db.set_system_setting(
        HARNESS_CONFIG_SETTING_KEY,
        &serde_json::to_string(&topics).expect("document serializes"),
    )
    .await
    .expect("row written");

    timeout(Duration::from_secs(10), async {
        while destructive_cap(&guardian) != 1
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
    write_row(&db, &stored_document()).await;
    let registry = GuardianConfigRegistry::from_database(&db).await;
    assert!(registry.current().degraded_reason().is_none());

    // `[]` is what serde would otherwise read as a struct of defaults.
    for corrupt in ["{not json", "[]", r#"{"max_writes_per_turn": "lots"}"#] {
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
            snapshot.guardian.policy().max_destructive_per_turn,
            1,
            "{corrupt} must not move the policy"
        );
        assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 9);
        assert!(snapshot
            .degraded_reason()
            .expect("the snapshot says so")
            .contains("does not parse"));
    }

    // Repairing the row to the very document in force still clears the mark.
    write_row(&db, &stored_document()).await;
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

    let registry = GuardianConfigRegistry::from_database(&db).await;
    let snapshot = registry.current();
    assert_eq!(snapshot.guardian.policy().max_destructive_per_turn, 3);
    assert!(snapshot
        .degraded_reason()
        .expect("marked degraded")
        .contains("does not parse"));

    write_row(&db, &stored_document()).await;
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Installed
    );
    assert_eq!(destructive_cap(&registry), 1);
    assert!(registry.current().degraded_reason().is_none());
}

/// A failed read says nothing about the row, so it changes nothing, and in
/// particular does not put a loaded instance back on the defaults.
#[tokio::test]
async fn a_read_failure_on_a_tick_keeps_the_policy() {
    let db = create_test_database().await.expect("test database");
    write_row(&db, &stored_document()).await;
    let registry = GuardianConfigRegistry::from_database(&db).await;

    break_reads(&db).await;

    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::KeptOnReadFailure
    );
    assert_eq!(destructive_cap(&registry), 1);
}

#[tokio::test]
async fn a_read_failure_at_boot_runs_the_defaults_marked_degraded() {
    let db = create_test_database().await.expect("test database");
    write_row(&db, &stored_document()).await;
    break_reads(&db).await;

    let registry = GuardianConfigRegistry::from_database(&db).await;
    let snapshot = registry.current();
    assert_eq!(snapshot.guardian.policy().max_destructive_per_turn, 3);
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
    let registry = GuardianConfigRegistry::from_database(&db).await;
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Unchanged,
        "no row and already on the defaults"
    );

    registry.install(stored_document());
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Installed
    );
    assert_eq!(
        *registry.current().document,
        GuardianConfigDocument::default()
    );
    assert_eq!(destructive_cap(&registry), 3);
}

#[test]
fn a_stored_zero_write_budget_resolves_to_the_default_and_says_so() {
    let registry = GuardianConfigRegistry::with_document(GuardianConfigDocument {
        max_writes_per_turn: Some(0),
        // A zero destructive budget is a posture a super admin can mean.
        max_destructive_per_turn: Some(0),
    });
    let snapshot = registry.current();
    let policy = snapshot.guardian.policy();
    assert_eq!(
        policy.max_writes_per_turn,
        GuardianPolicy::default().max_writes_per_turn
    );
    assert_eq!(policy.max_destructive_per_turn, 0);
    assert!(snapshot
        .degraded_reason()
        .expect("marked degraded")
        .contains("max_writes_per_turn=0"));
}

/// Keys this binary neither reads nor retired are reported, never silently
/// dropped; retired ones are reported apart, as the ignored keys they are.
#[test]
fn a_foreign_row_parses_and_reports_what_it_held() {
    let persisted = parse_persisted_document(
        r#"{"schema_version": 7, "mode": "observe", "max_writes_per_turn": 4, "shiny_new_rule": true}"#,
    )
    .expect("the fields this binary reads are valid");
    assert_eq!(persisted.document.max_writes_per_turn, Some(4));
    assert_eq!(persisted.unknown_fields, ["shiny_new_rule"]);
    assert_eq!(persisted.retired_fields, ["mode", "schema_version"]);
}

/// A row written by every earlier binary still loads, at boot and on a poll
/// tick, as a healthy load with its caps in force: the schema-2 shape the admin
/// PUT wrote until carnet#1471 (`mode` included, whatever it said, `off`
/// too), and the schema-1 shape with the knobs removed before that.
#[tokio::test]
async fn rows_an_earlier_binary_wrote_load_at_boot_and_on_a_tick_without_degrading() {
    let db = create_test_database().await.expect("test database");

    let schema_two = r#"{"schema_version":2,"mode":"observe","max_destructive_per_turn":1,"max_writes_per_turn":9}"#;
    db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, schema_two)
        .await
        .expect("row written");
    let registry = GuardianConfigRegistry::from_database(&db).await;
    let snapshot = registry.current();
    assert_eq!(*snapshot.document, stored_document());
    assert_eq!(snapshot.guardian.policy().max_destructive_per_turn, 1);
    assert_eq!(snapshot.guardian.policy().max_writes_per_turn, 9);
    assert!(snapshot.degraded_reason().is_none());
    assert_eq!(
        registry.reload_from_database(&db).await,
        GuardianReloadOutcome::Unchanged,
        "the row reads as what the snapshot already holds, not as a change each tick"
    );

    for row in [
        r#"{"schema_version":2,"mode":"enforce"}"#,
        r#"{"mode":"off"}"#,
        r#"{"schema_version":1,"plan_mode":"enforce","tainted_destructive":"deny","external_send":"all","mode":"observe","max_destructive_per_turn":2}"#,
    ] {
        db.set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, row)
            .await
            .expect("row written");
        registry.reload_from_database(&db).await;
        let ticked = registry.current();
        let booted = GuardianConfigRegistry::from_database(&db).await.current();
        for snapshot in [&ticked, &booted] {
            assert!(snapshot.degraded_reason().is_none(), "{row}");
            assert_eq!(
                snapshot.guardian.policy().max_destructive_per_turn,
                parse_persisted_document(row)
                    .expect("parses")
                    .document
                    .max_destructive_per_turn
                    .unwrap_or(3),
                "{row}"
            );
        }
    }
}
