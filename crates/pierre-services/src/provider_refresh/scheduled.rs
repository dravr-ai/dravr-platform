// ABOUTME: The scheduled health-sync cycle: every connected athlete, every provider, on a jittered poll
// ABOUTME: Scrape-backed providers wait six hours; coach-linked wellness syncs per link every three hours
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The scheduled sync loop the server runs in place of enforme's own
//! scheduler.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use dravr_enforme::error::EnformeResult;
use dravr_enforme::models::connection::ConnectedUser;
use dravr_enforme::SyncOrchestrator;
use dravr_equilibre_sync::{SyncResult, SyncStatus};
use pierre_core::models::{DelegatedConnection, OAuthNotification, SmartScheduleWeights, TenantId};
use pierre_database::repositories::WorkerRunRepository;
use pierre_database::AuthRepos;
use pierre_providers::backend_resolver::{is_mirror_backend, sync_backend};
use pierre_providers::registry::global_registry;
use pierre_providers::request_budget::in_background;
use tokio::task::AbortHandle;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::{
    compute_smart_interval, record_sync_latency, SyncNotifier, SYNC_FAILURES, SYNC_SUCCESSES,
};
use crate::coach_platform::coach_platforms;
use crate::health_sync::{delegated_sync_scope, PierreSyncStorage};
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
/// - Then syncs the wellness of members read through a coach's link
///   ([`sync_delegated_members`]) from `storage`, the store the orchestrator
///   writes through: shared with the orchestrator and the server's context,
///   and moved into the loop's task
///
/// Returns an `AbortHandle` to cancel the background task on shutdown.
pub fn start_scheduled_sync(
    orchestrator: Arc<SyncOrchestrator>,
    storage: Arc<PierreSyncStorage>,
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

            run_scheduled_sync_cycle(
                &orchestrator,
                &storage,
                &repos,
                &sse_manager,
                &notices,
                SCHEDULED_SYNC_DEADLINE,
            )
            .await;
        }
    });

    let abort_handle = handle.abort_handle();
    info!("Scheduled sync task registered");
    abort_handle
}

/// Longest one athlete's scheduled sync of one provider may run.
///
/// The cycle syncs athletes one after another, so a sync that never returns —
/// a vendor that accepts the connection and never answers — would stop
/// background sync for every later athlete and provider until the process
/// restarted (carnet#995). dravr-enforme bounds each request (30 s); a sync
/// is a handful of paged requests per data type, so ten minutes is far past
/// any sync that is still making progress. A member's sync through their
/// coach's link runs in the same cycle and is held to the same bound.
const SCHEDULED_SYNC_DEADLINE: Duration = Duration::from_mins(10);

/// Execute one full sync cycle across all providers and users: every
/// athlete's own connections, then the wellness read through coach links,
/// each sync of one provider given at most `deadline`.
async fn run_scheduled_sync_cycle(
    orchestrator: &Arc<SyncOrchestrator>,
    storage: &PierreSyncStorage,
    repos: &AuthRepos,
    sse_manager: &Arc<dyn SyncNotifier>,
    notices: &SyncFailureNotices,
    deadline: Duration,
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
            deadline,
        )
        .await;
    }

    let now_ms = Utc::now().timestamp_millis();
    for provider in delegated_wellness_providers(orchestrator) {
        sync_delegated_members(storage, orchestrator, provider, now_ms, deadline).await;
    }
}

/// The providers the orchestrator syncs whose coaching platform's coach
/// credential reads a linked member's wellness (Intervals.icu): the ones the
/// delegated pass runs for.
fn delegated_wellness_providers(orchestrator: &SyncOrchestrator) -> Vec<&'static str> {
    let synced = orchestrator.provider_names();
    coach_platforms(&global_registry())
        .iter()
        .filter(|platform| platform.reads_member_wellness())
        .map(|platform| platform.user_facing())
        .filter(|provider| synced.contains(provider))
        .collect()
}

