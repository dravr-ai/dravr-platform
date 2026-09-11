// ABOUTME: Repository trait for agent package artefacts — the flavour, skeleton and workout files stored per agent
// ABOUTME: The seeder replaces an agent's set wholesale; the resolver and the admin review read it back
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::{AgentArtefact, PackageArtefact};

/// Persistent storage for an agent package's training artefacts.
///
/// Tenant-scoped through the agent row: `agent_artefacts` carries the
/// agent's `tenant_id` so every query includes it, and the caller has
/// already resolved the agent it asks about. A package is written as a set —
/// the files beside one prompt at one seed run — so the write replaces
/// whatever the agent carried before rather than merging: an artefact
/// deleted from the checkout leaves the database on the next seed, the way
/// a retired agent does.
#[async_trait]
pub trait AgentArtefactRepository: Send + Sync {
    /// Replace the agent's artefacts with `artefacts`, in one transaction.
    ///
    /// Returns how many rows the agent carries afterwards. An empty set
    /// clears the package.
    async fn replace_agent_artefacts(
        &self,
        tenant_id: &str,
        agent_id: &str,
        artefacts: &[PackageArtefact],
    ) -> AppResult<usize>;

    /// Every artefact the agent carries, ordered by `(kind, slug)` so two
    /// reads of the same package list it the same way.
    async fn list_agent_artefacts(
        &self,
        tenant_id: &str,
        agent_id: &str,
    ) -> AppResult<Vec<AgentArtefact>>;
}
