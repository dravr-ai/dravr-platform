// ABOUTME: Adopts the activity cache into the activity store: every cached row of a live source is kept in barrage
// ABOUTME: Each row keeps its key and its sync stamp; each (user, key) is checked held == cached afterwards

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Adopting the activity cache (carnet#1482).
//!
//! Until every provider read went through [`LiveActivityStore`], the rows of
//! `cached_activities` were the only copy of the activities the platform
//! had read. Adoption keeps each of them in the store, under the key it is
//! cached under and at the `synced_at` it carries, once per environment
//! after deploy. The store projects them back into the cache unchanged, so
//! the readers see nothing move. An archive's rows are skipped: its import
//! already kept them in the store.
//!
//! Afterwards each `(user, key)` is checked: the store must hold as many
//! records as the cache holds rows. A row whose id the store cannot hold, or
//! a key the store does not know, shows as a difference.

use std::fmt;

use chrono::{DateTime, Utc};
use dravr_barrage::SourceKey;
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::{CachedKeyCount, StoredActivityRow};
use pierre_database::RepositoryRegistry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::live_activities::LiveActivityStore;

/// At most this many rows go to the store in one ingest.
const INGEST_BATCH: usize = 200;

/// What the store makes of a cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKeyKind {
    /// A live source: its rows are adopted.
    Live,
    /// An archive: its import already kept its records in the store.
    Archive,
    /// A key the store does not know: its rows cannot be adopted.
    Unknown,
}

impl CacheKeyKind {
    /// The kind of the cache key `provider`.
    #[must_use]
    pub fn of(provider: &str) -> Self {
        match SourceKey::parse(provider) {
            Ok(SourceKey::Live(_)) => Self::Live,
            Ok(_) => Self::Archive,
            Err(_) => Self::Unknown,
        }
    }
}

/// How one `(user, key)` came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionOutcome {
    /// The store holds as many records as the cache holds rows.
    Matched,
    /// The store holds another number of records than the cache holds rows.
    Mismatched,
    /// An archive: left to its import.
    Skipped,
    /// A key the store does not know: nothing was adopted.
    Unadoptable,
    /// A dry run: these rows would be adopted.
    WouldAdopt,
}

impl AdoptionOutcome {
    /// Whether the operator has to act on it: the counts differ, or the rows
    /// cannot be adopted at all.
    #[must_use]
    pub const fn is_failure(self) -> bool {
        matches!(self, Self::Mismatched | Self::Unadoptable)
    }
}

/// One `(tenant, user, key)` of the cache, and what adoption did with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptionRow {
    /// The tenant.
    pub tenant_id: String,
    /// The user.
    pub user_id: Uuid,
    /// The cache key.
    pub provider: String,
    /// What the store makes of the key.
    pub kind: CacheKeyKind,
    /// The rows the cache holds.
    pub cached: u64,
    /// The records the store held before; `None` for a key it does not hold.
    pub held_before: Option<u64>,
    /// The records the store holds after; `None` on a dry run and for a key
    /// it does not hold.
    pub held_after: Option<u64>,
    /// How it came out.
    pub outcome: AdoptionOutcome,
}

/// What one adoption did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptionReport {
    /// Whether it wrote nothing.
    pub dry_run: bool,
    /// Every `(tenant, user, key)` of the cache, in tenant, user, key order.
    pub rows: Vec<AdoptionRow>,
}

impl AdoptionReport {
    /// How many rows the operator has to act on ([`AdoptionOutcome::is_failure`]).
    #[must_use]
    pub fn failures(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.outcome.is_failure())
            .count()
    }
}

/// Which part of the cache one adoption covers.
///
/// Every tenant's, or one tenant's, narrowed to one user and one key when
/// named. A whole environment is adopted one `(user, key)` per call, so no
/// call outlasts a request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdoptionScope {
    /// Only this tenant.
    pub tenant: Option<TenantId>,
    /// Only this user.
    pub user_id: Option<Uuid>,
    /// Only this cache key.
    pub provider: Option<String>,
}

impl AdoptionScope {
    /// Whether `key` is in the scope.
    fn covers(&self, key: &CachedKeyCount) -> bool {
        self.user_id.is_none_or(|user| user == key.user_id)
            && self
                .provider
                .as_deref()
                .is_none_or(|provider| provider == key.provider)
    }
}

/// Adopts the activity cache into the live store.
pub struct CacheAdoption<'repos> {
    repos: &'repos RepositoryRegistry,
    store: &'repos dyn LiveActivityStore,
}

impl fmt::Debug for CacheAdoption<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheAdoption").finish_non_exhaustive()
    }
}

impl<'repos> CacheAdoption<'repos> {
    /// An adoption of `repos`' cache into `store`.
    #[must_use]
    pub const fn new(
        repos: &'repos RepositoryRegistry,
        store: &'repos dyn LiveActivityStore,
    ) -> Self {
        Self { repos, store }
    }

