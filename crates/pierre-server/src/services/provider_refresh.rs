// ABOUTME: Provider data refresh service orchestrating on-demand and on-chat sync triggers
// ABOUTME: Checks data freshness per provider, delegates to enforme SyncOrchestrator, sends SSE notifications
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use pierre_core::models::{
    DataFreshness, OAuthNotification, ProviderFreshness, RefreshConfig, RefreshStatus,
    SmartScheduleWeights, TenantId,
};
use pierre_database::RepositoryRegistry;
use tracing::{info, instrument, warn};
use uuid::Uuid;

#[cfg(feature = "health-sync")]
use pierre_enforme::models::connection::ConnectedUser;
#[cfg(feature = "health-sync")]
use tokio::task::AbortHandle;

use crate::providers::caching_provider::SyncCursorChecker;
use crate::services::provider_rate_limiter::{ProviderRateLimiter, RateLimitStatus};
use crate::sse::SseManager;

/// Central service for provider data refresh decisions and execution.
///
/// Evaluates data freshness for a user's connected providers and triggers
/// background syncs via enforme's `SyncOrchestrator` when data is stale.
/// SSE notifications inform the client when sync completes.
/// Push notifications are sent when the notification service is available.
pub struct RefreshService {
    /// Repository access for OAuth tokens and sync cursors.
    repos: Arc<RepositoryRegistry>,
    /// Health data sync orchestrator (enforme).
    #[cfg(feature = "health-sync")]
    sync_orchestrator: Option<Arc<pierre_enforme::SyncOrchestrator>>,
    /// SSE manager for real-time notifications to clients.
    sse_manager: Arc<SseManager>,
    /// Optional notification service for push notifications on sync completion.
    #[cfg(feature = "client-notifications")]
    notification_service: Option<Arc<pierre_notifications::NotificationService>>,
    /// Per-provider rate limiter shared across all tenants.
    rate_limiter: Option<Arc<ProviderRateLimiter>>,
}

impl RefreshService {
    /// Create a new refresh service.
    #[must_use]
    pub fn new(
        repos: Arc<RepositoryRegistry>,
        #[cfg(feature = "health-sync")] sync_orchestrator: Option<
            Arc<pierre_enforme::SyncOrchestrator>,
        >,
        sse_manager: Arc<SseManager>,
    ) -> Self {
        Self {
            repos,
            #[cfg(feature = "health-sync")]
            sync_orchestrator,
            sse_manager,
            #[cfg(feature = "client-notifications")]
            notification_service: None,
            rate_limiter: None,
        }
    }

    /// Set the notification service for push notifications on sync completion.
    #[cfg(feature = "client-notifications")]
    #[must_use]
    pub fn with_notification_service(
        mut self,
        service: Option<Arc<pierre_notifications::NotificationService>>,
    ) -> Self {
        self.notification_service = service;
        self
    }

