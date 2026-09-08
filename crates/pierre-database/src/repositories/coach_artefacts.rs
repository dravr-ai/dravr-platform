// ABOUTME: Repository trait for coach package artefacts — the flavour, skeleton and workout files stored per coach
// ABOUTME: The seeder replaces a coach's set wholesale; the resolver and the admin review read it back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::{CoachArtefact, PackageArtefact};

/// Persistent storage for a coach package's training artefacts.
///
/// Tenant-scoped through the coach row: `coach_artefacts` carries the
/// coach's `tenant_id` so every query includes it, and the caller has
/// already resolved the coach it asks about. A package is written as a set —
/// the files beside one prompt at one seed run — so the write replaces
/// whatever the coach carried before rather than merging: an artefact
/// deleted from the checkout leaves the database on the next seed, the way
/// a retired coach does.
#[async_trait]
pub trait CoachArtefactRepository: Send + Sync {
    /// Replace the coach's artefacts with `artefacts`, in one transaction.
    ///
    /// Returns how many rows the coach carries afterwards. An empty set
    /// clears the package.
    async fn replace_coach_artefacts(
        &self,
        tenant_id: &str,
        coach_id: &str,
        artefacts: &[PackageArtefact],
    ) -> AppResult<usize>;

    /// Every artefact the coach carries, ordered by `(kind, slug)` so two
    /// reads of the same package list it the same way.
    async fn list_coach_artefacts(
        &self,
        tenant_id: &str,
        coach_id: &str,
    ) -> AppResult<Vec<CoachArtefact>>;
}
