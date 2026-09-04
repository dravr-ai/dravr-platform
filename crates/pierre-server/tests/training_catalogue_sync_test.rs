// ABOUTME: Integration test for the training-catalogue half of the contremaitre sync — the manifest's `training` section
// ABOUTME: Asserts the overlaid values the registry serves, unchanged hashes are skipped, a rejected file keeps the compiled-in entry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The catalogue is embedded at build time; a sync is how a flavour's cap or
//! a workout's default changes in production without a deploy. So the
//! assertions are on the numbers the registry serves after a sync — what
//! the coach would prescribe — never on a sync merely reporting a count.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use pierre_contremaitre::errors::ContremaitreError;
use pierre_contremaitre::evidence_registry::EvidenceRegistry;
use pierre_contremaitre::manifest::{
    compute_sha256, parse_manifest, Manifest, ManifestEntry, ManifestTraining,
};
use pierre_contremaitre::store::{PromptStore, StoredFile};
use pierre_contremaitre::sync::SyncResult;
use pierre_contremaitre::training_catalogue::{
    CatalogueKind, TrainingCatalogueRegistry, SELECTION_SLUG,
};
use pierre_contremaitre::training_sync::{sync_all_training, sync_changed_training};
use serde_json::Value;

/// A manifest with nothing but the required prompt section.
const BARE_MANIFEST: &str = r#"{"version":5,"prompts":{"system":{},"coaches":{},"personas":{}}}"#;

const POLARIZED_PATH: &str = "training/flavours/polarized-classic.yaml";
const HVLIT_PATH: &str = "training/flavours/hvlit-foundation.yaml";
const MARATHON_PATH: &str = "training/skeletons/marathon-linear.yaml";
const VO2MAX_PATH: &str = "training/workouts/vo2max_4x8.toml";
const SELECTION_PATH: &str = "training/selection.yaml";

/// A store holding a fixed set of files and the manifest that indexes
/// them, so a sync can be driven without a network and its failure paths
/// can be forced.
struct MemoryStore {
    files: HashMap<String, String>,
    training: ManifestTraining,
}

#[async_trait]
impl PromptStore for MemoryStore {
    async fn read_file(&self, path: &str) -> Result<StoredFile, ContremaitreError> {
        self.files
            .get(path)
            .map(|content| StoredFile {
                content: content.clone(),
                path: path.to_owned(),
            })
            .ok_or_else(|| ContremaitreError::ManifestParse(format!("no such file: {path}")))
    }

    async fn read_manifest(&self) -> Result<Manifest, ContremaitreError> {
        let mut manifest: Value = serde_json::from_str(BARE_MANIFEST).unwrap();
        manifest["training"] = serde_json::to_value(&self.training).unwrap();
        parse_manifest(&manifest.to_string())
    }

    fn backend_label(&self) -> &'static str {
        "memory"
    }
}

fn catalogue_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../training_catalogue")
}

/// The on-disk text of the catalogue file a manifest path names.
fn on_disk(path: &str) -> String {
    let relative = path.strip_prefix("training/").unwrap();
    fs::read_to_string(catalogue_dir().join(relative))
        .unwrap_or_else(|e| panic!("read training_catalogue/{relative}: {e}"))
}

/// Replace exactly one occurrence of `from` in the file, so the served body
/// differs from the compiled-in one by one known value.
fn altered(path: &str, from: &str, to: &str) -> String {
    let text = on_disk(path);
    assert_eq!(
        text.matches(from).count(),
        1,
        "{path}: {from:?} must appear once"
    );
    text.replace(from, to)
}

/// The manifest entry for a served body: its path and its real hash.
fn entry(path: &str, body: &str) -> ManifestEntry {
    ManifestEntry {
        path: path.to_owned(),
        sha256: compute_sha256(body.as_bytes()),
    }
}

