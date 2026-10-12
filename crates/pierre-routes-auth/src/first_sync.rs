// ABOUTME: The seam a provider connect reports to once the athlete's first data has landed
// ABOUTME: Installed post-Arc by the composition root, whose implementation has the coach speak first

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a provider connect reports once the athlete's data lands (carnet#831).
//!
//! A connect lands the athlete's first data one of two ways. A provider the
//! health sync manages is backfilled right after the connect, and a scrape
//! connect pre-fetches its activity list; either lands data this crate sees
//! arrive. A provider read on demand — Strava and Wahoo over OAuth — has
//! neither, and its first data is the first read of its activity list.
//!
//! The routes only report which of the two happened. What the athlete is told
//! is the hook's business, and telling them runs a chat-pipeline turn this
//! crate cannot reach, so the hook is a trait the composition root installs
//! once its own `Arc` exists.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use pierre_core::models::TenantId;
#[cfg(feature = "health-sync")]
use pierre_providers::backend_resolver;
#[cfg(feature = "health-sync")]
use pierre_providers::connect_prefetch::PrefetchWait;
#[cfg(feature = "health-sync")]
use pierre_services::provider_refresh::token_tenant;
#[cfg(feature = "health-sync")]
use pierre_services::sync_failure_notice::health_sync_failure_is_told;
use uuid::Uuid;

use crate::AuthRoutesContext;

/// Told when a provider connect has landed, or can land, the athlete's data.
#[async_trait]
pub trait FirstSyncHook: Send + Sync {
    /// The backfill or pre-fetch a connect started landed `provider`'s data
    /// for the athlete, whose token lives in `tenant_id`.
    async fn data_landed(&self, user_id: Uuid, tenant_id: TenantId, provider: &str);

    /// The athlete connected `provider` over OAuth, which the platform reads
    /// on demand rather than backfilling, so nothing has landed yet: the
    /// implementation reads the provider's activity list now and treats what
    /// it lands as the first data.
    async fn connected_on_demand(&self, user_id: Uuid, provider: &str);
}

/// The slot the composition root installs the hook into once its `Arc`
/// exists. Empty until then, and in a build or test that installs none, where
/// a connect reports to nobody.
pub type FirstSyncSlot = Arc<OnceLock<Arc<dyn FirstSyncHook>>>;

/// Whether the health sync backfills `provider` after a connect.
#[cfg(feature = "health-sync")]
pub fn health_sync_backfills(resources: &AuthRoutesContext, provider: &str) -> bool {
    resources
        .sync_orchestrator
        .as_ref()
        .is_some_and(|orchestrator| orchestrator.provider_names().contains(&provider))
}

/// What a connect's finished backfill tells the athlete, from the records it
/// landed or the error it ended on.
#[cfg(feature = "health-sync")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillVerdict {
    /// It landed records: the failure notices re-arm, and the athlete's data
    /// is in.
    Landed,
    /// It succeeded with nothing to land: the failure notices re-arm, and
    /// there is nothing for the coach to speak about.
    LandedNothing,
    /// It failed in a way the athlete is told about.
    Failed,
    /// It failed in a way the athlete is not told about — a dead credential
    /// is the reconnect notice's, a rate limit the shared quota's.
    FailedUntold,
}

/// Classify a finished connect backfill by the records it landed (created or
/// updated) or the error it ended on.
#[cfg(feature = "health-sync")]
pub fn backfill_verdict(landed: Result<u32, &dravr_enforme::EnformeError>) -> BackfillVerdict {
    match landed {
        Ok(0) => BackfillVerdict::LandedNothing,
        Ok(_) => BackfillVerdict::Landed,
        Err(e) if health_sync_failure_is_told(e) => BackfillVerdict::Failed,
        Err(_) => BackfillVerdict::FailedUntold,
    }
}

/// Report a scrape pre-fetch that lands activities as the athlete's data
/// landing, for a provider the health sync does not backfill — the Strava
/// mirror, whose pre-fetch is the only read a connect makes.
#[cfg(feature = "health-sync")]
pub fn spawn_prefetch_landing(
    resources: &AuthRoutesContext,
    user_id: &str,
    provider: &str,
    prefetch: PrefetchWait,
) {
    let first_sync = Arc::clone(&resources.first_sync);
    if first_sync.get().is_none() {
        return;
    }
    let Ok(user_uuid) = Uuid::parse_str(user_id) else {
        return;
    };
    let provider = provider.to_owned();
    let auth_repos = resources.repos.auth_repos();
    tokio::spawn(async move {
        let landed = prefetch
            .finished()
            .await
            .is_some_and(|window| !window.is_empty());
        if !landed {
            return;
        }
        // The session the pre-fetch read through is the mirror's, which a
        // scrape-only athlete holds without any OAuth token: the first backend
        // serving the provider that holds a token names the tenant.
        for backend in backend_resolver::serving_backends(&provider) {
            if let Some(tenant) = token_tenant(&auth_repos, user_uuid, &backend).await {
                if let Some(hook) = first_sync.get() {
                    hook.data_landed(user_uuid, tenant, &provider).await;
                }
                return;
            }
        }
    });
}

/// Report an OAuth connect of a provider the health sync does not backfill —
/// Strava and Wahoo are read on demand — so its first read can be made now
/// rather than whenever the athlete next asks.
pub fn spawn_on_demand_first_read(resources: &AuthRoutesContext, user_id: &str, provider: &str) {
    let first_sync = Arc::clone(&resources.first_sync);
    if first_sync.get().is_none() {
        return;
    }
    let Ok(user_uuid) = Uuid::parse_str(user_id) else {
        return;
    };
    let provider = provider.to_owned();
    tokio::spawn(async move {
        if let Some(hook) = first_sync.get() {
            hook.connected_on_demand(user_uuid, &provider).await;
        }
    });
}

#[cfg(all(test, feature = "health-sync"))]
mod tests {
    use super::*;
    use dravr_enforme::EnformeError;

    /// Only a backfill that landed records reports the athlete's data landing,
    /// and with it the coach's first post (carnet#831).
    #[test]
    fn a_backfill_that_landed_records_reports_a_landing() {
        assert_eq!(backfill_verdict(Ok(1)), BackfillVerdict::Landed);
        assert_eq!(backfill_verdict(Ok(214)), BackfillVerdict::Landed);
    }

    /// A backfill that succeeded with nothing to land re-arms the failure
    /// notices, but leaves the coach nothing to speak about.
    #[test]
    fn a_backfill_that_landed_nothing_reports_no_landing() {
        assert_eq!(backfill_verdict(Ok(0)), BackfillVerdict::LandedNothing);
    }

    /// A failed backfill is never a landing: a failure the athlete can act on
    /// is told, and one they cannot is neither told nor re-armed.
    #[test]
    fn a_failed_backfill_never_reports_a_landing() {
        let outage = EnformeError::ProviderError {
            provider: "whoop".to_owned(),
            message: "HTTP 503".to_owned(),
        };
        assert_eq!(backfill_verdict(Err(&outage)), BackfillVerdict::Failed);

        let expired = EnformeError::CredentialsExpired {
            user_id: Uuid::new_v4().to_string(),
            provider: "whoop".to_owned(),
        };
        assert_eq!(
            backfill_verdict(Err(&expired)),
            BackfillVerdict::FailedUntold
        );

        let throttled = EnformeError::RateLimited {
            provider: "whoop".to_owned(),
            retry_after_secs: 60,
        };
        assert_eq!(
            backfill_verdict(Err(&throttled)),
            BackfillVerdict::FailedUntold
        );
    }
}
