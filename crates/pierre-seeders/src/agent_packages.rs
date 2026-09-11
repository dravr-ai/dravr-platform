// ABOUTME: The agent seeder's package pass — stores the flavour, skeleton and workouts an agent directory ships beside its prompt
// ABOUTME: A package is replaced as a set per agent on every run; a directory with no package files clears what the agent carried
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent package artefacts, seeded
//!
//! Pass 1 of [`super::agents`] writes the agent row; this pass follows it
//! with the package the same directory carries — `flavour.yaml`,
//! `skeleton.yaml`, `workouts/*.toml` — read through
//! [`pierre_agent_parser::read_package_artefacts`], which runs each file
//! through the periodization kernel. A package the kernel refuses is logged
//! and left as it was: half a package would resolve a workout slug to a
//! template whose skeleton never landed.
//!
//! The write is the set: what the checkout carries replaces what the row
//! carried, so a file removed from contremaitre leaves the database on the
//! next seed the way a retired agent does.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};

use glob::glob;
use pierre_agent_parser::{has_package_files, read_package_artefacts};
use pierre_core::models::PackageArtefact;
use pierre_database::RepositoryRegistry;
use tracing::{debug, info, warn};

/// What the pass did, for the seeder's summary.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PackageStats {
    /// Coaches whose package was written (or would be, under dry-run).
    pub packages_synced: u32,
    /// Artefact rows those packages hold.
    pub artefacts_synced: u32,
    /// Packages the parser or the store refused, `slug: reason`.
    pub errors: Vec<String>,
}

/// Store the package of every seeded agent.
///
/// `slug_to_id` is Pass 1's map of the agents that have a row; an agent that
/// failed to upsert has nothing to attach a package to and is skipped here
/// too. `tenant_id` is the catalogue admin's tenant, the one every seeded
/// agent row carries.
pub async fn sync_packages<S: BuildHasher + Sync>(
    repos: &RepositoryRegistry,
    agents_dir: &Path,
    slug_to_id: &HashMap<String, String, S>,
    tenant_id: &str,
    dry_run: bool,
) -> PackageStats {
    info!("");
    info!("=== Pass 1b: Syncing Coach Packages ===");
    let mut stats = PackageStats::default();
    let mut slugs: Vec<&String> = slug_to_id.keys().collect();
    slugs.sort();
    for slug in slugs {
        let Some(agent_id) = slug_to_id.get(slug) else {
            continue;
        };
        let Some(dir) = agent_dir(agents_dir, slug) else {
            continue;
        };
        sync_one(repos, &dir, slug, agent_id, tenant_id, dry_run, &mut stats).await;
    }
    info!(
        "  → {} package(s), {} artefact(s) synced",
        stats.packages_synced, stats.artefacts_synced
    );
    stats
}

/// Read one agent's package and replace its rows.
///
/// A directory with no package files still writes — an empty set — when the
/// agent carried rows before, so a package deleted from the checkout is
/// deleted here; that read is skipped under dry-run, where nothing is
/// written anyway.
async fn sync_one(
    repos: &RepositoryRegistry,
    dir: &Path,
    slug: &str,
    agent_id: &str,
    tenant_id: &str,
    dry_run: bool,
    stats: &mut PackageStats,
) {
    let artefacts = match read_package_artefacts(dir) {
        Ok(a) => a,
        Err(e) => {
            record_error(stats, slug, &e.to_string());
            return;
        }
    };
    let packageless = artefacts.is_empty() && !has_package_files(dir);
    if packageless && (dry_run || !carries_rows(repos, tenant_id, agent_id).await) {
        return;
    }
    if dry_run {
        log_dry_run(slug, &artefacts);
        record_synced(stats, artefacts.len());
        return;
    }
    match repos
        .agent_artefacts
        .replace_agent_artefacts(tenant_id, agent_id, &artefacts)
        .await
    {
        Ok(count) => {
            debug!("  + {} package ({} artefact(s))", slug, count);
            record_synced(stats, count);
        }
        Err(e) => record_error(stats, slug, &e.to_string()),
    }
}

fn record_synced(stats: &mut PackageStats, artefacts: usize) {
    stats.packages_synced += 1;
    stats.artefacts_synced += u32::try_from(artefacts).unwrap_or(u32::MAX);
}

fn record_error(stats: &mut PackageStats, slug: &str, error: &str) {
    warn!("  ✗ {} package - Error: {}", slug, error);
    stats.errors.push(format!("{slug} package: {error}"));
}

async fn carries_rows(repos: &RepositoryRegistry, tenant_id: &str, agent_id: &str) -> bool {
    match repos
        .agent_artefacts
        .list_agent_artefacts(tenant_id, agent_id)
        .await
    {
        Ok(rows) => !rows.is_empty(),
        Err(e) => {
            warn!(agent_id, error = %e, "could not read the coach's stored package");
            false
        }
    }
}

fn log_dry_run(slug: &str, artefacts: &[PackageArtefact]) {
    for a in artefacts {
        info!("  + [dry-run] {} package {} '{}'", slug, a.kind, a.slug);
    }
    if artefacts.is_empty() {
        info!("  - [dry-run] {} package cleared", slug);
    }
}

/// The agent's directory: `agents_dir/<category>/<slug>`, whatever the
/// category directory is called — the layout the prompt scan walks.
fn agent_dir(agents_dir: &Path, slug: &str) -> Option<PathBuf> {
    let pattern = agents_dir.join("*").join(slug);
    glob(&pattern.to_string_lossy())
        .ok()?
        .filter_map(Result::ok)
        .find(|p| p.is_dir())
}