/// A store serving one altered file of each kind plus one unchanged
/// flavour, indexed by a manifest whose hashes are the served bodies'.
fn altered_store() -> MemoryStore {
    let files: HashMap<String, String> = HashMap::from([
        (
            POLARIZED_PATH.to_owned(),
            altered(POLARIZED_PATH, "max_weeks: 12", "max_weeks: 10"),
        ),
        (HVLIT_PATH.to_owned(), on_disk(HVLIT_PATH)),
        (
            MARATHON_PATH.to_owned(),
            altered(MARATHON_PATH, "\nmin_weeks: 12\n", "\nmin_weeks: 13\n"),
        ),
        (
            VO2MAX_PATH.to_owned(),
            altered(
                VO2MAX_PATH,
                "duration_minutes = 65",
                "duration_minutes = 70",
            ),
        ),
        (
            SELECTION_PATH.to_owned(),
            altered(
                SELECTION_PATH,
                "note: \"Frequency and consistency first;",
                "note: \"Served from contremaitre;",
            ),
        ),
    ]);
    let training = ManifestTraining {
        flavours: HashMap::from([
            (
                "polarized-classic".to_owned(),
                entry(POLARIZED_PATH, &files[POLARIZED_PATH]),
            ),
            (
                "hvlit-foundation".to_owned(),
                entry(HVLIT_PATH, &files[HVLIT_PATH]),
            ),
        ]),
        skeletons: HashMap::from([(
            "marathon-linear".to_owned(),
            entry(MARATHON_PATH, &files[MARATHON_PATH]),
        )]),
        workouts: HashMap::from([(
            "vo2max_4x8".to_owned(),
            entry(VO2MAX_PATH, &files[VO2MAX_PATH]),
        )]),
        selection: Some(entry(SELECTION_PATH, &files[SELECTION_PATH])),
    };
    MemoryStore { files, training }
}

fn counts(result: &SyncResult) -> (usize, usize, usize) {
    (result.synced, result.skipped, result.failed)
}

#[tokio::test]
async fn the_store_manifest_carries_the_training_section() {
    let store = altered_store();
    let manifest = store.read_manifest().await.unwrap();
    assert_eq!(manifest.training.flavours.len(), 2);
    assert_eq!(manifest.training.skeletons.len(), 1);
    assert_eq!(manifest.training.workouts.len(), 1);
    assert_eq!(
        manifest
            .training
            .selection
            .as_ref()
            .map(|e| e.path.as_str()),
        Some(SELECTION_PATH)
    );
    assert_eq!(manifest.training.workouts["vo2max_4x8"].path, VO2MAX_PATH);
}

#[tokio::test]
async fn a_full_sync_overlays_every_changed_file_and_skips_the_unchanged_one() {
    let registry = TrainingCatalogueRegistry::new();
    let evidence = EvidenceRegistry::new();
    let store = altered_store();
    let seeded_hvlit = registry.flavour("hvlit-foundation").unwrap();

    let result = sync_all_training(&registry, &evidence, &store, &store.training)
        .await
        .unwrap();
    assert_eq!(counts(&result), (4, 1, 0));

    assert_eq!(
        registry.flavour("polarized-classic").unwrap().max_weeks,
        Some(10)
    );
    assert_eq!(registry.skeleton("marathon-linear").unwrap().min_weeks, 13);
    assert_eq!(registry.workout("vo2max_4x8").unwrap().duration_minutes, 70);
    let note = registry.selection().unwrap().rows[0].note.clone().unwrap();
    assert!(note.starts_with("Served from contremaitre;"), "{note}");
    assert_eq!(registry.flavour("hvlit-foundation").unwrap(), seeded_hvlit);

    for (kind, slug, path) in [
        (CatalogueKind::Flavour, "polarized-classic", POLARIZED_PATH),
        (CatalogueKind::Skeleton, "marathon-linear", MARATHON_PATH),
        (CatalogueKind::Workout, "vo2max_4x8", VO2MAX_PATH),
        (CatalogueKind::Selection, SELECTION_SLUG, SELECTION_PATH),
    ] {
        assert_eq!(
            registry.sha256(kind, slug),
            Some(compute_sha256(store.files[path].as_bytes())),
            "{path}: the registry holds the served body's hash"
        );
    }
    let stats = registry.stats();
    assert_eq!(
        (stats.compiled_in_count, stats.contremaitre_count),
        (50, 4),
        "{stats}"
    );
    assert_eq!(
        (stats.flavours, stats.skeletons, stats.workouts),
        (8, 12, 33),
        "{stats}"
    );

    // The same files at the same hashes are not re-applied.
    let again = sync_all_training(&registry, &evidence, &store, &store.training)
        .await
        .unwrap();
    assert_eq!(counts(&again), (0, 5, 0));
    assert_eq!(registry.workout("vo2max_4x8").unwrap().duration_minutes, 70);
    assert_eq!(registry.stats().contremaitre_count, 4);
}

