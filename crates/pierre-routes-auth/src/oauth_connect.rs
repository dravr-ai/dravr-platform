// ABOUTME: What a provider's OAuth callback starts once its grant is stored: the backfill or the first read, then the report
// ABOUTME: The connection's report (the grant's history walk) waits for both and for the first post either leads to

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The follow-ups of an OAuth connect (carnet#1155, carnet#1482).
//!
//! A grant whose provider pushes its data (Garmin's Developer Program API)
//! has nothing to pull: its data arrives as Garmin pushes it, and a backfill
//! would read the athlete's usr/pwd session instead (carnet#1155). Any other
//! grant is backfilled by the health sync, or read on demand when the sync
//! does not manage it. The connection's report, which starts the grant's
//! history walk, runs once the backfill or the first read, and the first
//! post either leads to, let go of the connection (carnet#1482).

#[cfg(feature = "health-sync")]
use tracing::warn;
#[cfg(feature = "health-sync")]
use uuid::Uuid;

use pierre_providers::backend_resolver;

use crate::first_sync::spawn_on_demand_first_read;
#[cfg(feature = "health-sync")]
use crate::first_sync::{health_sync_backfills, spawn_connection_report};
#[cfg(feature = "health-sync")]
use crate::health_backfill::spawn_health_backfill;
use crate::AuthRoutesContext;

/// Start what the OAuth connect of `provider` for `user_id` owes, now that
/// its grant is stored.
pub(crate) fn spawn_connect_follow_ups(
    resources: &AuthRoutesContext,
    user_id: &str,
    provider: &str,
) {
    let pushed = backend_resolver::pushed_backend_for(provider) == Some(provider);
    #[cfg(feature = "health-sync")]
    let backfill = (!pushed)
        .then(|| spawn_health_backfill(resources, user_id, provider, None))
        .flatten();
    #[cfg(feature = "health-sync")]
    let read_on_demand = !health_sync_backfills(resources, provider);
    // Without the health sync no provider is backfilled after a connect, so
    // every one is read on demand.
    #[cfg(not(feature = "health-sync"))]
    let read_on_demand = true;
    let first_read = (read_on_demand && !pushed)
        .then(|| spawn_on_demand_first_read(resources, user_id, provider))
        .flatten();
    #[cfg(feature = "health-sync")]
    if let Ok(user_id) = Uuid::parse_str(user_id) {
        let backend = provider.to_owned();
        spawn_connection_report(resources, user_id, provider, async move {
            let started = [backfill, first_read].into_iter().flatten();
            for task in started {
                if let Err(e) = task.await {
                    warn!(user_id = %user_id, provider = %backend, error = %e, "a connect follow-up task failed; the connection is still reported");
                }
            }
        });
    }
    #[cfg(not(feature = "health-sync"))]
    drop(first_read);
}
