// ABOUTME: Provider data refresh service orchestrating on-demand and on-chat sync triggers
// ABOUTME: Checks data freshness per provider, delegates to enforme SyncOrchestrator, sends SSE notifications
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::fmt::Write as _;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
#[cfg(feature = "health-sync")]
use futures_util::future::join_all;
use pierre_core::errors::AppError;
use pierre_core::models::{
    DataFreshness, OAuthNotification, ProviderFreshness, RefreshConfig, RefreshStatus,
    SmartScheduleWeights, TenantId,
};
#[cfg(feature = "health-sync")]
use pierre_core::models::{DelegatedConnection, DelegationStatus};
use pierre_database::repositories::ActivityCacheRepository;
#[cfg(feature = "health-sync")]
use pierre_database::repositories::{CoachingGroupRepository, ProviderConnectionRepository};
use pierre_database::AuthRepos;
#[cfg(feature = "health-sync")]
use pierre_providers::backend_resolver::sync_backend;
#[cfg(feature = "health-sync")]
use pierre_providers::registry::global_registry;
use tokio::time::timeout;
use tracing::{info, instrument, warn};
use uuid::Uuid;

#[cfg(feature = "health-sync")]
use crate::coach_consents::{link_consents, wellness_read_skipped, WellnessReadSkip};
#[cfg(feature = "health-sync")]
use crate::coach_platform::coach_platform;
#[cfg(feature = "health-sync")]
use crate::delegated_read::{holds_own_session, OwnLogins};

/// Push interface used by the refresh service to inform connected clients.
///
/// Implemented by `pierre-server`'s `SseManager`; abstracted as a trait so
/// this service crate stays free of any pierre-server SSE/MCP machinery.
#[async_trait]
pub trait SyncNotifier: Send + Sync {
    /// Push an `OAuthNotification` to the user's active SSE stream, if any.
    ///
    /// # Errors
    /// Returns an error when no stream is registered for `user_id` or when
    /// the broadcast channel send fails. Callers typically treat these as
    /// best-effort and log at debug level.
    async fn send_notification(
        &self,
        user_id: Uuid,
        notification: &OAuthNotification,
    ) -> Result<(), AppError>;
}

/// Central service for provider data refresh decisions and execution.
///
/// Evaluates data freshness for a user's connected providers and triggers
/// background syncs via enforme's `SyncOrchestrator` when data is stale.
/// SSE notifications inform the client when sync completes.
pub struct RefreshService {
    /// Narrow repository view: `RefreshService` reads `repos.oauth_tokens`
    /// (last-sync timestamps for orchestrator-managed providers, credential
    /// refresh). On-demand providers' freshness comes from `activity_cache`.
    repos: AuthRepos,
    /// Source of truth for on-demand providers' freshness. sciotte (and other
    /// scrape-only providers) aren't background-synced, so their
    /// `oauth_tokens.last_sync` is written once at login and never updated —
    /// the cache's `synced_at` is the real timestamp of the last successful
    /// scrape. Read for on-demand providers so the agent hint reflects honest
    /// staleness instead of unconditionally reporting them Fresh.
    activity_cache: Arc<dyn ActivityCacheRepository>,
    /// The user's provider connections: whether a wellness source of their
    /// own keeps their human coach's link from syncing any wellness. Shared
    /// with the repository registry it is cloned from, whose pools every
    /// request reads through.
    #[cfg(feature = "health-sync")]
    provider_connections: Arc<dyn ProviderConnectionRepository>,
    /// The user's group memberships: whether they granted their human coach
    /// `wellness_read`, which a refresh served by the coach's link reports.
    #[cfg(feature = "health-sync")]
    groups: Arc<dyn CoachingGroupRepository>,
    /// Health data sync orchestrator (enforme).
    #[cfg(feature = "health-sync")]
    sync_orchestrator: Option<Arc<dravr_enforme::SyncOrchestrator>>,
    /// SSE manager for real-time notifications to clients. Only read by the
    /// `health-sync` spawn paths today; cfg-gated so non-health-sync builds
    /// (e.g. pierre-routes-web-admin's preflight clippy) don't see a dead
    /// field.
    #[cfg(feature = "health-sync")]
    sse_manager: Arc<dyn SyncNotifier>,
}

