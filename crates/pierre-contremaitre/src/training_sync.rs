// ABOUTME: Training-catalogue half of the contremaitre sync — the manifest's `training` section, one file per entry
// ABOUTME: Skips entries whose SHA-256 the registry already holds; a file the kernel rejects keeps the prior entry live
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use tracing::{debug, warn};

use super::errors::ContremaitreError;
use super::evidence_registry::EvidenceRegistry;
use super::manifest::{compute_sha256, ManifestEntry, ManifestTraining};
use super::store::PromptStore;
use super::sync::{accumulate_outcome, SyncOutcome, SyncResult};
use super::training_catalogue::{
    parse_catalogue_file, CatalogueKind, TrainingCatalogueRegistry, SELECTION_SLUG,
};

/// The manifest's `training` section flattened to `(kind, slug, entry)`, in
/// section order — the three keyed maps, then the one selection table.
fn manifest_entries(training: &ManifestTraining) -> Vec<(CatalogueKind, &str, &ManifestEntry)> {
    let keyed: [(CatalogueKind, &HashMap<String, ManifestEntry>); 3] = [
        (CatalogueKind::Flavour, &training.flavours),
        (CatalogueKind::Skeleton, &training.skeletons),
        (CatalogueKind::Workout, &training.workouts),
    ];
    let mut out: Vec<(CatalogueKind, &str, &ManifestEntry)> = keyed
        .into_iter()
        .flat_map(|(kind, map)| {
            map.iter()
                .map(move |(slug, entry)| (kind, slug.as_str(), entry))
        })
        .collect();
    if let Some(entry) = training.selection.as_ref() {
        out.push((CatalogueKind::Selection, SELECTION_SLUG, entry));
    }
    out
}

/// Sync every catalogue file the manifest lists (full-sync path).
///
/// A file whose manifest hash matches the registry's is skipped; one that
/// fails to download or that the kernel rejects is counted `failed` and
/// the previous entry — compiled-in or an earlier overlay — stays live.
///
/// # Errors
///
/// The per-file failures are counted, not raised; the `Result` is the
/// shape every section of the sync returns so the caller folds them alike.
pub async fn sync_all_training(
    registry: &TrainingCatalogueRegistry,
    evidence_registry: &EvidenceRegistry,
    store: &dyn PromptStore,
    training: &ManifestTraining,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (kind, slug, entry) in manifest_entries(training) {
        if registry.sha256(kind, slug).as_deref() == Some(&entry.sha256) {
            debug!(
                kind = kind.as_str(),
                slug, "catalogue file unchanged, skipping"
            );
            result.skipped += 1;
            continue;
        }
        let outcome = fetch_and_apply_catalogue_file(registry, store, kind, slug, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    warn_unresolved_references(registry, evidence_registry);
    Ok(result)
}

/// Sync only the catalogue files whose repo path appears in `changed_set`
/// (selective sync from a webhook push).
///
/// # Errors
///
/// The per-file failures are counted, not raised; the `Result` is the
/// shape every section of the sync returns so the caller folds them alike.
pub async fn sync_changed_training<S: BuildHasher + Sync>(
    registry: &TrainingCatalogueRegistry,
    evidence_registry: &EvidenceRegistry,
    store: &dyn PromptStore,
    training: &ManifestTraining,
    changed_set: &HashSet<&str, S>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for (kind, slug, entry) in manifest_entries(training) {
        if !changed_set.contains(entry.path.as_str()) {
            continue;
        }
        let outcome = fetch_and_apply_catalogue_file(registry, store, kind, slug, entry).await;
        accumulate_outcome(&mut result, outcome);
    }

    warn_unresolved_references(registry, evidence_registry);
    Ok(result)
}

/// Download one catalogue file and overlay it on the registry.
async fn fetch_and_apply_catalogue_file(
    registry: &TrainingCatalogueRegistry,
    store: &dyn PromptStore,
    kind: CatalogueKind,
    slug: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => apply_downloaded_catalogue_file(registry, kind, slug, entry, &file.content),
        Err(e) => {
            warn!(kind = kind.as_str(), slug, error = %e, "failed to download catalogue file");
            SyncOutcome::Failed
        }
    }
}

/// Parse and validate a downloaded file through the kernel and overlay it,
/// keeping the previous entry live when the kernel rejects it. A manifest
/// hash that differs from the downloaded text is logged and the downloaded
/// text wins, as for every other synced file.
fn apply_downloaded_catalogue_file(
    registry: &TrainingCatalogueRegistry,
    kind: CatalogueKind,
    slug: &str,
    entry: &ManifestEntry,
    content: &str,
) -> SyncOutcome {
    let actual_sha = compute_sha256(content.as_bytes());
    if actual_sha != entry.sha256 {
        warn!(
            kind = kind.as_str(),
            slug,
            expected = entry.sha256,
            actual = actual_sha,
            "manifest hash mismatch for catalogue file, using downloaded content"
        );
    }
    match parse_catalogue_file(kind, content) {
        Ok(item) => {
            registry.update(kind, slug, item, actual_sha);
            debug!(kind = kind.as_str(), slug, "synced catalogue file");
            SyncOutcome::Synced
        }
        Err(e) => {
            warn!(
                kind = kind.as_str(),
                slug,
                error = %e,
                "catalogue file rejected by the kernel — keeping previous entry"
            );
            SyncOutcome::Failed
        }
    }
}

/// After every pass, name each cross-file reference the live set leaves
/// dangling: an evidence path no synced proposition answers for, a
/// selection id with no flavour, a purpose no workout carries.
///
/// Evidence resolution is checked against the synced registry only — an
/// empty one means the sync has not reached the propositions yet (or is
/// disabled), and every path would read as unresolved, so the pass is
/// skipped until it fills. A catalogue with nothing dangling logs nothing;
/// one with a dangling reference says so on every tick, because the coach
/// is prescribing from it in the meantime.
fn warn_unresolved_references(
    registry: &TrainingCatalogueRegistry,
    evidence_registry: &EvidenceRegistry,
) {
    if evidence_registry.is_empty() {
        return;
    }
    let exists = |category: &str, slug: &str| evidence_registry.sha256(category, slug).is_some();
    for unresolved in registry.unresolved_references(&exists) {
        warn!(
            owner = %unresolved.owner,
            key = %unresolved.key,
            reference = %unresolved.reference,
            "training catalogue reference does not resolve"
        );
    }
}
