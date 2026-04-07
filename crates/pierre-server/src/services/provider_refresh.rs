// ABOUTME: Provider data refresh service orchestrating on-demand and on-chat sync triggers
// ABOUTME: Checks data freshness per provider, delegates to enforme SyncOrchestrator, sends SSE notifications

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_core::models::{
    DataFreshness, OAuthNotification, ProviderFreshness, RefreshConfig, RefreshStatus, TenantId,
};
use pierre_database::RepositoryRegistry;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::sse::SseManager;

/// Central service for provider data refresh decisions and execution.
///
/// Evaluates data freshness for a user's connected providers and triggers
/// background syncs via enforme's `SyncOrchestrator` when data is stale.
/// SSE notifications inform the client when sync completes.
pub struct RefreshService {
    /// Repository access for OAuth tokens and sync cursors.
    repos: Arc<RepositoryRegistry>,
    /// Health data sync orchestrator (enforme).
    #[cfg(feature = "health-sync")]
    sync_orchestrator: Option<Arc<dravr_enforme::SyncOrchestrator>>,
    /// SSE manager for real-time notifications to clients.
    sse_manager: Arc<SseManager>,
}

impl RefreshService {
    /// Create a new refresh service.
    #[must_use]
    pub fn new(
        repos: Arc<RepositoryRegistry>,
        #[cfg(feature = "health-sync")] sync_orchestrator: Option<
            Arc<dravr_enforme::SyncOrchestrator>,
        >,
        sse_manager: Arc<SseManager>,
    ) -> Self {
        Self {
            repos,
            #[cfg(feature = "health-sync")]
            sync_orchestrator,
            sse_manager,
        }
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

            let age = pf
                .last_sync_at
                .map(|ts| {
                    let delta = Utc::now().signed_duration_since(ts);
                    if delta.num_seconds() < 0 {
                        Duration::ZERO
                    } else {
                        #[allow(clippy::cast_sign_loss)]
                        Duration::from_secs(delta.num_seconds() as u64)
                    }
                })
                .unwrap_or(Duration::from_secs(u64::MAX));

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
            let age_str = pf
                .last_sync_at
                .map(|ts| format_age(Utc::now().signed_duration_since(ts)))
                .unwrap_or_else(|| "never synced".to_owned());
            hint.push_str(&format!(
                "- {}: {} ({})\n",
                pf.provider, pf.freshness, age_str
            ));
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

            let repos = self.repos.clone();
            let sse = self.sse_manager.clone();
            let user_id_str = user_id.to_string();

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
///
/// Returns an `AbortHandle` to cancel the background task on shutdown.
#[cfg(feature = "health-sync")]
pub fn start_scheduled_sync(
    orchestrator: Arc<dravr_enforme::SyncOrchestrator>,
    repos: Arc<RepositoryRegistry>,
    sse_manager: Arc<SseManager>,
) -> tokio::task::AbortHandle {
    use dravr_enforme::orchestrator::scheduler::with_jitter;

    let poll_interval = std::time::Duration::from_secs(orchestrator.config().poll_interval_secs);

    let handle = tokio::spawn(async move {
        info!(
            interval_secs = poll_interval.as_secs(),
            "Pierre scheduled sync started (with post-sync notifications)"
        );

        loop {
            let sleep_duration = with_jitter(poll_interval);
            tokio::time::sleep(sleep_duration).await;

            run_scheduled_sync_cycle(&orchestrator, &repos, &sse_manager).await;
        }
    });

    let abort_handle = handle.abort_handle();
    info!("Scheduled sync task registered");
    abort_handle
}

/// Execute one full sync cycle across all providers and users.
#[cfg(feature = "health-sync")]
async fn run_scheduled_sync_cycle(
    orchestrator: &Arc<dravr_enforme::SyncOrchestrator>,
    repos: &Arc<RepositoryRegistry>,
    sse_manager: &Arc<SseManager>,
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
                SYNC_FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }
        };