impl fmt::Debug for RefreshService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("RefreshService");
        debug.field("repos", &self.repos);
        #[cfg(feature = "health-sync")]
        debug.field("sync_orchestrator", &self.sync_orchestrator);
        debug.finish_non_exhaustive()
    }
}

impl RefreshService {
    /// Create a new refresh service.
    #[must_use]
    pub fn new(
        repos: &AuthRepos,
        activity_cache: Arc<dyn ActivityCacheRepository>,
        #[cfg(feature = "health-sync")] provider_connections: Arc<dyn ProviderConnectionRepository>,
        #[cfg(feature = "health-sync")] groups: Arc<dyn CoachingGroupRepository>,
        #[cfg(feature = "health-sync")] sync_orchestrator: Option<
            Arc<dravr_enforme::SyncOrchestrator>,
        >,
        #[cfg(feature = "health-sync")] sse_manager: Arc<dyn SyncNotifier>,
    ) -> Self {
        Self {
            repos: repos.clone(),
            activity_cache,
            #[cfg(feature = "health-sync")]
            provider_connections,
            #[cfg(feature = "health-sync")]
            groups,
            #[cfg(feature = "health-sync")]
            sync_orchestrator,
            #[cfg(feature = "health-sync")]
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
                .last_sync_for(user_id, tenant_id, &token.provider)
                .await;

            let freshness = DataFreshness::from_last_sync(last_sync);
            result.push(ProviderFreshness {
                provider: token.provider.clone(),
                last_sync_at: last_sync,
                freshness,
            });
        }

        result
    }