    /// Adopt the cache `scope` covers; with `dry_run`, only say what would be
    /// adopted and write nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope's tenant does not exist, or a read or
    /// write of the cache or the store fails; the report of the pairs adopted
    /// before it is lost with it, and a run again adopts them again
    /// harmlessly: a row ingested again at its own stamp is stored and
    /// projected as it was.
    pub async fn run(&self, scope: &AdoptionScope, dry_run: bool) -> AppResult<AdoptionReport> {
        let tenants = if let Some(id) = scope.tenant {
            vec![self.repos.tenants.get_by_id(id).await?.id]
        } else {
            let mut ids: Vec<TenantId> = self
                .repos
                .tenants
                .get_all()
                .await?
                .into_iter()
                .map(|t| t.id)
                .collect();
            ids.sort_by_key(ToString::to_string);
            ids
        };
        let mut rows = Vec::new();
        for tenant_id in tenants {
            for key in self
                .repos
                .activity_cache
                .count_by_user_and_provider(&tenant_id)
                .await?
                .into_iter()
                .filter(|key| scope.covers(key))
            {
                rows.push(self.adopt_key(tenant_id, key, dry_run).await?);
            }
        }
        Ok(AdoptionReport { dry_run, rows })
    }

    /// Adopt one `(user, key)` of `tenant_id`.
    async fn adopt_key(
        &self,
        tenant_id: TenantId,
        key: CachedKeyCount,
        dry_run: bool,
    ) -> AppResult<AdoptionRow> {
        let kind = CacheKeyKind::of(&key.provider);
        let mut row = AdoptionRow {
            tenant_id: tenant_id.to_string(),
            user_id: key.user_id,
            provider: key.provider,
            kind,
            cached: key.rows,
            held_before: None,
            held_after: None,
            outcome: match kind {
                CacheKeyKind::Archive => AdoptionOutcome::Skipped,
                CacheKeyKind::Unknown => AdoptionOutcome::Unadoptable,
                CacheKeyKind::Live => AdoptionOutcome::WouldAdopt,
            },
        };
        if kind != CacheKeyKind::Live {
            return Ok(row);
        }
        row.held_before = Some(
            self.store
                .held(row.user_id, tenant_id, &row.provider)
                .await?,
        );
        if dry_run {
            return Ok(row);
        }
        let stored = self
            .repos
            .activity_cache
            .get_stored_activity_rows(row.user_id, &tenant_id, &row.provider)
            .await?;
        for (synced_at, activities) in by_stamp(stored) {
            for batch in activities.chunks(INGEST_BATCH) {
                self.store
                    .ingest(row.user_id, tenant_id, &row.provider, batch, synced_at)
                    .await?;
            }
        }
        let held = self
            .store
            .held(row.user_id, tenant_id, &row.provider)
            .await?;
        row.held_after = Some(held);
        row.outcome = if held == row.cached {
            AdoptionOutcome::Matched
        } else {
            AdoptionOutcome::Mismatched
        };
        Ok(row)
    }
}

/// `rows`, oldest sync first, as runs of the activities sharing one stamp:
/// each run goes to the store at its own stamp, so no row's `synced_at`
/// moves when the store projects it back.
fn by_stamp(rows: Vec<StoredActivityRow>) -> Vec<(DateTime<Utc>, Vec<Activity>)> {
    let mut runs: Vec<(DateTime<Utc>, Vec<Activity>)> = Vec::new();
    for row in rows {
        match runs.last_mut() {
            Some((stamp, activities)) if *stamp == row.synced_at => activities.push(row.activity),
            _ => runs.push((row.synced_at, vec![row.activity])),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_key_is_live_archive_or_unknown_as_the_store_reads_it() {
        assert_eq!(
            CacheKeyKind::of("strava"),
            CacheKeyKind::Live,
            "bug: a live key is not adopted"
        );
        assert_eq!(
            CacheKeyKind::of("upload"),
            CacheKeyKind::Live,
            "bug: uploads, written to the cache before the store kept them, are not adopted"
        );
        assert_eq!(
            CacheKeyKind::of("strava_archive"),
            CacheKeyKind::Archive,
            "bug: an archive, already in the store, is adopted again"
        );
        assert_eq!(
            CacheKeyKind::of("no such key"),
            CacheKeyKind::Unknown,
            "bug: a key the store refuses is not reported"
        );
    }

    #[test]
    fn only_a_difference_or_an_unadoptable_key_is_a_failure() {
        assert!(
            AdoptionOutcome::Mismatched.is_failure(),
            "a difference is a failure"
        );
        assert!(
            AdoptionOutcome::Unadoptable.is_failure(),
            "an unadoptable key is a failure"
        );
        assert!(
            !AdoptionOutcome::Matched.is_failure(),
            "a match is not a failure"
        );
        assert!(
            !AdoptionOutcome::Skipped.is_failure(),
            "a skip is not a failure"
        );
        assert!(
            !AdoptionOutcome::WouldAdopt.is_failure(),
            "a dry run's would-adopt is not a failure"
        );
    }
}