/// How long one link's wellness waits between two syncs through its coach's
/// key, whichever instance ran the last.
///
/// One sync spends three requests of the coach's key, which every athlete the
/// coach reads shares, and wellness changes about once a day: eight syncs a
/// day keep a morning's night current by mid-morning for 24 requests a member,
/// where the 15-minute poll would spend 288 a member on each instance.
const DELEGATED_WELLNESS_PERIOD: Duration = Duration::from_hours(3);

/// How long one instance holds a link's sync before another may take it over.
const DELEGATED_WELLNESS_LEASE: Duration = Duration::from_mins(15);

/// Sync the `provider` wellness of every member read through a coach's
/// confirmed link that is due at `now_ms` (Unix milliseconds), once across
/// the server's instances.
///
/// Each link whose member granted it `wellness_read` and that Dravr's own
/// rows do not exclude (`PierreSyncStorage::delegated_wellness_links`) is
/// claimed in the worker ledger as `delegated wellness {link_id}`: due three
/// hours after its last landed sync (`DELEGATED_WELLNESS_PERIOD`), and held
/// for fifteen minutes while it runs. A link that is not due, or that another
/// instance holds, spends nothing. The sync runs inside the delegated sync
/// scope (`crate::health_sync::delegated_sync_scope`), the only place its
/// credentials are handed out, and inside the background share of the coach
/// key's budget ([`in_background`]); the credentials decide the rest
/// (`crate::delegated_wellness`). A sync that landed finishes the claim; one
/// that failed, or found no credentials, holds the link a full period, so a
/// member the platform refuses is not asked for again on every poll. No
/// `last_sync` is stamped and no failure notice is sent: the member holds no
/// connection of their own to report on.
///
/// Each link's sync gets at most `deadline`, as an athlete's own does in the
/// cycle: one still running then is dropped and the link held a full period,
/// so a read that never answers cannot stop the pass or the cycle after it
/// (carnet#995).
///
/// Public so an integration test can drive one pass at a chosen time:
/// pierre-services builds without enforme's Intervals.icu provider, so the
/// pass cannot be exercised by an inline test.
pub async fn sync_delegated_members(
    storage: &PierreSyncStorage,
    orchestrator: &SyncOrchestrator,
    provider: &str,
    now_ms: i64,
    deadline: Duration,
) {
    let links = match storage.delegated_wellness_links(provider).await {
        Ok(links) => links,
        Err(e) => {
            warn!(provider, error = %e, "Failed to list the links for delegated wellness");
            return;
        }
    };
    for link in &links {
        sync_delegated_member(storage, orchestrator, provider, link, now_ms, deadline).await;
    }
}

/// One link's turn in the delegated pass: claimed when due, synced within
/// `deadline`, then finished when the sync landed or held a full period when
/// it did not.
async fn sync_delegated_member(
    storage: &PierreSyncStorage,
    orchestrator: &SyncOrchestrator,
    provider: &str,
    link: &DelegatedConnection,
    now_ms: i64,
    deadline: Duration,
) {
    let ledger = storage.worker_runs();
    let name = format!("delegated wellness {}", link.id);
    let Some(lease) = claim_delegated_sync(ledger, link, &name, now_ms).await else {
        return;
    };
    let member = link.member_user_id;
    let member_id = member.to_string();
    let sync = delegated_sync_scope(
        member,
        in_background(orchestrator.sync_user(&member_id, provider)),
    );
    let landed = timeout(deadline, sync).await.map_or_else(
        |_| {
            warn!(
                link_id = %link.id,
                group_id = %link.group_id,
                deadline_secs = deadline.as_secs(),
                "Delegated wellness sync did not finish by its deadline; the link is held a full period"
            );
            false
        },
        |synced| delegated_sync_landed(link, &synced),
    );
    let recorded = if landed {
        ledger.finish_worker_run(&name, now_ms, Some(&lease)).await
    } else {
        ledger
            .defer_worker_run(
                &name,
                now_ms + millis(DELEGATED_WELLNESS_PERIOD),
                Some(&lease),
            )
            .await
    };
    if let Err(e) = recorded {
        warn!(link_id = %link.id, error = %e, "Could not record a delegated wellness sync");
    }
}