#[tokio::test]
async fn a_webhook_push_applies_only_the_file_it_changed() {
    let registry = TrainingCatalogueRegistry::new();
    let evidence = EvidenceRegistry::new();
    let store = altered_store();
    let changed: HashSet<&str> = HashSet::from([VO2MAX_PATH]);

    let result = sync_changed_training(&registry, &evidence, &store, &store.training, &changed)
        .await
        .unwrap();
    assert_eq!(counts(&result), (1, 0, 0));
    assert_eq!(registry.workout("vo2max_4x8").unwrap().duration_minutes, 70);
    assert_eq!(
        registry.flavour("polarized-classic").unwrap().max_weeks,
        Some(12),
        "a flavour the push did not touch keeps its compiled-in value"
    );
    assert_eq!(registry.skeleton("marathon-linear").unwrap().min_weeks, 12);
    let stats = registry.stats();
    assert_eq!(
        (stats.compiled_in_count, stats.contremaitre_count),
        (53, 1),
        "{stats}"
    );

    // A path the manifest does not list applies nothing.
    let unknown: HashSet<&str> = HashSet::from(["training/workouts/no_such.toml"]);
    let nothing = sync_changed_training(&registry, &evidence, &store, &store.training, &unknown)
        .await
        .unwrap();
    assert_eq!(counts(&nothing), (0, 0, 0));
}

#[tokio::test]
async fn a_rejected_file_counts_failed_and_the_compiled_in_entry_stays_live() {
    let registry = TrainingCatalogueRegistry::new();
    let evidence = EvidenceRegistry::new();
    let seeded_flavour = registry.flavour("polarized-classic").unwrap();
    let seeded_workout = registry.workout("vo2max_4x8").unwrap();
    let seeded_flavour_sha = registry.sha256(CatalogueKind::Flavour, "polarized-classic");

    // One file that is not YAML, one that parses but breaks a catalogue rule.
    let files: HashMap<String, String> = HashMap::from([
        (POLARIZED_PATH.to_owned(), "{not yaml".to_owned()),
        (
            VO2MAX_PATH.to_owned(),
            altered(
                VO2MAX_PATH,
                "phases = [\"base\", \"build\", \"peak\"]",
                "phases = []",
            ),
        ),
    ]);
    let training = ManifestTraining {
        flavours: HashMap::from([(
            "polarized-classic".to_owned(),
            entry(POLARIZED_PATH, &files[POLARIZED_PATH]),
        )]),
        workouts: HashMap::from([(
            "vo2max_4x8".to_owned(),
            entry(VO2MAX_PATH, &files[VO2MAX_PATH]),
        )]),
        ..ManifestTraining::default()
    };
    let store = MemoryStore { files, training };

    let result = sync_all_training(&registry, &evidence, &store, &store.training)
        .await
        .unwrap();
    assert_eq!(counts(&result), (0, 0, 2));
    assert_eq!(
        registry.flavour("polarized-classic").unwrap(),
        seeded_flavour
    );
    assert_eq!(registry.workout("vo2max_4x8").unwrap(), seeded_workout);
    assert_eq!(
        registry.sha256(CatalogueKind::Flavour, "polarized-classic"),
        seeded_flavour_sha,
        "a rejected file leaves the seed's hash in place"
    );
    assert_eq!(registry.stats().contremaitre_count, 0);
}

#[tokio::test]
async fn a_missing_file_is_a_failure_not_a_panic() {
    let registry = TrainingCatalogueRegistry::new();
    let evidence = EvidenceRegistry::new();
    let training = ManifestTraining {
        workouts: HashMap::from([(
            "vo2max_4x8".to_owned(),
            ManifestEntry {
                path: VO2MAX_PATH.to_owned(),
                sha256: "0".repeat(64),
            },
        )]),
        ..ManifestTraining::default()
    };
    let store = MemoryStore {
        files: HashMap::new(),
        training,
    };
    let result = sync_all_training(&registry, &evidence, &store, &store.training)
        .await
        .unwrap();
    assert_eq!(counts(&result), (0, 0, 1));
    assert_eq!(registry.workout("vo2max_4x8").unwrap().duration_minutes, 65);
}

#[tokio::test]
async fn a_manifest_without_training_parses_and_syncs_nothing() {
    let manifest = parse_manifest(BARE_MANIFEST).unwrap();
    assert!(manifest.training.flavours.is_empty());
    assert!(manifest.training.skeletons.is_empty());
    assert!(manifest.training.workouts.is_empty());
    assert!(manifest.training.selection.is_none());

    let registry = TrainingCatalogueRegistry::new();
    let evidence = EvidenceRegistry::new();
    let store = MemoryStore {
        files: HashMap::new(),
        training: ManifestTraining::default(),
    };
    let before = registry.stats();
    let result = sync_all_training(&registry, &evidence, &store, &manifest.training)
        .await
        .unwrap();
    assert_eq!(counts(&result), (0, 0, 0));
    assert_eq!(registry.stats(), before, "the seed is untouched");
    assert_eq!(before.compiled_in_count, 54);
}
