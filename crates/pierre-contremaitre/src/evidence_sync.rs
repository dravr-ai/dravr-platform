// ABOUTME: Evidence-proposition half of the contremaitre sync — the manifest's domain → category → slug tree
// ABOUTME: Skips entries whose SHA-256 the EvidenceRegistry already holds; a parse failure keeps the prior entry live
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};

use tracing::{debug, warn};

use super::errors::ContremaitreError;
use super::evidence_registry::{parse_evidence_markdown, EvidenceRegistry};
use super::manifest::{compute_sha256, ManifestEntry};
use super::store::PromptStore;
use super::sync::{accumulate_outcome, SyncOutcome, SyncResult};

/// Nested `domain → category → slug → entry` shape used by the manifest's
/// evidence tree. Aliased to keep [`sync_all_evidence`] and
/// [`sync_changed_evidence`] signatures under the `type_complexity` threshold.
pub(crate) type EvidenceTree = HashMap<String, HashMap<String, HashMap<String, ManifestEntry>>>;

/// Iterate the manifest's evidence tree and apply each proposition,
/// skipping entries whose SHA-256 matches what's already in the registry.
pub(crate) async fn sync_all_evidence(
    registry: &EvidenceRegistry,
    store: &dyn PromptStore,
    evidence_tree: &EvidenceTree,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for categories in evidence_tree.values() {
        for (category, propositions) in categories {
            for (slug, entry) in propositions {
                if registry.sha256(category, slug).as_deref() == Some(&entry.sha256) {
                    debug!(category, slug, "evidence unchanged, skipping");
                    result.skipped += 1;
                    continue;
                }
                let outcome =
                    fetch_and_apply_evidence(registry, store, category, slug, entry).await;
                accumulate_outcome(&mut result, outcome);
            }
        }
    }

    Ok(result)
}

/// Sync only the evidence propositions whose paths appear in `changed_set`.
pub(crate) async fn sync_changed_evidence(
    registry: &EvidenceRegistry,
    store: &dyn PromptStore,
    evidence_tree: &EvidenceTree,
    changed_set: &HashSet<&str>,
) -> Result<SyncResult, ContremaitreError> {
    let mut result = SyncResult {
        synced: 0,
        skipped: 0,
        failed: 0,
    };

    for categories in evidence_tree.values() {
        for (category, propositions) in categories {
            for (slug, entry) in propositions {
                if !changed_set.contains(entry.path.as_str()) {
                    continue;
                }
                let outcome =
                    fetch_and_apply_evidence(registry, store, category, slug, entry).await;
                accumulate_outcome(&mut result, outcome);
            }
        }
    }

    Ok(result)
}

/// Download a single evidence markdown file and apply it to the registry.
async fn fetch_and_apply_evidence(
    registry: &EvidenceRegistry,
    store: &dyn PromptStore,
    category: &str,
    slug: &str,
    entry: &ManifestEntry,
) -> SyncOutcome {
    match store.read_file(&entry.path).await {
        Ok(file) => {
            let actual_sha = compute_sha256(file.content.as_bytes());
            match parse_evidence_markdown(&file.content) {
                Ok(corpus) => {
                    registry.update(category, slug, corpus, actual_sha);
                    debug!(category, slug, "synced evidence proposition");
                    SyncOutcome::Synced
                }
                Err(e) => {
                    warn!(category, slug, error = %e, "failed to parse evidence markdown");
                    SyncOutcome::Failed
                }
            }
        }
        Err(e) => {
            warn!(category, slug, error = %e, "failed to download evidence proposition");
            SyncOutcome::Failed
        }
    }
}