/// Whether `link`'s delegated sync landed, logging what it did.
fn delegated_sync_landed(link: &DelegatedConnection, synced: &EnformeResult<SyncResult>) -> bool {
    match synced {
        Ok(result) if result.status == SyncStatus::Failed => {
            // The platform refused or failed a wellness read the credentials
            // were handed for: a coach key it no longer honours ends up here,
            // and is reported by this line alone (`crate::delegated_wellness`).
            warn!(
                link_id = %link.id,
                group_id = %link.group_id,
                data_types_failed = result.records_errored,
                "Delegated wellness sync failed; the link is held a full period"
            );
            false
        }
        Ok(result) => {
            info!(
                link_id = %link.id,
                group_id = %link.group_id,
                records_created = result.records_created,
                "Delegated wellness pass synced a link"
            );
            true
        }
        // No credentials, whose reason the decision logged, or a store that
        // could not be read before any wellness read.
        Err(e) => {
            info!(
                link_id = %link.id,
                group_id = %link.group_id,
                error = %e,
                "Delegated wellness pass synced nothing for a link; it is held a full period"
            );
            false
        }
    }
}

/// Take `link`'s sync, ledgered as `name`, when it is due at `now_ms` and no
/// instance holds it, returning the claim's token; a ledger that cannot be
/// read takes nothing (logged).
async fn claim_delegated_sync(
    ledger: &dyn WorkerRunRepository,
    link: &DelegatedConnection,
    name: &str,
    now_ms: i64,
) -> Option<String> {
    ledger
        .claim_worker_run(
            name,
            millis(DELEGATED_WELLNESS_PERIOD),
            now_ms,
            millis(DELEGATED_WELLNESS_LEASE),
        )
        .await
        .unwrap_or_else(|e| {
            warn!(link_id = %link.id, error = %e, "Could not claim a delegated wellness sync");
            None
        })
}

/// `duration` in whole milliseconds, as the worker ledger counts time.
fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// Sync all active users for a single provider.
///
/// Each request a sync makes is admitted against the request budget of the
/// user's token (the credentials carry its gate), so a spent window refuses
/// only the users counted in it.
async fn sync_provider_users(
    orchestrator: &Arc<SyncOrchestrator>,
    repos: &AuthRepos,
    sse_manager: &Arc<dyn SyncNotifier>,
    notices: &SyncFailureNotices,
    users: &[ConnectedUser],
    provider_name: &str,
    deadline: Duration,
) {
    for user in users {
        if !user.is_active || scrape_sync_not_due(repos, user, provider_name).await {
            continue;
        }

        sync_single_user_within(
            orchestrator,
            repos,
            sse_manager,
            notices,
            user,
            provider_name,
            deadline,
        )
        .await;
    }
}

/// Sync a single user for a given provider within `deadline`. A sync still
/// running then is dropped and counted as a failure, and the cycle moves on
/// to the next athlete.
async fn sync_single_user_within(
    orchestrator: &Arc<SyncOrchestrator>,
    repos: &AuthRepos,
    sse_manager: &Arc<dyn SyncNotifier>,
    notices: &SyncFailureNotices,
    user: &ConnectedUser,
    provider_name: &str,
    deadline: Duration,
) {
    let sync = sync_single_user(
        orchestrator,
        repos,
        sse_manager,
        notices,
        user,
        provider_name,
    );
    if timeout(deadline, sync).await.is_err() {
        SYNC_FAILURES.fetch_add(1, Ordering::Relaxed);
        warn!(
            user_id = user.user_id,
            provider = provider_name,
            deadline_secs = deadline.as_secs(),
            "Scheduled sync did not finish by its deadline; moving on to the next athlete"
        );
    }
}