    /// Set the per-provider rate limiter for API quota enforcement.
    #[must_use]
    pub fn with_rate_limiter(mut self, limiter: Arc<ProviderRateLimiter>) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    /// Get freshness status for all connected providers of a user.
    ///
    /// Queries `user_oauth_tokens` to find connected providers, then checks
    /// `last_sync` to determine freshness.
    #[instrument(skip(self), fields(%user_id, %tenant_id))]
    pub async fn get_provider_freshness(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> Vec<ProviderFreshness> {
        let tokens = match self
            .repos
            .oauth_tokens
            .get_tokens(user_id, Some(tenant_id))
            .await
        {
            Ok(tokens) => tokens,
            Err(e) => {
                warn!("Failed to get OAuth tokens for refresh check: {e}");
                return Vec::new();
            }
        };

        let mut result = Vec::with_capacity(tokens.len());
        for token in &tokens {
            let last_sync = self
                .repos
                .oauth_tokens
                .get_provider_last_sync(user_id, tenant_id, &token.provider)
                .await
                .unwrap_or(None);

            let freshness = DataFreshness::from_last_sync(last_sync);
            result.push(ProviderFreshness {
                provider: token.provider.clone(),
                last_sync_at: last_sync,
                freshness,
            });
        }

        result
    }

    /// Check freshness and trigger non-blocking background refresh for stale providers.
    ///
    /// Returns immediately with the current status. Syncs run in background
    /// tokio tasks and send SSE notifications on completion.
    #[instrument(skip(self, config), fields(%user_id, %tenant_id))]
    pub async fn check_and_refresh(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        config: &RefreshConfig,
    ) -> RefreshStatus {
        let freshness_list = self.get_provider_freshness(user_id, tenant_id).await;

        let mut refreshing = Vec::new();
        let mut fresh = Vec::new();

        for pf in &freshness_list {
            if !config.is_provider_eligible(&pf.provider) {
                fresh.push(pf.provider.clone());
                continue;
            }

            let age = pf.last_sync_at.map_or(Duration::from_secs(u64::MAX), |ts| {
                let delta = Utc::now().signed_duration_since(ts);
                if delta.num_seconds() < 0 {
                    Duration::ZERO
                } else {
                    #[allow(clippy::cast_sign_loss)]
                    Duration::from_secs(delta.num_seconds() as u64)
                }
            });

            if config.should_refresh_on_chat(age) {
                self.spawn_provider_sync(user_id, tenant_id, pf.provider.clone());
                refreshing.push(pf.provider.clone());
            } else {
                fresh.push(pf.provider.clone());
            }
        }

        if !refreshing.is_empty() {
            info!(
                providers = ?refreshing,
                "Triggered background refresh for stale providers"
            );
        }

        RefreshStatus {
            refreshing,
            fresh,
            details: freshness_list,
        }
    }

    /// Trigger a sync for a specific provider and optionally wait for completion.
    ///
    /// When `wait` is true, blocks until the sync finishes and returns the result.
    /// When `wait` is false, spawns a background task and returns immediately.
    #[instrument(skip(self), fields(%user_id, %tenant_id, %provider))]
    pub async fn refresh_provider(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        wait: bool,
    ) -> RefreshResult {
        if wait {
            self.sync_provider_blocking(user_id, tenant_id, provider)
                .await
        } else {
            self.spawn_provider_sync(user_id, tenant_id, provider.to_owned());
            RefreshResult {
                provider: provider.to_owned(),
                success: true,
                message: "Sync started in background".to_owned(),
                records_synced: 0,
            }
        }
    }

    /// Build a coach-facing freshness hint for the system prompt.
    ///
    /// Returns `None` if all providers are fresh or no providers are connected.
    #[must_use]
    pub fn build_coach_hint(freshness: &[ProviderFreshness]) -> Option<String> {
        if freshness.is_empty() {
            return None;
        }

        let stale_providers: Vec<&ProviderFreshness> = freshness
            .iter()
            .filter(|pf| !matches!(pf.freshness, DataFreshness::Fresh))
            .collect();

        if stale_providers.is_empty() {
            return None;
        }

        let mut hint = String::from(
            "Provider data freshness — some data may not reflect the user's most recent activities:\n",
        );
        for pf in &stale_providers {
            let age_str = pf.last_sync_at.map_or_else(
                || "never synced".to_owned(),
                |ts| format_age(Utc::now().signed_duration_since(ts)),
            );
            let _ = writeln!(hint, "- {}: {} ({})", pf.provider, pf.freshness, age_str);
        }

        Some(hint)
    }

    /// Spawn a non-blocking background sync for a single provider.
    fn spawn_provider_sync(&self, user_id: Uuid, tenant_id: TenantId, provider: String) {
        #[cfg(feature = "health-sync")]
        {
            let Some(orchestrator) = self.sync_orchestrator.clone() else {
                warn!("SyncOrchestrator not available, skipping refresh for {provider}");
                return;
            };

            // Check per-provider rate limit before spawning the sync task.
            if let Some(ref limiter) = self.rate_limiter {
                match limiter.check_rate_limit(&provider) {
                    RateLimitStatus::Allowed => limiter.record_call(&provider),
                    RateLimitStatus::Exceeded { retry_after } => {
                        warn!(
                            provider = %provider,
                            retry_after_secs = retry_after.as_secs(),
                            "Provider rate limit exceeded, skipping on-demand sync"
                        );
                        return;
                    }
                }
            }

            let repos = self.repos.clone();
            let sse = self.sse_manager.clone();
            let user_id_str = user_id.to_string();
            #[cfg(feature = "client-notifications")]
            let notif_service = self.notification_service.clone();

            tokio::spawn(async move {
                let result = orchestrator.sync_user(&user_id_str, &provider).await;

                match &result {
                    Ok(sync_result) => {
                        info!(
                            provider = %provider,
                            records_created = sync_result.records_created,
                            "Background provider sync completed"
                        );

                        // Update last_sync timestamp
                        if let Err(e) = repos
                            .oauth_tokens
                            .update_provider_last_sync(user_id, tenant_id, &provider, Utc::now())
                            .await
                        {
                            warn!("Failed to update last_sync after refresh: {e}");
                        }

                        // Send SSE notification
                        let notification = OAuthNotification {
                            id: Uuid::new_v4().to_string(),
                            user_id: user_id.to_string(),
                            provider: provider.clone(),
                            success: true,
                            message: format!(
                                "Synced {} new records from {}",
                                sync_result.records_created, provider
                            ),
                            expires_at: None,
                            created_at: Utc::now(),
                            read_at: None,
                        };
                        if let Err(e) = sse.send_notification(user_id, &notification).await {
                            // Not all users have an active SSE stream — this is expected
                            tracing::debug!(
                                "SSE notification not delivered (no active stream): {e}"
                            );
                        }

                        // Send push notification for successful syncs with records
                        #[cfg(feature = "client-notifications")]
                        if sync_result.records_created > 0 {
                            dispatch_sync_push_notification(
                                notif_service.as_deref(),
                                user_id,
                                tenant_id,
                                &provider,
                                sync_result.records_created,
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        warn!(provider = %provider, error = %e, "Background provider sync failed");

                        let notification = OAuthNotification {
                            id: Uuid::new_v4().to_string(),
                            user_id: user_id.to_string(),
                            provider: provider.clone(),
                            success: false,
                            message: format!("Failed to sync {provider}: {e}"),
                            expires_at: None,
                            created_at: Utc::now(),
                            read_at: None,
                        };
                        let _ = sse.send_notification(user_id, &notification).await;
                    }
                }
            });
        }

        #[cfg(not(feature = "health-sync"))]
        {
            let _ = (user_id, tenant_id, provider);
            warn!("health-sync feature not enabled, skipping provider refresh");
        }
    }

    /// Synchronously sync a provider and return the result.
    async fn sync_provider_blocking(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> RefreshResult {
        #[cfg(feature = "health-sync")]
        {
            let Some(ref orchestrator) = self.sync_orchestrator else {
                return RefreshResult {
                    provider: provider.to_owned(),
                    success: false,
                    message: "SyncOrchestrator not available".to_owned(),
                    records_synced: 0,
                };
            };

            let user_id_str = user_id.to_string();
            match orchestrator.sync_user(&user_id_str, provider).await {
                Ok(sync_result) => {
                    // Update last_sync timestamp
                    if let Err(e) = self
                        .repos
                        .oauth_tokens
                        .update_provider_last_sync(user_id, tenant_id, provider, Utc::now())
                        .await
                    {
                        warn!("Failed to update last_sync after blocking refresh: {e}");
                    }

                    RefreshResult {
                        provider: provider.to_owned(),
                        success: true,
                        message: format!("Synced {} new records", sync_result.records_created),
                        records_synced: sync_result.records_created,
                    }
                }
                Err(e) => RefreshResult {
                    provider: provider.to_owned(),
                    success: false,
                    message: format!("Sync failed: {e}"),
                    records_synced: 0,
                },
            }
        }

        #[cfg(not(feature = "health-sync"))]
        {
            let _ = (user_id, tenant_id, provider);
            RefreshResult {
                provider: provider.to_owned(),
                success: false,
                message: "health-sync feature not enabled".to_owned(),
                records_synced: 0,
            }
        }
    }
}

/// Result of a single provider refresh operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefreshResult {
    /// Provider that was refreshed.
    pub provider: String,
    /// Whether the sync completed successfully.
    pub success: bool,
    /// Human-readable status message.
    pub message: String,
    /// Number of records synced (0 if failed or async).
    pub records_synced: u32,
}

/// Format a chrono Duration as a human-readable age string.
fn format_age(duration: chrono::Duration) -> String {
    let total_secs = duration.num_seconds();
    if total_secs < 0 {
        return "just now".to_owned();
    }

    let hours = total_secs / 3_600;
    let days = hours / 24;

    if days > 0 {
        format!("{days}d ago")
    } else if hours > 0 {
        format!("{hours}h ago")
    } else {
        let mins = total_secs / 60;
        format!("{mins}m ago")
    }
}

// ============================================================================
// Cross-instance cache invalidation via sync cursor
// ============================================================================

/// Implements `SyncCursorChecker` by querying the `RepositoryRegistry`.
///
/// Looks up the latest sync timestamp for a user/provider pair so that
/// `CachingFitnessProvider` can invalidate cache entries when another
/// instance has recently synced fresher data.
pub struct RepositorySyncCursorChecker {
    /// Shared repository registry for database access.
    repos: Arc<RepositoryRegistry>,
    /// Tenant ID for the query (cache is already tenant-scoped via cache key).
    tenant_id: TenantId,
}

impl RepositorySyncCursorChecker {
    /// Create a new sync cursor checker for the given tenant.
    #[must_use]
    pub fn new(repos: Arc<RepositoryRegistry>, tenant_id: TenantId) -> Self {
        Self { repos, tenant_id }
    }
}

#[async_trait::async_trait]
impl SyncCursorChecker for RepositorySyncCursorChecker {
    async fn last_sync_at(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        self.repos
            .oauth_tokens
            .get_provider_last_sync(user_id, self.tenant_id, provider)
            .await
            .ok()
            .flatten()
    }
}

// ============================================================================
// Smart scheduling: per-user refresh interval based on activity patterns
// ============================================================================

/// Compute the poll interval for a user based on their recent activity count.
///
/// Daily trainers (high activity) get more frequent polls to ensure their data
/// stays current. Casual users (zero recent activities) get less frequent
/// polls to conserve provider API quota. Users in between get the default.
#[must_use]
pub fn compute_smart_interval(
    recent_activity_count: u32,
    weights: &SmartScheduleWeights,
) -> Duration {
    if recent_activity_count >= weights.daily_trainer_threshold {
        Duration::from_secs(weights.daily_trainer_interval_secs)
    } else if recent_activity_count == 0 {
        Duration::from_secs(weights.casual_interval_secs)
    } else {
        Duration::from_secs(weights.default_interval_secs)
    }
}

// ============================================================================
// Scheduled sync with post-sync notifications
// ============================================================================

/// Start the scheduled sync loop with post-sync SSE notifications.
///
/// Replaces enforme's built-in scheduler with a Pierre-aware version that:
/// - Iterates all connected users/providers on a jittered interval
/// - Calls `SyncOrchestrator::sync_user` for the actual sync
/// - Updates `user_oauth_tokens.last_sync` after successful syncs
/// - Sends SSE notifications to connected clients
/// - Tracks sync metrics (success/failure counts, latency)
/// - Checks per-provider rate limits before each sync
///
/// Returns an `AbortHandle` to cancel the background task on shutdown.
#[cfg(feature = "health-sync")]
pub fn start_scheduled_sync(
    orchestrator: Arc<pierre_enforme::SyncOrchestrator>,
    repos: Arc<RepositoryRegistry>,
    sse_manager: Arc<SseManager>,
    rate_limiter: Option<Arc<ProviderRateLimiter>>,
) -> AbortHandle {
    use pierre_enforme::orchestrator::scheduler::with_jitter;
    use tokio::time::sleep;

    let poll_interval = Duration::from_secs(orchestrator.config().poll_interval_secs);

    let handle = tokio::spawn(async move {
        info!(
            interval_secs = poll_interval.as_secs(),
            "Pierre scheduled sync started (with post-sync notifications)"
        );

        loop {
            let sleep_duration = with_jitter(poll_interval);
            sleep(sleep_duration).await;

            run_scheduled_sync_cycle(&orchestrator, &repos, &sse_manager, rate_limiter.as_ref())
                .await;
        }
    });

    let abort_handle = handle.abort_handle();
    info!("Scheduled sync task registered");
    abort_handle
}

/// Execute one full sync cycle across all providers and users.
#[cfg(feature = "health-sync")]
async fn run_scheduled_sync_cycle(
    orchestrator: &Arc<pierre_enforme::SyncOrchestrator>,
    repos: &Arc<RepositoryRegistry>,
    sse_manager: &Arc<SseManager>,
    rate_limiter: Option<&Arc<ProviderRateLimiter>>,
) {
    for provider_name in orchestrator.provider_names() {
        if is_provider_rate_limited(rate_limiter, provider_name) {
            continue;
        }

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
            rate_limiter,
            &users,
            provider_name,
        )
        .await;
    }
}

/// Sync all active users for a single provider, checking rate limits per user.
#[cfg(feature = "health-sync")]
async fn sync_provider_users(
    orchestrator: &Arc<pierre_enforme::SyncOrchestrator>,
    repos: &Arc<RepositoryRegistry>,
    sse_manager: &Arc<SseManager>,
    rate_limiter: Option<&Arc<ProviderRateLimiter>>,
    users: &[ConnectedUser],
    provider_name: &str,
) {
    for user in users {
        if !user.is_active {
            continue;
        }

        // Check rate limit before each individual user sync
        if let Some(limiter) = rate_limiter {
            match limiter.check_rate_limit(provider_name) {
                RateLimitStatus::Allowed => limiter.record_call(provider_name),
                RateLimitStatus::Exceeded { retry_after } => {
                    warn!(
                        provider = provider_name,
                        user_id = user.user_id,
                        retry_after_secs = retry_after.as_secs(),
                        "Provider rate limit hit during user iteration, stopping provider cycle"
                    );
                    break;
                }
            }
        }

        sync_single_user(orchestrator, repos, sse_manager, user, provider_name).await;
    }
}

/// Check whether a provider is rate-limited for the current window.
///
/// Returns `true` if the rate limit is exceeded and the provider should be skipped.
#[cfg(feature = "health-sync")]
fn is_provider_rate_limited(
    rate_limiter: Option<&Arc<ProviderRateLimiter>>,
    provider_name: &str,
) -> bool {
    let Some(limiter) = rate_limiter else {
        return false;
    };
    match limiter.check_rate_limit(provider_name) {
        RateLimitStatus::Allowed => false,
        RateLimitStatus::Exceeded { retry_after } => {
            warn!(
                provider = provider_name,
                retry_after_secs = retry_after.as_secs(),
                "Provider rate limit exceeded, skipping scheduled sync cycle for provider"
            );
            true
        }
    }
}

/// Sync a single user for a given provider, recording metrics and sending notifications.
#[cfg(feature = "health-sync")]
async fn sync_single_user(
    orchestrator: &Arc<pierre_enforme::SyncOrchestrator>,
    repos: &Arc<RepositoryRegistry>,
    sse_manager: &Arc<SseManager>,
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
                update_last_sync_timestamp(repos, user_uuid, provider_name).await;
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
        }
    }
}

