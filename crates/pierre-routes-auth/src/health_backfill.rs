// ABOUTME: The health backfill a provider connect starts, and how its landing or failure is reported
// ABOUTME: Shared by the OAuth callback, the pasted-key connect and the scrape login
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_providers::backend_resolver;
use pierre_providers::connect_prefetch::PrefetchWait;
use pierre_services::health_sync::wellness_pull_stands_down;
use pierre_services::provider_refresh::token_tenant;
use tracing::info;
use uuid::Uuid;

use crate::first_sync::{
    backfill_verdict, health_sync_backfills, spawn_prefetch_landing, BackfillVerdict,
};
use crate::AuthRoutesContext;

/// Spawn background health data backfill after a provider connects.
///
/// Runs the sync orchestrator's incremental sync, whose reach is the
/// provider's own recent window, so the user gets recent data immediately
/// after connecting a wearable provider, whether through OAuth, a pasted API
/// key (intervals.icu) or a scrape login (Garmin, COROS). `provider` is the
/// name the sync knows it under; a provider the sync does not manage
/// (Strava's activities, read on demand) has nothing to backfill. A backfill
/// that fails tells the athlete once, through the sync-failure notices; one
/// that lands re-arms them.
///
/// `after` is the activity pre-fetch a scrape connect started on the same
/// session: the backfill's first read waits for it, because each read is a
/// browser on the scraper service and a connect that fires both at once sheds
/// its own reads (carnet#736). An API connect has none to wait on.
///
/// A backfill that lands records is reported to the [`crate::FirstSyncHook`] as the
/// athlete's data landing (carnet#831); whether it is their first is the
/// hook's to decide. A provider the sync does not manage lands its data
/// through the scrape pre-fetch instead, when the connect started one, and
/// that is reported the same way.
pub(crate) fn spawn_health_backfill(
    resources: &AuthRoutesContext,
    user_id: &str,
    provider: &str,
    after: Option<PrefetchWait>,
) {
    if !health_sync_backfills(resources, provider) {
        tracing::debug!(
            provider,
            "No health sync for this provider; nothing to backfill"
        );
        if let Some(prefetch) = after {
            spawn_prefetch_landing(resources, user_id, provider, prefetch);
        }
        return;
    }
    let Some(orchestrator) = resources.sync_orchestrator.clone() else {
        return;
    };
    let user_id = user_id.to_owned();
    let provider = provider.to_owned();
    let auth_repos = resources.repos.auth_repos();
    let repos = Arc::clone(&resources.repos);
    let notices = resources.sync_failure_notices.clone();
    let first_sync = Arc::clone(&resources.first_sync);
    tokio::spawn(async move {
        if let Some(prefetch) = after {
            // What the pre-fetch read is the activity readers'; the backfill
            // only needs the session to itself.
            drop(prefetch.finished().await);
        }
        // Wellness the provider pushes has nothing to pull (carnet#1257).
        if wellness_pull_stands_down(&repos, &user_id, &provider).await {
            return;
        }
        info!(
            user_id = %user_id,
            provider = %provider,
            "Triggering initial health data backfill after OAuth connection"
        );
        let outcome = orchestrator.sync_user(&user_id, &provider).await;
        if let Err(e) = &outcome {
            // `backend` is the connection the sync read: the Garmin grant's
            // backfill reads the username-and-password session.
            tracing::error!(
                error = %e,
                user_id = %user_id,
                provider = %provider,
                backend = %backend_resolver::sync_backend(&provider),
                "Initial health data backfill failed"
            );
        }
        let verdict = backfill_verdict(outcome.as_ref().map(|landed| {
            landed
                .records_created
                .saturating_add(landed.records_updated)
        }));
        // A failure the athlete is not told about (a dead credential, the
        // shared quota) neither notifies nor re-arms.
        if verdict == BackfillVerdict::FailedUntold {
            return;
        }
        let Ok(user_uuid) = Uuid::parse_str(&user_id) else {
            return;
        };
        let backend = backend_resolver::sync_backend(&provider);
        let Some(tenant) = token_tenant(&auth_repos, user_uuid, &backend).await else {
            return;
        };
        if verdict == BackfillVerdict::Failed {
            notices.sync_failed(user_uuid, tenant, &backend).await;
            return;
        }
        notices.sync_landed(user_uuid, tenant, &backend).await;
        if verdict == BackfillVerdict::Landed {
            if let Some(hook) = first_sync.get() {
                hook.data_landed(user_uuid, tenant, &provider).await;
            }
        }
    });
}
