// ABOUTME: The scheduled health-sync cycle: every connected athlete, every provider, on a jittered poll
// ABOUTME: Scrape-backed providers wait six hours between syncs; stamps last_sync and notifies clients
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The scheduled sync loop the server runs in place of enforme's own
//! scheduler.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use dravr_enforme::models::connection::ConnectedUser;
use pierre_core::models::{OAuthNotification, SmartScheduleWeights, TenantId};
use pierre_database::AuthRepos;
use pierre_providers::backend_resolver::{is_mirror_backend, sync_backend};
use tokio::task::AbortHandle;
use tracing::{info, warn};
use uuid::Uuid;

use super::{
    compute_smart_interval, record_sync_latency, SyncNotifier, SYNC_FAILURES, SYNC_SUCCESSES,
};
use crate::sync_failure_notice::{health_sync_failure_is_told, SyncFailureNotices};

/// Start the scheduled sync loop with post-sync SSE notifications.
///
/// Replaces enforme's built-in scheduler with a Pierre-aware version that:
/// - Iterates all connected users/providers on a jittered interval
/// - Calls `SyncOrchestrator::sync_user` for the actual sync
/// - Updates `user_oauth_tokens.last_sync` after successful syncs
/// - Sends SSE notifications to connected clients
/// - Tells the athlete once when a provider's sync fails, and re-arms that
///   notice when one lands ([`SyncFailureNotices`])
/// - Tracks sync metrics (success/failure counts, latency)
/// - Leaves each request to the signing app's budget, which the credentials carry
///
/// Returns an `AbortHandle` to cancel the background task on shutdown.
pub fn start_scheduled_sync(
    orchestrator: Arc<dravr_enforme::SyncOrchestrator>,
    repos: &AuthRepos,
    sse_manager: Arc<dyn SyncNotifier>,
    notices: SyncFailureNotices,
) -> AbortHandle {
    use dravr_enforme::orchestrator::scheduler::with_jitter;
    use tokio::time::sleep;

    let repos = repos.clone();
    let poll_interval = Duration::from_secs(orchestrator.config().poll_interval_secs);

    let handle = tokio::spawn(async move {
        info!(
            interval_secs = poll_interval.as_secs(),
            "Pierre scheduled sync started (with post-sync notifications)"
        );

        loop {
            let sleep_duration = with_jitter(poll_interval);
            sleep(sleep_duration).await;

            run_scheduled_sync_cycle(&orchestrator, &repos, &sse_manager, &notices).await;
        }
    });

    let abort_handle = handle.abort_handle();
    info!("Scheduled sync task registered");
    abort_handle
}