/// Log the recommended smart-schedule interval for a user.
///
/// Activity count is not yet available from the database; uses 0 as baseline.
/// When a recent-activity-count query is added to the repository layer,
/// wire it here to enable true per-user adaptive scheduling.
#[cfg(feature = "health-sync")]
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

/// Update the `last_sync` timestamp on the OAuth token for the given user and provider.
#[cfg(feature = "health-sync")]
async fn update_last_sync_timestamp(
    repos: &Arc<RepositoryRegistry>,
    user_uuid: Uuid,
    provider_name: &str,
) {
    if let Ok(tokens) = repos.oauth_tokens.get_tokens(user_uuid, None).await {
        if let Some(token) = tokens.iter().find(|t| t.provider == provider_name) {
            if let Ok(tid) = token.tenant_id.parse::<TenantId>() {
                let _ = repos
                    .oauth_tokens
                    .update_provider_last_sync(user_uuid, tid, provider_name, Utc::now())
                    .await;
            }
        }
    }
}

/// Send an SSE notification after a successful sync with new records.
#[cfg(feature = "health-sync")]
async fn send_sync_notification(
    sse_manager: &Arc<SseManager>,
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

// ============================================================================
// Push notification dispatch for sync completions
// ============================================================================

/// Dispatch a push notification after a successful sync with new records.
///
/// Sends via the notification service (dravr-commere) when available.
/// Silently skips if no service is configured or dispatch fails (best-effort).
#[cfg(feature = "client-notifications")]
async fn dispatch_sync_push_notification(
    service: Option<&pierre_notifications::NotificationService>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider_name: &str,
    records_created: u32,
) {
    use pierre_notifications::models::NotificationCategory as CommNotifCategory;
    use pierre_notifications::{DispatchRequest, TenantId as CommTenantId};

    let Some(svc) = service else {
        return;
    };

    let request = DispatchRequest {
        user_id,
        tenant_id: CommTenantId(tenant_id.0),
        category: CommNotifCategory::Training,
        notification_type: "data_synced".to_owned(),
        title: "Data Synced".to_owned(),
        body: format!("Synced {records_created} new records from {provider_name}"),
        data: None,
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };

    match svc.dispatch(&request).await {
        Ok(outcome) => {
            tracing::debug!(
                user_id = %user_id,
                provider = provider_name,
                outcome = ?outcome,
                "Push notification dispatched for sync"
            );
        }
        Err(e) => {
            tracing::debug!(
                user_id = %user_id,
                provider = provider_name,
                error = %e,
                "Push notification dispatch failed (best-effort)"
            );
        }
    }
}

// ============================================================================
// Observability: sync metrics
// ============================================================================

use std::sync::atomic::{AtomicU64, Ordering};

/// Total successful sync operations since process start.
static SYNC_SUCCESSES: AtomicU64 = AtomicU64::new(0);

/// Total failed sync operations since process start.
static SYNC_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Sum of sync latencies in milliseconds (for computing average).
static SYNC_LATENCY_SUM_MS: AtomicU64 = AtomicU64::new(0);

/// Maximum sync latency observed in milliseconds.
static SYNC_LATENCY_MAX_MS: AtomicU64 = AtomicU64::new(0);

/// Record a sync latency observation.
fn record_sync_latency(elapsed: Duration) {
    let ms = elapsed.as_millis() as u64;
    SYNC_LATENCY_SUM_MS.fetch_add(ms, Ordering::Relaxed);

    // Update max using CAS loop
    let mut current = SYNC_LATENCY_MAX_MS.load(Ordering::Relaxed);
    while ms > current {
        match SYNC_LATENCY_MAX_MS.compare_exchange_weak(
            current,
            ms,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

/// Snapshot of sync metrics for observability endpoints.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncMetrics {
    /// Total successful syncs since process start.
    pub successes: u64,
    /// Total failed syncs since process start.
    pub failures: u64,
    /// Average sync latency in milliseconds (0 if no syncs).
    pub avg_latency_ms: u64,
    /// Maximum sync latency observed in milliseconds.
    pub max_latency_ms: u64,
}

impl SyncMetrics {
    /// Capture current metric values.
    #[must_use]
    pub fn snapshot() -> Self {
        let successes = SYNC_SUCCESSES.load(Ordering::Relaxed);
        let failures = SYNC_FAILURES.load(Ordering::Relaxed);
        let latency_sum = SYNC_LATENCY_SUM_MS.load(Ordering::Relaxed);
        let max_latency = SYNC_LATENCY_MAX_MS.load(Ordering::Relaxed);

        let total = successes + failures;
        let avg_latency_ms = latency_sum.checked_div(total).unwrap_or(0);

        Self {
            successes,
            failures,
            avg_latency_ms,
            max_latency_ms: max_latency,
        }
    }
}
