// ABOUTME: The one retention sweep — hourly, it expires provider activities in barrage and wellness in the platform
// ABOUTME: Both halves apply one policy read from activity_retention.* config; each expiry writes its attestation (cache_expired, wellness_expired)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The retention sweep (carnet#1482, D8).
//!
//! One worker applies the one retention policy ([`ProviderRetention`]) to
//! every copy of provider data the platform holds:
//!
//! 1. **Activities**, in the activity store: dravr-barrage's
//!    [`RetentionSweep`] expires each live source's records last read before
//!    the source's cutoff, retracting each from the cache through the
//!    projection sink, which writes the `cache_expired` attestation, then
//!    deleting it. An athlete's own exports never expire.
//! 2. **Wellness**, in the platform's tables:
//!    [`ProviderDataRepository::expire_provider_wellness`] deletes, per
//!    provider, the rows last synced before the same cutoff, each scope with
//!    its `wellness_expired` attestation, which names no activity.
//!
//! The policy is read on every pass, so a `pierre-cli config set` takes
//! effect on the next one without a restart. Under the start values nothing
//! expires, and a pass does no work beyond reading the two parameters.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::{ProjectionSink, RetentionSweep, SourceKey};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{
    ProviderDataRepository, ProviderWellnessExpiry, WorkerRunRepository,
};
use pierre_runtime_context::AdminConfigLookup;
use tokio::task::AbortHandle;
use tracing::info;

use crate::periodic::spawn_periodic;
use crate::retention_policy::ProviderRetention;

/// The ledger name the sweep runs under.
const WORKER_NAME: &str = "provider data retention sweep";

/// Sweep cadence. A retention is counted in days, so an hourly pass holds a
/// copy at most an hour past it.
const SWEEP_INTERVAL: Duration = Duration::from_hours(1);

/// Most pages of expired records one source gives up per pass (500 records
/// each, barrage's page): a backlog drains over the following passes.
const PAGES_PER_PASS: u32 = 20;

/// What the sweep works over.
pub struct RetentionSweeper {
    sweep: RetentionSweep,
    provider_data: Arc<dyn ProviderDataRepository>,
    config: Arc<dyn AdminConfigLookup>,
    wellness_providers: Vec<String>,
}

/// What one pass expired.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionPass {
    /// Activity records expired, per source.
    pub activities: Vec<(SourceKey, u64)>,
    /// Wellness expired, per provider that held any.
    pub wellness: Vec<(String, ProviderWellnessExpiry)>,
}

impl fmt::Debug for RetentionSweeper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetentionSweeper")
            .field("wellness_providers", &self.wellness_providers)
            .finish_non_exhaustive()
    }
}

impl RetentionSweeper {
    /// A sweep over the activity store's `records`, retracting through
    /// `sink`, and over `provider_data`'s wellness of `wellness_providers`
    /// (the registry's providers; the policy's overrides are added to them),
    /// under the policy `config` holds.
    #[must_use]
    pub fn new(
        records: Arc<dyn ActivityRecordStore>,
        sink: Arc<dyn ProjectionSink>,
        provider_data: Arc<dyn ProviderDataRepository>,
        config: Arc<dyn AdminConfigLookup>,
        wellness_providers: Vec<String>,
    ) -> Self {
        Self {
            sweep: RetentionSweep::new(records, sink),
            provider_data,
            config,
            wellness_providers,
        }
    }

    /// Expire everything the policy, as configured now, says has been held
    /// past its retention at `now`.
    ///
    /// # Errors
    ///
    /// The first configuration, store or database error; what expired
    /// before it stays expired, and the next pass resumes.
    pub async fn run(&self, now: DateTime<Utc>) -> AppResult<RetentionPass> {
        let policy = ProviderRetention::resolve(self.config.as_ref()).await?;
        self.run_with(&policy, now).await
    }

    /// [`Self::run`] under `policy`.
    ///
    /// # Errors
    ///
    /// As [`Self::run`].
    pub async fn run_with(
        &self,
        policy: &ProviderRetention,
        now: DateTime<Utc>,
    ) -> AppResult<RetentionPass> {
        if !policy.expires_anything() {
            return Ok(RetentionPass::default());
        }
        let activities = self
            .sweep
            .run(policy, now, PAGES_PER_PASS)
            .await
            .map_err(|e| AppError::database(format!("activity retention sweep: {e}")))?
            .expired;
        for (source, expired) in &activities {
            info!(source = %source, expired, "Expired provider activities held past their retention");
        }
        let wellness = self.expire_wellness(policy, now).await?;
        Ok(RetentionPass {
            activities,
            wellness,
        })
    }

    /// The wellness half: each provider with a retention loses the rows last
    /// synced before its cutoff.
    async fn expire_wellness(
        &self,
        policy: &ProviderRetention,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<(String, ProviderWellnessExpiry)>> {
        let mut providers: Vec<&str> = self
            .wellness_providers
            .iter()
            .map(String::as_str)
            .chain(policy.overridden_providers())
            .collect();
        providers.sort_unstable();
        providers.dedup();
        let mut expired = Vec::new();
        for provider in providers {
            // A retention too long to subtract from `now` reaches back before
            // any copy could have been written: nothing of it is expired.
            let Some(cutoff) = policy
                .ttl_for_provider(provider)
                .and_then(|ttl| TimeDelta::from_std(ttl).ok())
                .and_then(|ttl| now.checked_sub_signed(ttl))
            else {
                continue;
            };
            let expiry = self
                .provider_data
                .expire_provider_wellness(provider, cutoff)
                .await?;
            if expiry.scopes > 0 {
                info!(
                    provider,
                    scopes = expiry.scopes,
                    expired = expiry.evicted.total(),
                    rows_expired = ?expiry.evicted.rows_removed,
                    "Expired provider wellness held past its retention"
                );
                expired.push((provider.to_owned(), expiry));
            }
        }
        Ok(expired)
    }
}

/// Start the retention sweep on [`spawn_periodic`], hourly.
pub fn start_retention_sweeper(
    sweeper: Arc<RetentionSweeper>,
    ledger: Arc<dyn WorkerRunRepository>,
) -> AbortHandle {
    let handle = spawn_periodic(WORKER_NAME, SWEEP_INTERVAL, ledger, move || {
        let sweeper = Arc::clone(&sweeper);
        async move { sweeper.run(Utc::now()).await.map(|_| ()) }
    });
    info!("Provider data retention sweep started (hourly, policy from activity_retention.*)");
    handle
}
