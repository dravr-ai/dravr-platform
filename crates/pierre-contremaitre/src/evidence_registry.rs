// ABOUTME: Hot-reloadable evidence registry for the Tier 5.5 bullshit detector
// ABOUTME: Mirrors ToolDescriptionRegistry — YAML frontmatter markdown files, keyed by (category, slug)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Evidence Registry
//!
//! Tier 5.5 counterpart to [`super::PromptRegistry`] and [`super::ToolDescriptionRegistry`].
//!
//! Holds one [`EvidenceCorpus`] per claim category, populated at startup
//! from `dravr-contremaitre` and updated via webhook push events.
//!
//! The registry starts empty — the compiled-in fallback in
//! `services::claim_verification::EMBEDDED_PROPOSITIONS` is used whenever
//! the registry has no records for a given category. This matches the
//! fallback philosophy used by [`super::PromptRegistry`]: the server
//! always boots and functions correctly even when GitHub is unreachable.
//!
//! Entries are keyed by `(category, slug)` where `category` is the stable
//! [`ClaimCategory::as_str`] form and `slug` is the filename stem (for
//! example `morton-2018-protein-intake`). This is what the manifest's
//! `evidence.sports_science.{category}.{slug}` path resolves to.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use pierre_evals::evidence_retriever::EvidenceCorpus;
use pierre_memory::ClaimCategory;

use super::errors::ContremaitreError;
use super::registry::PromptSource;

/// A single proposition entry in the registry.
#[derive(Debug, Clone)]
pub struct EvidenceEntry {
    /// Evidence corpus parsed from this proposition's markdown file.
    ///
    /// In practice a single proposition file yields a one-record corpus;
    /// we store the whole [`EvidenceCorpus`] so calling code can extend
    /// larger category corpora from it without re-parsing.
    pub corpus: EvidenceCorpus,
    /// SHA-256 hex digest of the markdown content.
    pub sha256: String,
    /// Where this entry was loaded from.
    pub source: PromptSource,
    /// When this entry was loaded or last updated.
    pub loaded_at: DateTime<Utc>,
}

/// Thread-safe registry for externalized sports-science evidence.
///
/// Starts empty; the compiled-in fallback in
/// [`crate::services::claim_verification`] serves as the bootstrap corpus.
/// Entries are populated by the contremaitre sync and updated via webhook.
pub struct EvidenceRegistry {
    /// Keyed by `(category_str, slug)` — e.g. `("nutrition",
    /// "morton-2018-protein-intake")`.
    entries: RwLock<HashMap<(String, String), EvidenceEntry>>,
}

impl EvidenceRegistry {
    /// Create an empty registry.
    ///
    /// The compiled-in propositions are the bootstrap fallback and live
    /// in the `claim_verification` service.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// True when the registry has no entries.
    ///
    /// Callers use this to decide whether to fall back to the compiled-in
    /// corpus.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// Total number of proposition entries across all categories.
    #[must_use]
    pub fn count(&self) -> usize {
        self.read().len()
    }

    /// Return the current SHA-256 hash for a registered proposition, if any.
    #[must_use]
    pub fn sha256(&self, category: &str, slug: &str) -> Option<String> {
        self.read()
            .get(&(category.to_owned(), slug.to_owned()))
            .map(|e| e.sha256.clone())
    }

    /// Insert or update a proposition.
    pub fn update(&self, category: &str, slug: &str, corpus: EvidenceCorpus, sha256: String) {
        self.write().insert(
            (category.to_owned(), slug.to_owned()),
            EvidenceEntry {
                corpus,
                sha256,
                source: PromptSource::Contremaitre,
                loaded_at: Utc::now(),
            },
        );
    }

    /// Remove a proposition.
    ///
    /// Reverts callers to the compiled-in fallback for that slot on the
    /// next lookup. Returns `true` if an entry was removed.
    #[must_use]
    pub fn remove(&self, category: &str, slug: &str) -> bool {
        self.write()
            .remove(&(category.to_owned(), slug.to_owned()))
            .is_some()
    }

    /// Assemble an [`EvidenceCorpus`] for the given category.
    ///
    /// Returns an empty corpus when the registry has no entries for that
    /// category; callers should fall back to the compiled-in corpus in
    /// that case.
    #[must_use]
    pub fn corpus_for(&self, category: ClaimCategory) -> EvidenceCorpus {
        let guard = self.read();
        let cat_str = category.as_str();
        let mut combined = EvidenceCorpus::default();
        for ((c, _slug), entry) in &*guard {
            if c == cat_str {
                combined.extend(entry.corpus.clone());
            }
        }
        combined
    }

    /// Assemble an [`EvidenceCorpus`] containing every proposition.
    ///
    /// Used by the `claim_verification` service as the runtime corpus
    /// source. Callers that want category-scoped recall should prefer
    /// [`Self::corpus_for`] to keep the working set small.
    #[must_use]
    pub fn full_corpus(&self) -> EvidenceCorpus {
        let guard = self.read();
        let mut combined = EvidenceCorpus::default();
        for entry in guard.values() {
            combined.extend(entry.corpus.clone());
        }
        combined
    }

    /// List all registered entries sorted by `(category, slug)`.
    ///
    /// Used by the admin/diagnostics surface to show which propositions
    /// have been synced from contremaitre.
    #[must_use]
    pub fn list(&self) -> Vec<(String, String, EvidenceEntry)> {
        let guard = self.read();
        let mut out: Vec<(String, String, EvidenceEntry)> = guard
            .iter()
            .map(|((c, s), e)| (c.clone(), s.clone(), e.clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        out
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<(String, String), EvidenceEntry>> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, HashMap<(String, String), EvidenceEntry>> {
        self.entries.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for EvidenceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a markdown proposition file into a single-record [`EvidenceCorpus`].
///
/// Thin wrapper around [`EvidenceCorpus::from_markdown`] that maps parse
/// failures into [`ContremaitreError::ManifestParse`] so the sync engine
/// can route them through the same error channel it uses for YAML tool
/// descriptions.
///
/// # Errors
///
/// Returns [`ContremaitreError::ManifestParse`] if the frontmatter is
/// missing, malformed, or the body is empty.
pub fn parse_evidence_markdown(contents: &str) -> Result<EvidenceCorpus, ContremaitreError> {
    EvidenceCorpus::from_markdown(contents)
        .map_err(|e| ContremaitreError::ManifestParse(format!("evidence markdown: {e}")))
}