/// Execute one full sync cycle across all providers and users.
async fn run_scheduled_sync_cycle(
    orchestrator: &Arc<dravr_enforme::SyncOrchestrator>,
    repos: &AuthRepos,
    sse_manager: &Arc<dyn SyncNotifier>,
    notices: &SyncFailureNotices,
) {
    for provider_name in orchestrator.provider_names() {
        let users = match orchestrator
            .deps()
            .connections
            .list_connected_users(provider_name)
            .await
        {
            Ok(users) => users,
            Err(e) => {
                warn!(
                    provider = provider_name,
                    error = %e,
                    "Failed to list connected users for scheduled sync"
                );
                SYNC_FAILURES.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        sync_provider_users(
            orchestrator,
            repos,
            sse_manager,
            notices,
            &users,
            provider_name,
        )
        .await;
    }
}

/// Sync all active users for a single provider.
///
/// Each request a sync makes is admitted against the budget of the app that
/// signs the user's token (the credentials carry its gate), so a spent budget
/// refuses only the users that app signs.
async fn sync_provider_users(
    orchestrator: &Arc<dravr_enforme::SyncOrchestrator>,
    repos: &AuthRepos,
    sse_manager: &Arc<dyn SyncNotifier>,
    notices: &SyncFailureNotices,
    users: &[ConnectedUser],
    provider_name: &str,
) {
    for user in users {
        if !user.is_active || scrape_sync_not_due(repos, user, provider_name).await {
            continue;
        }

        sync_single_user(
            orchestrator,
            repos,
            sse_manager,
            notices,
            user,
            provider_name,
        )
        .await;
    }
}

/// Sync a single user for a given provider, recording metrics and sending notifications.
async fn sync_single_user(
    orchestrator: &Arc<dravr_enforme::SyncOrchestrator>,
    repos: &AuthRepos,
    sse_manager: &Arc<dyn SyncNotifier>,
    notices: &SyncFailureNotices,
    user: &ConnectedUser,
    provider_name: &str,
) {
    log_smart_schedule_interval(user, provider_name);

    let start = Instant::now();
    match orchestrator.sync_user(&user.user_id, provider_name).await {
        Ok(result) => {
            let elapsed = start.elapsed();
            SYNC_SUCCESSES.fetch_add(1, Ordering::Relaxed);
            record_sync_latency(elapsed);

            info!(
                user_id = user.user_id,
                provider = provider_name,
                records_created = result.records_created,
                elapsed_ms = elapsed.as_millis() as u64,
                "Scheduled sync completed"
            );

            if let Ok(user_uuid) = user.user_id.parse::<Uuid>() {
                record_landed_sync(repos, notices, user_uuid, provider_name).await;
                send_sync_notification(
                    sse_manager,
                    user_uuid,
                    &user.user_id,
                    provider_name,
                    result.records_created,
                )
                .await;
            }
        }
        Err(e) => {
            SYNC_FAILURES.fetch_add(1, Ordering::Relaxed);
            warn!(
                user_id = user.user_id,
                provider = provider_name,
                error = %e,
                "Scheduled sync failed"
            );
            if health_sync_failure_is_told(&e) {
                report_failed_sync(repos, notices, user, provider_name).await;
            }
        }
    }
}

/// Stamp `last_sync` on the token row a landed sync read and re-arm its
/// sync-failure notice.
async fn record_landed_sync(
    repos: &AuthRepos,
    notices: &SyncFailureNotices,
    user_uuid: Uuid,
    provider_name: &str,
) {
    let backend = sync_backend(provider_name);
    if let Some(tenant) = token_tenant(repos, user_uuid, &backend).await {
        let _ = repos
            .oauth_tokens
            .update_provider_last_sync(user_uuid, tenant, &backend, Utc::now())
            .await;
        notices.sync_landed(user_uuid, tenant, &backend).await;
    }
}

/// Tell the athlete, once, that the scheduled sync of `provider_name` failed.
async fn report_failed_sync(
    repos: &AuthRepos,
    notices: &SyncFailureNotices,
    user: &ConnectedUser,
    provider_name: &str,
) {
    let Ok(user_uuid) = user.user_id.parse::<Uuid>() else {
        return;
    };
    let backend = sync_backend(provider_name);
    if let Some(tenant) = token_tenant(repos, user_uuid, &backend).await {
        notices.sync_failed(user_uuid, tenant, &backend).await;
    }
}

/// Log the recommended smart-schedule interval for a user.
///
/// Activity count is not yet available from the database; uses 0 as baseline.
/// When a recent-activity-count query is added to the repository layer,
/// wire it here to enable true per-user adaptive scheduling.
fn log_smart_schedule_interval(user: &ConnectedUser, provider_name: &str) {
    let weights = SmartScheduleWeights::default();
    let activity_count: u32 = 0;
    let recommended_interval = compute_smart_interval(activity_count, &weights);
    info!(
        user_id = user.user_id,
        provider = provider_name,
        activity_count_7d = activity_count,
        recommended_interval_secs = recommended_interval.as_secs(),
        "Smart schedule: computed per-user refresh interval"
    );
}

/// The tenant of the athlete's `backend` token row — the row a sync of it
/// reads, whose `last_sync` it stamps and whose connection it reports on.
pub async fn token_tenant(repos: &AuthRepos, user_uuid: Uuid, backend: &str) -> Option<TenantId> {
    let tokens = repos.oauth_tokens.get_tokens(user_uuid, None).await.ok()?;
    tokens
        .iter()
        .find(|t| t.provider == backend)
        .and_then(|t| TenantId::parse_str(&t.tenant_id).ok())
}

/// Least time between two scheduled syncs of a scrape-backed provider for one
/// athlete.
///
/// A scrape-backed sync loads one browser page per day it reads, on the
/// dedicated scraper service. What it reads — the night's sleep, resting heart
/// rate, overnight HRV, body metrics — changes once a day, so the 15-minute
/// poll that suits an API provider would load several hundred pages per
/// athlete per day across the server's instances for nothing new, and would
/// show the provider an automated login every few minutes. Four reads a day
/// keep a morning's night current by midday; connecting, and an athlete's own
/// refresh request, sync at once.
pub const SCRAPE_SYNC_MIN_INTERVAL: Duration = Duration::from_hours(6);

/// Whether the scheduled cycle should skip `provider_name` for this athlete.
///
/// True when its sync is scrape-backed and the token row it reads was synced
/// less than [`SCRAPE_SYNC_MIN_INTERVAL`] ago. The stamp is shared by every
/// server instance, so the instances' cycles do not each scrape the athlete.
pub async fn scrape_sync_not_due(
    repos: &AuthRepos,
    user: &ConnectedUser,
    provider_name: &str,
) -> bool {
    let backend = sync_backend(provider_name);
    if !is_mirror_backend(&backend) {
        return false;
    }
    let Ok(user_uuid) = user.user_id.parse::<Uuid>() else {
        return false;
    };
    let Some(tenant) = token_tenant(repos, user_uuid, &backend).await else {
        return false;
    };
    let Ok(Some(last_sync)) = repos
        .oauth_tokens
        .get_provider_last_sync(user_uuid, tenant, &backend)
        .await
    else {
        return false;
    };
    Utc::now()
        .signed_duration_since(last_sync)
        .to_std()
        .is_ok_and(|age| age < SCRAPE_SYNC_MIN_INTERVAL)
}

/// Send an SSE notification after a successful sync with new records.
async fn send_sync_notification(
    sse_manager: &Arc<dyn SyncNotifier>,
    user_uuid: Uuid,
    user_id_str: &str,
    provider_name: &str,
    records_created: u32,
) {
    if records_created > 0 {
        let notification = OAuthNotification {
            id: Uuid::new_v4().to_string(),
            user_id: user_id_str.to_owned(),
            provider: provider_name.to_owned(),
            success: true,
            message: format!("Synced {records_created} new records from {provider_name}"),
            expires_at: None,
            created_at: Utc::now(),
            read_at: None,
        };
        let _ = sse_manager
            .send_notification(user_uuid, &notification)
            .await;
    }
}