/// Sync a single user for a given provider, recording metrics and sending notifications.
async fn sync_single_user(
    orchestrator: &Arc<SyncOrchestrator>,
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
        if let Err(e) = repos
            .oauth_tokens
            .update_provider_last_sync(user_uuid, tenant, &backend, Utc::now())
            .await
        {
            warn!(%user_uuid, provider = %backend, error = %e, "could not stamp last_sync after a landed sync");
        }
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
        if let Err(e) = sse_manager
            .send_notification(user_uuid, &notification)
            .await
        {
            debug!(%user_uuid, error = %e, "sync result notification not delivered over SSE");
        }
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;
    use std::collections::HashMap;
    use std::future::pending;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use dravr_enforme::error::EnformeResult;
    use dravr_enforme::models::connection::ProviderCredentials;
    use dravr_enforme::models::cursor::{SyncBatch, SyncCursor};
    use dravr_enforme::models::webhook::{WebhookConfig, WebhookEvent};
    use dravr_enforme::traits::sync_provider::{DataType, SyncProvider};
    use dravr_enforme::{SyncConfig, SyncDeps, SyncOrchestrator};
    use dravr_equilibre_sync::ContinuousMetricBatch;
    use http::HeaderMap;
    use pierre_core::errors::AppError;
    use pierre_core::models::{
        StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession, Tenant, UserOAuthToken,
    };
    use pierre_database::RepositoryRegistry;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;

    use super::*;
    use crate::health_sync::PierreSyncStorage;

    /// A WHOOP stand-in whose sleep read never answers for one athlete — a
    /// vendor that accepts the connection and never replies — and answers
    /// every other athlete with nothing new. It records whose sleep it read,
    /// in a list the test keeps a handle to (`Arc`) while the orchestrator
    /// owns the provider.
    struct StallsFor {
        stalled: String,
        read: Arc<Mutex<Vec<String>>>,
    }

    fn empty<T>(creds: &ProviderCredentials, data: &str) -> SyncBatch<T> {
        SyncBatch::empty(SyncCursor::new(creds.user_id.clone(), "whoop", data))
    }

    #[async_trait]
    impl SyncProvider for StallsFor {
        fn name(&self) -> &'static str {
            "whoop"
        }

        fn supported_data_types(&self) -> &[DataType] {
            &[DataType::Sleep]
        }

        async fn fetch_sleep(
            &self,
            creds: &ProviderCredentials,
            _cursor: Option<&SyncCursor>,
        ) -> EnformeResult<SyncBatch<StoredSleepSession>> {
            self.read.lock().unwrap().push(creds.user_id.clone());
            if creds.user_id == self.stalled {
                pending::<()>().await;
            }
            Ok(empty(creds, "sleep"))
        }

        async fn fetch_recovery(
            &self,
            creds: &ProviderCredentials,
            _cursor: Option<&SyncCursor>,
        ) -> EnformeResult<SyncBatch<StoredRecoveryMetrics>> {
            Ok(empty(creds, "recovery"))
        }

        async fn fetch_health(
            &self,
            creds: &ProviderCredentials,
            _cursor: Option<&SyncCursor>,
        ) -> EnformeResult<SyncBatch<StoredHealthMetrics>> {
            Ok(empty(creds, "health"))
        }

        async fn fetch_continuous(
            &self,
            creds: &ProviderCredentials,
            _cursor: Option<&SyncCursor>,
        ) -> EnformeResult<SyncBatch<ContinuousMetricBatch>> {
            Ok(empty(creds, "continuous"))
        }

        async fn on_connected(
            &self,
            _creds: &ProviderCredentials,
            _webhook_url: &str,
        ) -> EnformeResult<()> {
            Ok(())
        }

        async fn on_disconnected(&self, _creds: &ProviderCredentials) -> EnformeResult<()> {
            Ok(())
        }

        fn webhook_config(&self) -> Option<WebhookConfig> {
            None
        }

        async fn validate_webhook(&self, _headers: &HeaderMap, _body: &[u8]) -> EnformeResult<()> {
            Ok(())
        }

        async fn parse_webhook(&self, _body: &[u8]) -> EnformeResult<Vec<WebhookEvent>> {
            Ok(Vec::new())
        }
    }

    /// Clients with no stream: the cycle's notification is best-effort.
    struct NoStreams;

    #[async_trait]
    impl SyncNotifier for NoStreams {
        async fn send_notification(
            &self,
            _user_id: Uuid,
            _notification: &OAuthNotification,
        ) -> Result<(), AppError> {
            Ok(())
        }
    }

    /// Seed an athlete with a WHOOP token in their own tenant.
    async fn whoop_athlete(repos: &Arc<RepositoryRegistry>) -> (Uuid, TenantId) {
        let user = create_test_user(&format!("{}@example.com", Uuid::new_v4()), None);
        repos.users.create(&user).await.unwrap();
        let tenant_id = TenantId::generate();
        let now = Utc::now();
        repos
            .tenants
            .create(&Tenant {
                id: tenant_id,
                name: format!("Tenant {tenant_id}"),
                slug: format!("tenant-{tenant_id}"),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: user.id,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: user.id,
                tenant_id: tenant_id.to_string(),
                provider: "whoop".to_owned(),
                access_token: SecretString::from("whoop_access_token"),
                refresh_token: Some(SecretString::from("whoop_refresh")),
                token_type: "Bearer".to_owned(),
                expires_at: Some(now + chrono::Duration::hours(1)),
                scope: Some("read:sleep".to_owned()),
                provider_user_id: None,
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        (user.id, tenant_id)
    }

    // carnet#995: one athlete's sync that never returns used to stop the
    // cycle, and with it background sync for every later athlete.
    #[tokio::test]
    async fn a_stalled_sync_is_dropped_at_its_deadline_and_the_next_athlete_syncs() {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let (stalled, _) = whoop_athlete(&repos).await;
        let (next, next_tenant) = whoop_athlete(&repos).await;

        let read = Arc::new(Mutex::new(Vec::new()));
        let storage = Arc::new(PierreSyncStorage::new(&repos));
        let deps = Arc::new(SyncDeps {
            sleep: storage.clone(),
            recovery: storage.clone(),
            health: storage.clone(),
            time_series: storage.clone(),
            data_sources: storage.clone(),
            cursors: storage.clone(),
            credentials: storage.clone(),
            connections: storage,
        });
        let provider: Box<dyn SyncProvider> = Box::new(StallsFor {
            stalled: stalled.to_string(),
            read: Arc::clone(&read),
        });
        let providers = HashMap::from([("whoop".to_owned(), provider)]);
        let orchestrator = Arc::new(SyncOrchestrator::new(
            deps,
            providers,
            SyncConfig::from_env(),
        ));

        let auth_repos = repos.auth_repos();
        let notifier: Arc<dyn SyncNotifier> = Arc::new(NoStreams);
        let notices = SyncFailureNotices::new(Arc::clone(&repos.provider_connections), None);
        let athletes: Vec<ConnectedUser> = [stalled, next]
            .iter()
            .map(|id| ConnectedUser {
                user_id: id.to_string(),
                provider: "whoop".to_owned(),
                connected_at: Utc::now(),
                is_active: true,
            })
            .collect();

        // Without a deadline this never returns; the outer bound turns that
        // hang into a failure.
        timeout(
            Duration::from_secs(30),
            sync_provider_users(
                &orchestrator,
                &auth_repos,
                &notifier,
                &notices,
                &athletes,
                "whoop",
                Duration::from_millis(300),
            ),
        )
        .await
        .expect("the cycle moves past the stalled athlete");

        assert_eq!(
            *read.lock().unwrap(),
            vec![stalled.to_string(), next.to_string()],
            "the stalled athlete was read first, then the next one"
        );
        let landed = auth_repos
            .oauth_tokens
            .get_provider_last_sync(next, next_tenant, "whoop")
            .await
            .unwrap();
        assert!(landed.is_some(), "the next athlete's sync landed");
    }
}