        for user in &users {
            if !user.is_active {
                continue;
            }

            let start = std::time::Instant::now();
            match orchestrator.sync_user(&user.user_id, provider_name).await {
                Ok(result) => {
                    let elapsed = start.elapsed();
                    SYNC_SUCCESSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    record_sync_latency(elapsed);

                    info!(
                        user_id = user.user_id,
                        provider = provider_name,
                        records_created = result.records_created,
                        elapsed_ms = elapsed.as_millis() as u64,
                        "Scheduled sync completed"
                    );

                    // Update last_sync timestamp on oauth tokens
                    if let Ok(user_uuid) = user.user_id.parse::<Uuid>() {
                        // Resolve tenant_id from the token
                        if let Ok(tokens) = repos.oauth_tokens.get_tokens(user_uuid, None).await {
                            if let Some(token) = tokens.iter().find(|t| t.provider == provider_name)
                            {
                                if let Ok(tid) =
                                    token.tenant_id.parse::<pierre_core::models::TenantId>()
                                {
                                    let _ = repos
                                        .oauth_tokens
                                        .update_provider_last_sync(
                                            user_uuid,
                                            tid,
                                            provider_name,
                                            Utc::now(),
                                        )
                                        .await;
                                }
                            }
                        }

                        // SSE notify (best-effort, user may not have active stream)
                        if result.records_created > 0 {
                            let notification = OAuthNotification {
                                id: Uuid::new_v4().to_string(),
                                user_id: user.user_id.clone(),
                                provider: provider_name.to_owned(),
                                success: true,
                                message: format!(
                                    "Synced {} new records from {}",
                                    result.records_created, provider_name
                                ),
                                expires_at: None,
                                created_at: Utc::now(),
                                read_at: None,
                            };
                            let _ = sse_manager
                                .send_notification(user_uuid, &notification)
                                .await;
                        }
                    }
                }
                Err(e) => {
                    SYNC_FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    warn!(
                        user_id = user.user_id,
                        provider = provider_name,
                        error = %e,
                        "Scheduled sync failed"
                    );
                }
            }
        }
    }
}

// ============================================================================
// Observability: sync metrics
// ============================================================================

use std::sync::atomic::AtomicU64;

/// Total successful sync operations since process start.
static SYNC_SUCCESSES: AtomicU64 = AtomicU64::new(0);

/// Total failed sync operations since process start.
static SYNC_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Sum of sync latencies in milliseconds (for computing average).
static SYNC_LATENCY_SUM_MS: AtomicU64 = AtomicU64::new(0);

/// Maximum sync latency observed in milliseconds.
static SYNC_LATENCY_MAX_MS: AtomicU64 = AtomicU64::new(0);

/// Record a sync latency observation.
fn record_sync_latency(elapsed: std::time::Duration) {
    let ms = elapsed.as_millis() as u64;
    SYNC_LATENCY_SUM_MS.fetch_add(ms, std::sync::atomic::Ordering::Relaxed);

    // Update max using CAS loop
    let mut current = SYNC_LATENCY_MAX_MS.load(std::sync::atomic::Ordering::Relaxed);
    while ms > current {
        match SYNC_LATENCY_MAX_MS.compare_exchange_weak(
            current,
            ms,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
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
        let successes = SYNC_SUCCESSES.load(std::sync::atomic::Ordering::Relaxed);
        let failures = SYNC_FAILURES.load(std::sync::atomic::Ordering::Relaxed);
        let latency_sum = SYNC_LATENCY_SUM_MS.load(std::sync::atomic::Ordering::Relaxed);
        let max_latency = SYNC_LATENCY_MAX_MS.load(std::sync::atomic::Ordering::Relaxed);

        let total = successes + failures;
        let avg_latency_ms = if total > 0 { latency_sum / total } else { 0 };

        Self {
            successes,
            failures,
            avg_latency_ms,
            max_latency_ms: max_latency,
        }
    }
}
