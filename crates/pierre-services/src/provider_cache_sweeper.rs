// ABOUTME: Cache TTL sweep — evicts every copy of a provider's data held past the cap its terms declare
// ABOUTME: Hourly on the worker ledger; each evicted scope also loses its sync state, so the next read fetches again

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider cache TTL enforcement (carnet#725).
//!
//! A provider's terms can cap how long a copy of its data may be held: Nolio
//! allows a transient copy, any cache included, seven days (API terms §7.1).
//! The cap is declared on the provider's descriptor
//! ([`ProviderDescriptor::cache_ttl`]) and collected by
//! [`ProviderRegistry::cache_ttls`]; this sweep enforces it.
//!
//! Each pass evicts, for every capped provider, the copies written before
//! `now - ttl` through
//! [`ProviderDataRepository::expire_provider_cache`]. A copy re-synced inside
//! the window carries its new sync time and stays. Each scope that lost a copy
//! also loses the provider's sync state, so the next read or sync fetches the
//! evicted data again instead of trusting a coverage mark over an emptied
//! cache, and each writes an attestation row (`cache_expired`).
//!
//! The sweep is a backstop, not the only bound: a provider's own data is
//! re-copied by every sync, and a disconnect deletes it at once.
//!
//! [`ProviderDescriptor::cache_ttl`]: pierre_providers::spi::ProviderDescriptor::cache_ttl
//! [`ProviderRegistry::cache_ttls`]: pierre_providers::registry::ProviderRegistry::cache_ttls

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use pierre_core::errors::AppResult;
use pierre_database::repositories::{
    ProviderCacheExpiry, ProviderDataRepository, WorkerRunRepository,
};
use tracing::info;

use crate::periodic::spawn_periodic;

/// The ledger name the sweep runs under.
const WORKER_NAME: &str = "provider cache ttl sweeper";

/// Sweep cadence. The shortest declared cap is seven days, so an hourly pass
/// holds a copy at most an hour past its cap.
const SWEEP_INTERVAL: Duration = Duration::from_hours(1);

/// Evict every copy of each capped provider's data older than its cap at
/// `now`, provider by provider, in the order given.
///
/// Returns what each provider lost, a provider with nothing expired included.
///
/// # Errors
/// Returns the first database error; providers swept before it stay swept.
pub async fn sweep_provider_caches(
    provider_data: &dyn ProviderDataRepository,
    ttls: &[(&str, Duration)],
    now: DateTime<Utc>,
) -> AppResult<Vec<(String, ProviderCacheExpiry)>> {
    let mut swept = Vec::with_capacity(ttls.len());
    for (provider, ttl) in ttls {
        // A cap too long to subtract from `now` reaches back before any copy
        // could have been written, so nothing of that provider is expired.
        let Some(cutoff) = TimeDelta::from_std(*ttl)
            .ok()
            .and_then(|ttl| now.checked_sub_signed(ttl))
        else {
            swept.push(((*provider).to_owned(), ProviderCacheExpiry::default()));
            continue;
        };
        let expiry = provider_data
            .expire_provider_cache(provider, cutoff)
            .await?;
        if expiry.scopes > 0 {
            info!(
                provider = %provider,
                ttl_secs = ttl.as_secs(),
                scopes = expiry.scopes,
                evicted = expiry.evicted.total(),
                rows_evicted = ?expiry.evicted.rows_removed,
                "Evicted provider data held past its cache TTL"
            );
        }
        swept.push(((*provider).to_owned(), expiry));
    }
    Ok(swept)
}

/// Start the cache TTL sweep on [`spawn_periodic`] for the providers in
/// `ttls`, the registry's declared caps.
///
/// A build in which no registered provider declares a cap has nothing to
/// enforce, and the sweep is not started.
pub fn start_provider_cache_sweeper(
    provider_data: Arc<dyn ProviderDataRepository>,
    ttls: Vec<(&'static str, Duration)>,
    ledger: Arc<dyn WorkerRunRepository>,
) {
    if ttls.is_empty() {
        info!("No registered provider declares a cache TTL; the provider cache sweeper is not started");
        return;
    }
    let ttls: Arc<[(&'static str, Duration)]> = ttls.into();
    spawn_periodic(WORKER_NAME, SWEEP_INTERVAL, ledger, move || {
        let provider_data = Arc::clone(&provider_data);
        let ttls = Arc::clone(&ttls);
        async move {
            sweep_provider_caches(provider_data.as_ref(), &ttls, Utc::now())
                .await
                .map(|_| ())
        }
    });
}