    /// The effective last-sync timestamp used to judge a provider's freshness.
    ///
    /// On-demand providers (sciotte) are scraped per chat request and never
    /// background-synced, so their `oauth_tokens.last_sync` is stamped once at
    /// login and never moves — reading it would report perpetual staleness (or,
    /// with the old shortcut, perpetual freshness). Their real freshness is the
    /// activity cache's `synced_at`, the timestamp of the last successful
    /// scrape. Orchestrator-managed providers keep using `oauth_tokens.last_sync`,
    /// which their background sync updates.
    async fn last_sync_for(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> Option<chrono::DateTime<Utc>> {
        // Without the `health-sync` feature there is no orchestrator at all, so
        // every provider is on-demand and sources its freshness from the cache.
        #[cfg(feature = "health-sync")]
        let on_demand = self.is_on_demand_provider(provider);
        #[cfg(not(feature = "health-sync"))]
        let on_demand = true;

        if on_demand {
            return self
                .activity_cache
                .latest_activity_sync(user_id, &tenant_id, provider)
                .await
                .unwrap_or_else(|e| {
                    warn!(
                        %user_id, %tenant_id, %provider, error = %e,
                        "Failed to read activity-cache sync time for on-demand provider; \
                         reporting freshness as never-synced"
                    );
                    None
                });
        }

        match self
            .repos
            .oauth_tokens
            .get_provider_last_sync(user_id, tenant_id, provider)
            .await
        {
            Ok(ts) => ts,
            Err(e) => {
                warn!(
                    %user_id, %tenant_id, %provider, error = %e,
                    "Failed to read provider last_sync timestamp; \
                     reporting freshness as never-synced for this provider"
                );
                None
            }
        }
    }

    /// Whether a provider is scraped on demand (per chat request) rather than
    /// synced in the background by the orchestrator. On-demand providers source
    /// their freshness from the activity cache and have no sync to trigger.
    ///
    /// Activity-only providers count as on-demand even when registered with
    /// the orchestrator: enforme background-syncs sleep/recovery/health/
    /// continuous data only, so a provider serving nothing but activities to
    /// chat gets no activity refresh from that sync. Reading the
    /// `oauth_tokens.last_sync` those (empty) cycles keep stamping reported
    /// Strava as perpetually Fresh, which suppressed the agent-hint directive
    /// to re-fetch via `get_activities` — the model then answered activity
    /// questions from stale conversation history (live failure 2026-07-11:
    /// OAuth-connected Strava user could not get activity data on messaging
    /// channels).
    ///
    /// A token row is sync-managed only when it is the row the sync reads
    /// ([`sync_backend`] of its own name). A scrape-connected athlete's
    /// session row (`sciotte_garmin`) is named by no sync, and a leftover
    /// `garmin` OAuth row is not what the Garmin sync reads: both stay
    /// on-demand, so a chat turn never scrapes on their account and their
    /// freshness is the activity cache's.
    #[cfg(feature = "health-sync")]
    fn is_on_demand_provider(&self, provider: &str) -> bool {
        if is_activity_only_provider(provider) || sync_backend(provider) != provider {
            return true;
        }
        self.sync_orchestrator
            .as_ref()
            .is_none_or(|o| !o.provider_names().contains(&provider))
    }

    /// Check freshness and refresh stale providers.
    ///
    /// When `config.wait_for_refresh` is `true` (the default), this awaits
    /// each per-provider sync up to `config.wait_for_refresh_timeout` so
    /// the LLM downstream reads the freshly-synced cache. Providers whose
    /// sync timed out or errored stay in the `refreshing` list and the
    /// agent-hint flags them as stale so the model can acknowledge the
    /// limitation. When `wait_for_refresh` is `false`, the sync runs as a
    /// detached background task and the LLM proceeds against the cache
    /// as it stands.
    ///
    /// On success the freshly-synced providers move into `fresh` so
    /// [`Self::build_agent_hint`] correctly omits them from the staleness
    /// hint — the data IS now fresh.
    #[instrument(skip(self, config), fields(%user_id, %tenant_id))]
    pub async fn check_and_refresh(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        config: &RefreshConfig,
    ) -> RefreshStatus {
        let mut freshness_list = self.get_provider_freshness(user_id, tenant_id).await;

        let mut refreshing = Vec::new();
        let mut fresh = Vec::new();
        // Providers the turn waits on. Each wait is bounded on its own, and they
        // run together below, so the turn waits for the slowest of them rather
        // than for the sum: N stale providers once cost N times the budget.
        #[cfg(feature = "health-sync")]
        let mut blocking: Vec<usize> = Vec::new();

        for (index, pf) in freshness_list.iter_mut().enumerate() {
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

            if !config.should_refresh_on_chat(age) {
                fresh.push(pf.provider.clone());
                continue;
            }

            #[cfg(feature = "health-sync")]
            {
                // On-demand providers (sciotte) reaching here are genuinely
                // stale per the activity cache, and there is no background sync
                // to trigger — they are re-scraped per chat request via
                // get_activities (the agent hint instructs the model to do so).
                // Report them honestly as stale instead of the previous
                // unconditional Fresh shortcut, which made the agent claim
                // sciotte data was current when it was hours old.
                if self.is_on_demand_provider(&pf.provider) {
                    refreshing.push(pf.provider.clone());
                    continue;
                }

                if config.wait_for_refresh {
                    blocking.push(index);
                } else {
                    self.spawn_provider_sync(user_id, tenant_id, pf.provider.clone());
                    refreshing.push(pf.provider.clone());
                }
            }
            #[cfg(not(feature = "health-sync"))]
            {
                let _ = (user_id, tenant_id, config, index);
                refreshing.push(pf.provider.clone());
            }
        }

        #[cfg(feature = "health-sync")]
        {
            let budget = config.wait_for_refresh_timeout();
            let syncs = blocking.iter().map(|&index| {
                let provider = freshness_list[index].provider.clone();
                async move {
                    bound_blocking_sync(
                        &provider,
                        budget,
                        self.sync_provider_blocking(user_id, tenant_id, &provider),
                    )
                    .await
                }
            });
            let results = join_all(syncs).await;
            for (&index, result) in blocking.iter().zip(results) {
                let pf = &mut freshness_list[index];
                if result.success {
                    // Reflect the just-completed sync so build_agent_hint
                    // (which reads .freshness) treats it as Fresh and
                    // omits the stale-data warning.
                    pf.last_sync_at = Some(Utc::now());
                    pf.freshness = DataFreshness::Fresh;
                    fresh.push(pf.provider.clone());
                } else {
                    refreshing.push(pf.provider.clone());
                }
            }
        }

        if !refreshing.is_empty() {
            info!(
                providers = ?refreshing,
                "Stale providers remain after refresh attempt"
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
    ///
    /// A user who reads the provider only through their human coach's link,
    /// on a platform whose linked members' wellness the scheduler syncs, is
    /// answered without a sync: that wellness syncs on the scheduler's own
    /// cadence through the coach's key once the member granted it, and never
    /// on demand (`Self::coach_link_serving`).
    #[instrument(skip(self), fields(%user_id, %tenant_id, %provider))]
    pub async fn refresh_provider(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        wait: bool,
    ) -> RefreshResult {
        #[cfg(feature = "health-sync")]
        {
            if let Some(link) = self.coach_link_serving(user_id, provider).await {
                let wellness_read = self.wellness_granted(&link).await;
                let skipped = if wellness_read {
                    self.wellness_skip(&link).await
                } else {
                    None
                };
                return RefreshResult {
                    provider: provider.to_owned(),
                    success: false,
                    message: coach_link_refresh_message(provider, wellness_read, skipped),
                    records_synced: 0,
                };
            }
            if wait {
                // Same budget as the provider=="all" path above — this branch
                // ran unbounded, so a wedged single-provider sync could hold a
                // chat turn open indefinitely while the "all" form of the same
                // ask was capped.
                bound_blocking_sync(
                    provider,
                    RefreshConfig::default().wait_for_refresh_timeout(),
                    self.sync_provider_blocking(user_id, tenant_id, provider),
                )
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
        #[cfg(not(feature = "health-sync"))]
        {
            let _ = (user_id, tenant_id, wait);
            RefreshResult {
                provider: provider.to_owned(),
                success: false,
                message: "health-sync feature not enabled".to_owned(),
                records_synced: 0,
            }
        }
    }

    /// Build an agent-facing freshness hint for the system prompt.
    ///
    /// Returns `None` if all providers are fresh or no providers are connected.
    #[must_use]
    pub fn build_agent_hint(freshness: &[ProviderFreshness]) -> Option<String> {
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

        // Provider-agnostic tool-call directive. The freshness lines
        // above are read-only — they tell the LLM "data may be stale"
        // but don't push it to act. Without this directive the model
        // happily answers from the prior turn's baked-in context (see
        // 2026-05-21 incident where the agent said "Le 17 mai" for the
        // user's "today only" question on the 20th because it never
        // re-fetched). The instruction below applies to every
        // connected provider via the same `get_activities` tool, so
        // it scales without per-provider plumbing.
        hint.push_str(
            "\nBefore answering any question that depends on recent activities, sleep, recovery, or workouts \
             (e.g. \"today only\", \"this week\", \"aujourd'hui\", \"hier\", \"recent sessions\"), invoke \
             `get_activities` with only a `limit` and NO `after`/`before` — the server returns your newest \
             activities first, so you never have to compute a Unix timestamp for this freshness fetch (a \
             miscomputed `before` a year in the past returns year-old data). Do not rely on data baked into \
             earlier turns when the freshness hint above lists a stale provider.",
        );

        Some(hint)
    }

    /// The human coach's confirmed link `user_id` reads `provider` through,
    /// when no token of their own serves its sync, a confirmed link the group
    /// relation backs does, and the provider's coaching platform is one whose
    /// linked members' wellness the scheduler syncs through the coach's key.
    ///
    /// A coaching platform that syncs nothing for a linked member keeps the
    /// answer it had (an on-demand platform has no sync to run), so a refresh
    /// never claims a schedule that does not exist. A lookup that fails reads
    /// as `None`, and the sync then runs: with no credential of the user's own
    /// and outside the scheduler's delegated pass, it reads nothing.
    #[cfg(feature = "health-sync")]
    async fn coach_link_serving(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Option<DelegatedConnection> {
        let backend = sync_backend(provider);
        coach_platform(&global_registry(), &backend)
            .filter(|platform| platform.reads_member_wellness())?;
        match holds_own_session(self.repos.oauth_tokens.as_ref(), user_id, None, &[&backend]).await
        {
            Ok(true) => return None,
            Ok(false) => {}
            Err(e) => {
                warn!(%user_id, error = %e, "Could not read the user's tokens before a refresh");
                return None;
            }
        }
        match self
            .repos
            .delegated_connections
            .list_backed_for_member(user_id, &backend)
            .await
        {
            Ok(links) => links
                .into_iter()
                .find(|link| link.status == DelegationStatus::Confirmed),
            Err(e) => {
                warn!(%user_id, error = %e, "Could not read the user's coach links before a refresh");
                None
            }
        }
    }

    /// Whether `link`'s member granted their coach `wellness_read`
    /// ([`link_consents`]); a lookup that fails is logged and reads as not
    /// granted.
    #[cfg(feature = "health-sync")]
    async fn wellness_granted(&self, link: &DelegatedConnection) -> bool {
        link_consents(self.groups.as_ref(), link)
            .await
            .map_or_else(
                |e| {
                    warn!(link_id = %link.id, error = %e, "Could not read the member's coach consents before a refresh");
                    false
                },
                |consents| consents.wellness_read,
            )
    }

    /// Why `link` syncs none of its member's wellness although they granted
    /// it ([`wellness_read_skipped`]); a lookup that fails is logged and
    /// reads as `None`.
    #[cfg(feature = "health-sync")]
    async fn wellness_skip(&self, link: &DelegatedConnection) -> Option<WellnessReadSkip> {
        let logins = OwnLogins {
            tokens: self.repos.oauth_tokens.as_ref(),
            connections: self.provider_connections.as_ref(),
        };
        wellness_read_skipped(logins, link)
            .await
            .unwrap_or_else(|e| {
                warn!(link_id = %link.id, error = %e, "Could not read the member's own wellness sources before a refresh");
                None
            })
    }

    /// Spawn a non-blocking background sync for a single provider.
    #[cfg(feature = "health-sync")]
    fn spawn_provider_sync(&self, user_id: Uuid, tenant_id: TenantId, provider: String) {
        {
            let Some(orchestrator) = self.sync_orchestrator.clone() else {
                warn!("SyncOrchestrator not available, skipping refresh for {provider}");
                return;
            };

            // Skip providers the orchestrator doesn't manage (e.g. sciotte, an
            // on-demand scrape-only provider fetched per chat request). Spawning
            // a sync for them only yields a "provider not registered" failure
            // and a bogus failure notification to the user.
            if !orchestrator.provider_names().contains(&provider.as_str()) {
                tracing::debug!(
                    provider = %provider,
                    "provider not background-syncable (on-demand); skipping scheduled sync"
                );
                return;
            }

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

                        info!(
                            target: "notify",
                            event = "sync.completed",
                            user_id = %user_id,
                            tenant_id = %tenant_id,
                            provider = %provider,
                            records = sync_result.records_created,
                            trigger = "background",
                            "provider sync completed"
                        );

                        // Update last_sync on the token row the sync read
                        if let Err(e) = repos
                            .oauth_tokens
                            .update_provider_last_sync(
                                user_id,
                                tenant_id,
                                &sync_backend(&provider),
                                Utc::now(),
                            )
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

                        info!(
                            target: "notify",
                            event = "sync.failed",
                            user_id = %user_id,
                            tenant_id = %tenant_id,
                            provider = %provider,
                            trigger = "background",
                            reason = %e,
                            "provider sync failed"
                        );

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
                        if let Err(e) = sse.send_notification(user_id, &notification).await {
                            tracing::debug!(%user_id, error = %e, "sync failure notification not delivered over SSE");
                        }
                    }
                }
            });
        }
    }

    /// Synchronously sync a provider and return the result.
    #[cfg(feature = "health-sync")]
    async fn sync_provider_blocking(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> RefreshResult {
        let Some(ref orchestrator) = self.sync_orchestrator else {
            return RefreshResult {
                provider: provider.to_owned(),
                success: false,
                message: "SyncOrchestrator not available".to_owned(),
                records_synced: 0,
            };
        };

        // On-demand providers (e.g. sciotte) are not registered with the
        // orchestrator — there is no background sync to run; they are scraped
        // per chat request. The chat path already classifies their freshness
        // from the activity cache (see `check_and_refresh`/`last_sync_for`), so
        // this only handles the explicit `refresh_provider` endpoint: report a
        // no-op success rather than a sync failure.
        if !orchestrator.provider_names().contains(&provider) {
            return RefreshResult {
                provider: provider.to_owned(),
                success: true,
                message: "on-demand provider; no scheduled sync".to_owned(),
                records_synced: 0,
            };
        }

        let user_id_str = user_id.to_string();
        match orchestrator.sync_user(&user_id_str, provider).await {
            Ok(sync_result) => {
                // Update last_sync on the token row the sync read
                if let Err(e) = self
                    .repos
                    .oauth_tokens
                    .update_provider_last_sync(
                        user_id,
                        tenant_id,
                        &sync_backend(provider),
                        Utc::now(),
                    )
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

/// Await a provider sync under a wall-clock budget, reporting the outcome.
///
/// Both blocking refresh paths — the per-provider sweep and the single-provider
/// ask — owe the same three behaviours: log a completed sync, log a reported
/// failure, and cap a sync that never returns. Written twice, they drifted:
/// only one of them capped the wait at all until 938829b8c, and the copy that
/// gained the cap also gained a line-wrap that put 34 spaces into the middle of
/// a sentence the agent relays to the athlete.
///
/// On timeout the awaited work is NOT respawned: it ran inside this task, so
/// `tokio::time::timeout` does not cancel it, and a second spawn would
/// double-sync.
pub async fn bound_blocking_sync<F>(provider: &str, budget: Duration, sync: F) -> RefreshResult
where
    F: Future<Output = RefreshResult>,
{
    match timeout(budget, sync).await {
        Ok(result) if result.success => {
            info!(
                provider,
                records = result.records_synced,
                "Blocking refresh completed before chat handoff"
            );
            result
        }
        Ok(result) => {
            warn!(
                provider,
                message = %result.message,
                "Blocking refresh reported failure; LLM will see stale cache"
            );
            result
        }
        Err(_elapsed) => {
            warn!(
                provider,
                timeout_secs = budget.as_secs(),
                "Blocking refresh exceeded budget; LLM will see stale cache"
            );
            RefreshResult {
                provider: provider.to_owned(),
                success: false,
                message: format!(
                    "Sync did not complete within {}s; ask again shortly or retry the refresh",
                    budget.as_secs()
                ),
                records_synced: 0,
            }
        }
    }
}

/// What an on-demand refresh answers a member who reads `provider` only
/// through their human coach's link: the scheduler syncs the link's wellness
/// every few hours once the member granted it `wellness_read`, unless a
/// source of their own already gives Dravr it (`skipped`), and syncs nothing
/// through it otherwise.
#[cfg(feature = "health-sync")]
fn coach_link_refresh_message(
    provider: &str,
    wellness_read: bool,
    skipped: Option<WellnessReadSkip>,
) -> String {
    match (wellness_read, skipped) {
        (false, _) => format!(
            "This athlete's {provider} data is read through their human coach's link when it is \
             asked for; the athlete has not let that link sync their wellness, so there is \
             nothing to refresh."
        ),
        (true, Some(WellnessReadSkip::OwnWellnessSource)) => format!(
            "This athlete's {provider} data is read through their human coach's link, which \
             syncs none of their wellness: their own Garmin or WHOOP connection already gives \
             Dravr their wellness, and refreshing that provider refreshes it. There is nothing \
             to refresh through the link."
        ),
        (true, Some(WellnessReadSkip::OwnConnection)) => format!(
            "This athlete's own {provider} connection is what Dravr syncs, not their human \
             coach's link; refresh it by its provider name."
        ),
        (true, None) => format!(
            "This athlete's {provider} data is read through their human coach's link, which \
             syncs their wellness on its own schedule every few hours; it cannot be refreshed \
             on demand."
        ),
    }
}

/// Whether the registry describes `provider` as serving activities and
/// nothing else (no sleep/recovery/health/continuous capabilities).
///
/// The health orchestrator has no activity data type, so background sync can
/// never refresh what an activity-only provider serves to chat; its honest
/// freshness source is the activity cache's last successful fetch.
#[cfg(feature = "health-sync")]
fn is_activity_only_provider(provider: &str) -> bool {
    global_registry().get_descriptor(provider).is_some_and(|d| {
        let caps = d.capabilities();
        caps.supports_activities()
            && !caps.supports_sleep()
            && !caps.supports_recovery()
            && !caps.supports_health()
            && !caps.supports_continuous_data()
    })
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

/// The scheduled sync cycle the server runs
#[cfg(feature = "health-sync")]
mod scheduled;

#[cfg(feature = "health-sync")]
pub use scheduled::token_tenant;
#[cfg(feature = "health-sync")]
pub use scheduled::{
    scrape_sync_not_due, start_scheduled_sync, sync_delegated_members, SCRAPE_SYNC_MIN_INTERVAL,
};

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

/// Record a sync latency observation. Only called from `sync_single_user`,
/// which is `#[cfg(feature = "health-sync")]`; cfg-gated here so the
/// fn isn't flagged as dead in non-health-sync builds.
#[cfg(feature = "health-sync")]
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
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
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
