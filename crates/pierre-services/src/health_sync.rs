// ABOUTME: Adapter bridging dravr-enforme's store traits to Pierre's RepositoryRegistry
// ABOUTME: Implements enforme's 8 granular store traits by delegating to pierre-database repos
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::Utc;
use dravr_equilibre_sync::SyncStatus;
use dravr_riviere::DataPoint;
use pierre_core::models::{TenantId, UserOAuthToken};
use pierre_database::repositories::SyncCursorRow;
use pierre_database::{AuthRepos, FitnessRepos, RepositoryRegistry};
use pierre_enforme::error::{EnformeError, EnformeResult};
use pierre_enforme::models::connection::{ConnectedUser, ProviderCredentials};
use pierre_enforme::models::cursor::SyncCursor;
use pierre_enforme::models::deletion::DeletionPolicy;
use pierre_enforme::providers::build_provider_registry_with_reader;
use pierre_enforme::providers::sciotte_reader::DailySummaryReader;
use pierre_enforme::traits::connection_store::UserConnectionStore;
use pierre_enforme::traits::credential_store::CredentialStore;
use pierre_enforme::traits::cursor_store::SyncCursorStore;
use pierre_enforme::traits::data_source_store::DataSourceStore;
use pierre_enforme::traits::health_store::HealthStore;
use pierre_enforme::traits::recovery_store::RecoveryStore;
use pierre_enforme::traits::sleep_store::SleepStore;
use pierre_enforme::traits::timeseries_store::TimeSeriesPointStore;
use pierre_providers::backend_resolver::sync_backend;
use tracing::info;
use uuid::Uuid;

use crate::sciotte_health_reader::SciotteServiceReader;
use crate::whoop_terms;

/// Adapter bridging dravr-enforme's store traits to Pierre's repository layer.
///
/// Wraps narrow `FitnessRepos` + `AuthRepos` views and implements each
/// of enforme's 8 granular store traits by delegating to the
/// corresponding repository method. The constructor still takes the
/// full `RepositoryRegistry` because the adapter cross-cuts both views
/// — internal narrowing keeps the trait impls tight without forcing
/// callers to construct two separate view-structs.
///
/// `dravr-equilibre` resolves to a single workspace-wide version (the patch
/// in the workspace `Cargo.toml` redirects the crates.io alias used by
/// enforme to the same git tag pierre-core consumes), so the model types
/// flow through this adapter without any cross-version translation.
///
/// The sync knows a provider by its own name (`garmin`, `coros`), while a
/// scrape-backed connection's session lives on its mirror backend's token row
/// (`sciotte_garmin`, `sciotte_coros`). Every lookup here — the roster, the
/// credentials, the tenant a row is written under — goes through
/// [`sync_backend`], the one mapping between the two.
///
/// Every WHOOP sleep and recovery record passes through
/// [`whoop_terms`] before it is written, so WHOOP's own scores
/// (recovery %, strain, sleep performance, WHOOP's sleep efficiency) are
/// never stored; a disconnect deletes the rest through the chokepoint's
/// provider data purge. No WHOOP sleep, recovery or health record is written
/// at all for an account that owes WHOOP's owner authorization
/// ([`whoop_terms::owner_authorization_outstanding`]); each store call logs
/// how many it withheld. WHOOP exposes no continuous series, so the
/// time-series store receives none of its data.
///
/// LIMITATION(registre#539): `PierreSyncStorage` persists WHOOP-classified physiology (HRV rMSSD, sleep
/// stage durations) instead of holding it in memory within the cache header, and applies no AI-clause
/// hygiene (WHOOP-derived turns kept out of evals, tuning sets and cross-athlete analytics); derived rows
/// with no provider column (`training_history`, `user_facts`) cannot be attributed to WHOOP by a purge.
pub struct PierreSyncStorage {
    /// Fitness-domain stores backing enforme's sleep / recovery / health /
    /// data-source / sync-cursor / time-series trait impls.
    fitness: FitnessRepos,
    /// Auth-domain handle for the `oauth_tokens` lookup behind
    /// `resolve_tenant_id` (enforme's contract gives us only `user_id` +
    /// `provider`, so we recover the owning tenant from the token row).
    auth: AuthRepos,
    /// Bridge to the platform OAuth refresh flow (`AuthService`), injected by
    /// pierre-server after runtime construction — the refresh path lives above
    /// this crate in the dependency graph, so it cannot be built here. Until
    /// injection, credential reads fall back to the stored token row (startup
    /// window before the first scheduled sync tick, and storage-only tests).
    refresher: OnceLock<Arc<dyn SyncCredentialRefresher>>,
    /// The full registry, for the WHOOP owner-authorization read: the notice
    /// flag and the account's acceptance live outside both views. Shared with
    /// the server's own handle to the same pools.
    registry: Arc<RepositoryRegistry>,
}

/// Which users' WHOOP records one store call withholds, decided once per
/// user, and how many it withheld.
struct OwnerAuthorizationGate<'a> {
    registry: &'a RepositoryRegistry,
    decided: HashMap<(TenantId, String), bool>,
    withheld: u64,
}

impl<'a> OwnerAuthorizationGate<'a> {
    fn new(registry: &'a RepositoryRegistry) -> Self {
        Self {
            registry,
            decided: HashMap::new(),
            withheld: 0,
        }
    }

    /// Whether a record from `source_name` for `user_id` must not be kept.
    ///
    /// True when it is WHOOP's and the account owes the owner authorization
    /// in `tenant_id`, the tenant it would be written under. Counts what it
    /// withholds.
    async fn withholds(
        &mut self,
        tenant_id: TenantId,
        user_id: &str,
        source_name: &str,
    ) -> EnformeResult<bool> {
        if !whoop_terms::is_whoop(source_name) {
            return Ok(false);
        }
        let key = (tenant_id, user_id.to_owned());
        let outstanding = if let Some(decided) = self.decided.get(&key) {
            *decided
        } else {
            let user_uuid = user_id
                .parse::<Uuid>()
                .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;
            let outstanding = whoop_terms::owner_authorization_outstanding(
                self.registry,
                tenant_id.as_uuid(),
                user_uuid,
            )
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to read the WHOOP owner authorization: {e}"))
            })?;
            self.decided.insert(key, outstanding);
            outstanding
        };
        if outstanding {
            self.withheld += 1;
        }
        Ok(outstanding)
    }

    /// Log what this call withheld, per user it withheld for.
    fn report(&self, data_type: &str) {
        if self.withheld == 0 {
            return;
        }
        for ((tenant_id, user_id), outstanding) in &self.decided {
            if *outstanding {
                info!(
                    %tenant_id,
                    user_id = %user_id,
                    data_type,
                    withheld_in_call = self.withheld,
                    "WHOOP records not persisted: the account has not accepted the current WHOOP owner-authorization notice"
                );
            }
        }
    }
}

/// Bridge to the platform's OAuth token-refresh infrastructure.
///
/// Implemented in pierre-server on top of `AuthService` (which refreshes
/// near-expiry tokens, persists the result, and maintains provider-connection
/// status). `pierre-services` sits below `pierre-tool-runtime` in the crate
/// graph, so the implementation is injected via
/// [`PierreSyncStorage::set_credential_refresher`] rather than called directly.
#[async_trait]
pub trait SyncCredentialRefresher: Send + Sync {
    /// Return valid credentials for the user+provider, refreshing through the
    /// platform OAuth flow when the stored token is expired or near expiry.
    /// `None` means no usable token exists and reconnecting is the remedy: none
    /// is stored, none can be refreshed, or the provider refused the refresh,
    /// now or earlier with the connection still flagged. An error means the
    /// lookup failed, or a refresh failed without the provider refusing it (a
    /// rate limit, a 5xx, a transport failure) over an unflagged connection:
    /// the grant stands, and a later sync can refresh it.
    async fn valid_credentials(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>>;

    /// Refresh the stored token regardless of its recorded expiry — the
    /// reactive path after the provider rejected the current token (e.g. a
    /// 401 despite a DB-valid `expires_at`). `None` and an error mean what
    /// they mean for [`Self::valid_credentials`].
    async fn force_refresh(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>>;
}

impl PierreSyncStorage {
    /// Create a new adapter wrapping the given repository registry.
    ///
    /// Cross-cuts `FitnessRepos` (sleep/recovery/health/etc.) and
    /// `AuthRepos` (`oauth_tokens` for tenant recovery); narrowing
    /// happens inside the struct.
    #[must_use]
    pub fn new(repos: &Arc<RepositoryRegistry>) -> Self {
        Self {
            fitness: repos.fitness_repos(),
            auth: repos.auth_repos(),
            refresher: OnceLock::new(),
            registry: Arc::clone(repos),
        }
    }

    /// Inject the OAuth refresh bridge once the server runtime exists.
    ///
    /// Idempotent: only the first injection wins (subsequent calls are
    /// ignored, mirroring `OnceLock` semantics).
    pub fn set_credential_refresher(&self, refresher: Arc<dyn SyncCredentialRefresher>) {
        let _ = self.refresher.set(refresher);
    }

    /// Build a `SyncOrchestrator` backed by this adapter with default configuration.
    ///
    /// Takes `&Arc<Self>` (not `self`) so the caller keeps a handle to the
    /// storage for post-construction injection of the credential refresher.
    /// The orchestrator is ready to run the scheduler or handle webhook events.
    /// The scrape-backed providers read their daily summaries on the dedicated
    /// sciotte service (ADR-021): no browser runs in this pod.
    #[must_use]
    pub fn build_orchestrator(self: &Arc<Self>) -> Arc<pierre_enforme::SyncOrchestrator> {
        let reader: Arc<dyn DailySummaryReader> = Arc::new(SciotteServiceReader);
        self.build_orchestrator_with_reader(&reader)
    }

    /// Build a `SyncOrchestrator` backed by this adapter whose scrape-backed
    /// providers read their daily summaries through `reader`.
    #[must_use]
    pub fn build_orchestrator_with_reader(
        self: &Arc<Self>,
        reader: &Arc<dyn DailySummaryReader>,
    ) -> Arc<pierre_enforme::SyncOrchestrator> {
        let storage = Arc::clone(self);
        let deps = Arc::new(pierre_enforme::SyncDeps {
            sleep: storage.clone(),
            recovery: storage.clone(),
            health: storage.clone(),
            time_series: storage.clone(),
            data_sources: storage.clone(),
            cursors: storage.clone(),
            credentials: storage.clone(),
            connections: storage,
        });

        let config = pierre_enforme::SyncConfig::from_env();
        let providers = build_provider_registry_with_reader(reader);

        Arc::new(pierre_enforme::SyncOrchestrator::new(
            deps, providers, config,
        ))
    }

    /// The token row that serves `provider` for this user: the row of its
    /// [`sync_backend`], or `None` when the user holds none.
    async fn serving_token(
        &self,
        user_uuid: Uuid,
        provider: &str,
    ) -> EnformeResult<Option<UserOAuthToken>> {
        let backend = sync_backend(provider);
        let tokens = self
            .auth
            .oauth_tokens
            .get_tokens(user_uuid, None)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to look up user tokens: {e}")))?;
        Ok(tokens.into_iter().find(|t| t.provider == backend))
    }

    /// Resolve the tenant a user's `provider` rows are written under: the
    /// tenant of the token row that serves that provider.
    ///
    /// There is no fallback to another provider's token. A record is written
    /// only under the tenant of the athlete's own connection to its provider:
    /// once a disconnect has deleted that connection and purged its rows, a
    /// sync still in flight must fail rather than write them again under
    /// another provider's tenant.
    async fn resolve_tenant_id(&self, user_id: &str, provider: &str) -> EnformeResult<TenantId> {
        let user_uuid = user_id
            .parse::<Uuid>()
            .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;

        let token = self
            .serving_token(user_uuid, provider)
            .await?
            .ok_or_else(|| {
                EnformeError::store(format!(
                    "No token serves provider '{provider}' for user '{user_id}'"
                ))
            })?;
        TenantId::parse_str(&token.tenant_id).map_err(|e| {
            EnformeError::store(format!("Invalid tenant_id UUID '{}': {e}", token.tenant_id))
        })
    }
}

impl fmt::Debug for PierreSyncStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PierreSyncStorage").finish_non_exhaustive()
    }
}

// ============================================================================
// SleepStore
// ============================================================================

#[async_trait]
impl SleepStore for PierreSyncStorage {
    async fn store_sleep_sessions(
        &self,
        sessions: &[dravr_equilibre_sync::StoredSleepSession],
    ) -> EnformeResult<u64> {
        let mut count = 0u64;
        let mut gate = OwnerAuthorizationGate::new(&self.registry);
        for session in sessions {
            let tenant_id = self
                .resolve_tenant_id(&session.user_id, &session.source_name)
                .await?;
            if gate
                .withholds(tenant_id, &session.user_id, &session.source_name)
                .await?
            {
                continue;
            }
            let session = whoop_terms::sleep_session_to_store(session);
            self.fitness
                .sleep
                .upsert_sleep_session(&tenant_id, &session)
                .await
                .map_err(|e| EnformeError::store(format!("Failed to upsert sleep session: {e}")))?;
            count += 1;
        }
        gate.report("sleep");
        Ok(count)
    }

    async fn delete_sleep_session(&self, id: &str, policy: &DeletionPolicy) -> EnformeResult<()> {
        // dravr-enforme's contract gives us the id only. Resolve the owning
        // tenant first (discovery query selects only tenant_id), then delete
        // through the tenant-scoped repository call so the data-access query
        // honors multi-tenant isolation.
        let tenant_id = self
            .fitness
            .sleep
            .find_sleep_session_tenant(id)
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to look up sleep session tenant: {e}"))
            })?
            .ok_or_else(|| {
                EnformeError::store(format!("Sleep session {id} not found for delete"))
            })?;

        self.fitness
            .sleep
            .delete_sleep_session_by_id(&tenant_id, id, policy.is_soft_delete())
            .await
            .map_err(|e| EnformeError::store(format!("Failed to delete sleep session: {e}")))?;

        Ok(())
    }
}

// ============================================================================
// RecoveryStore
// ============================================================================

#[async_trait]
impl RecoveryStore for PierreSyncStorage {
    async fn store_recovery_metrics(
        &self,
        metrics: &[dravr_equilibre_sync::StoredRecoveryMetrics],
    ) -> EnformeResult<u64> {
        let mut count = 0u64;
        let mut gate = OwnerAuthorizationGate::new(&self.registry);
        for metric in metrics {
            let tenant_id = self
                .resolve_tenant_id(&metric.user_id, &metric.source_name)
                .await?;
            if gate
                .withholds(tenant_id, &metric.user_id, &metric.source_name)
                .await?
            {
                continue;
            }
            let metric = whoop_terms::recovery_metrics_to_store(metric);
            self.fitness
                .recovery
                .upsert_recovery_metrics(&tenant_id, &metric)
                .await
                .map_err(|e| {
                    EnformeError::store(format!("Failed to upsert recovery metrics: {e}"))
                })?;
            count += 1;
        }
        gate.report("recovery");
        Ok(count)
    }

    async fn delete_recovery_metric(&self, id: &str, policy: &DeletionPolicy) -> EnformeResult<()> {
        // Resolve tenant first (discovery query) then delete through the
        // tenant-scoped repository call.
        let tenant_id = self
            .fitness
            .recovery
            .find_recovery_metric_tenant(id)
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to look up recovery metric tenant: {e}"))
            })?
            .ok_or_else(|| {
                EnformeError::store(format!("Recovery metric {id} not found for delete"))
            })?;

        self.fitness
            .recovery
            .delete_recovery_metric_by_id(&tenant_id, id, policy.is_soft_delete())
            .await
            .map_err(|e| EnformeError::store(format!("Failed to delete recovery metric: {e}")))?;

        Ok(())
    }
}

// ============================================================================
// HealthStore
// ============================================================================

#[async_trait]
impl HealthStore for PierreSyncStorage {
    async fn store_health_snapshots(
        &self,
        snapshots: &[dravr_equilibre_sync::StoredHealthMetrics],
    ) -> EnformeResult<u64> {
        let mut count = 0u64;
        let mut gate = OwnerAuthorizationGate::new(&self.registry);
        for snapshot in snapshots {
            let tenant_id = self
                .resolve_tenant_id(&snapshot.user_id, &snapshot.source_name)
                .await?;
            if gate
                .withholds(tenant_id, &snapshot.user_id, &snapshot.source_name)
                .await?
            {
                continue;
            }
            self.fitness
                .health_snapshots
                .upsert_health_snapshot(&tenant_id, snapshot)
                .await
                .map_err(|e| {
                    EnformeError::store(format!("Failed to upsert health snapshot: {e}"))
                })?;
            count += 1;
        }
        gate.report("health");
        Ok(count)
    }

    async fn delete_health_snapshot(&self, id: &str, policy: &DeletionPolicy) -> EnformeResult<()> {
        // Resolve tenant first (discovery query) then delete through the
        // tenant-scoped repository call.
        let tenant_id = self
            .fitness
            .health_snapshots
            .find_health_snapshot_tenant(id)
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to look up health snapshot tenant: {e}"))
            })?
            .ok_or_else(|| {
                EnformeError::store(format!("Health snapshot {id} not found for delete"))
            })?;

        self.fitness
            .health_snapshots
            .delete_health_snapshot_by_id(&tenant_id, id, policy.is_soft_delete())
            .await
            .map_err(|e| EnformeError::store(format!("Failed to delete health snapshot: {e}")))?;

        Ok(())
    }
}

// ============================================================================
// DataSourceStore
// ============================================================================

#[async_trait]
impl DataSourceStore for PierreSyncStorage {
    async fn upsert_data_source(
        &self,
        source: &dravr_equilibre_sync::DataSource,
    ) -> EnformeResult<String> {
        let tenant_id = self
            .resolve_tenant_id(&source.user_id, &source.provider)
            .await?;
        self.fitness
            .data_sources
            .upsert_data_source(&tenant_id, source)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to upsert data source: {e}")))
    }
}

// ============================================================================
// SyncCursorStore
// ============================================================================

#[async_trait]
impl SyncCursorStore for PierreSyncStorage {
    async fn get_cursor(
        &self,
        user_id: &str,
        provider: &str,
        data_type: &str,
    ) -> EnformeResult<Option<SyncCursor>> {
        let tenant_id = self.resolve_tenant_id(user_id, provider).await?;
        let row = self
            .fitness
            .sync_cursors
            .get_sync_cursor(user_id, &tenant_id, provider, data_type)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to get sync cursor: {e}")))?;

        Ok(row.map(|r| sync_cursor_row_to_enforme(&r)))
    }

    async fn update_cursor(&self, cursor: &SyncCursor) -> EnformeResult<()> {
        let tenant_id = self
            .resolve_tenant_id(&cursor.user_id, &cursor.provider)
            .await?;
        let row = sync_cursor_to_row(cursor, &tenant_id);
        self.fitness
            .sync_cursors
            .upsert_sync_cursor(&row)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to update sync cursor: {e}")))
    }
}

// ============================================================================
// CredentialStore
// ============================================================================

#[async_trait]
impl CredentialStore for PierreSyncStorage {
    async fn get_credentials(
        &self,
        user_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>> {
        let user_uuid = user_id
            .parse::<Uuid>()
            .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;

        let tenant_id = self.resolve_tenant_id(user_id, provider).await?;
        let backend = sync_backend(provider);

        // The injected refresher routes through AuthService::get_valid_token,
        // which transparently refreshes near-expiry tokens and persists the
        // result — the same path live tool calls use.
        if let Some(refresher) = self.refresher.get() {
            return refresher
                .valid_credentials(user_uuid, &tenant_id.to_string(), &backend)
                .await;
        }

        // No refresher yet (startup window before injection, storage-only
        // tests): return the stored token row as-is.
        let token = self
            .auth
            .oauth_tokens
            .get_token(user_uuid, tenant_id, &backend)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to get OAuth token: {e}")))?;

        Ok(token.map(|t| {
            let scopes = t
                .scope
                .map(|s| s.split(' ').map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();
            ProviderCredentials {
                access_token: t.access_token,
                refresh_token: t.refresh_token,
                expires_at: t.expires_at,
                scopes,
                user_id: t.user_id.to_string(),
                provider: t.provider,
                provider_user_id: t.provider_user_id,
            }
        }))
    }

    async fn refresh_credentials(
        &self,
        user_id: &str,
        provider: &str,
    ) -> EnformeResult<ProviderCredentials> {
        let expired = || EnformeError::CredentialsExpired {
            user_id: user_id.to_owned(),
            provider: provider.to_owned(),
        };

        if let Some(refresher) = self.refresher.get() {
            let user_uuid = user_id
                .parse::<Uuid>()
                .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;
            let tenant_id = self.resolve_tenant_id(user_id, provider).await?;
            return refresher
                .force_refresh(user_uuid, &tenant_id.to_string(), &sync_backend(provider))
                .await?
                .ok_or_else(expired);
        }

        // No refresher yet: return the stored credentials and let the caller
        // retry — without the OAuth bridge there is nothing to refresh with.
        self.get_credentials(user_id, provider)
            .await?
            .ok_or_else(expired)
    }
}

// ============================================================================
// UserConnectionStore
// ============================================================================

#[async_trait]
impl UserConnectionStore for PierreSyncStorage {
    async fn list_connected_users(&self, provider: &str) -> EnformeResult<Vec<ConnectedUser>> {
        let rows = self
            .fitness
            .sync_cursors
            .list_connected_provider_users(&sync_backend(provider))
            .await
            .map_err(|e| EnformeError::store(format!("Failed to list connected users: {e}")))?;

        // dravr-enforme's ConnectedUser.user_id is `String` (leaf-dep API).
        // ConnectedUserRow.user_id is the UserId newtype, so we render to the
        // canonical hyphenated form via Display at the boundary. The roster
        // names the provider as the sync knows it, whichever backend row
        // served the user.
        Ok(rows
            .into_iter()
            .map(|r| ConnectedUser {
                user_id: r.user_id.to_string(),
                provider: provider.to_owned(),
                connected_at: Utc::now(),
                is_active: true,
            })
            .collect())
    }
}

// ============================================================================
// TimeSeriesPointStore
// ============================================================================

#[async_trait]
impl TimeSeriesPointStore for PierreSyncStorage {
    async fn store_continuous_metrics(
        &self,
        source_id: &str,
        batches: &[dravr_equilibre_sync::ContinuousMetricBatch],
    ) -> EnformeResult<u64> {
        let mut total: u64 = 0;
        for batch in batches {
            // riviere's TimeSeriesStore::insert_batch reports success, not a row
            // count, so the written total is the number of points submitted.
            let count = batch.points.len() as u64;
            let points: Vec<DataPoint> = batch
                .points
                .iter()
                .map(|&(timestamp, value)| DataPoint::new(timestamp, value))
                .collect();
            self.fitness
                .time_series_points
                .insert_batch(source_id, batch.series_type_id, points)
                .await
                .map_err(|e| {
                    EnformeError::store(format!("Failed to insert continuous metrics: {e}"))
                })?;
            total += count;
        }
        Ok(total)
    }
}

// ============================================================================
// Conversion helpers
// ============================================================================

/// Convert a database `SyncCursorRow` to an enforme `SyncCursor`.
fn sync_cursor_row_to_enforme(row: &SyncCursorRow) -> SyncCursor {
    let status = match row.last_sync_status.as_str() {
        "completed" => SyncStatus::Completed,
        "failed" => SyncStatus::Failed,
        "in_progress" => SyncStatus::InProgress,
        "cancelled" => SyncStatus::Cancelled,
        _ => SyncStatus::Pending,
    };

    let last_sync_at = row.last_sync_at.unwrap_or_else(Utc::now);
    let next_retry_at = row.next_retry_at;

    SyncCursor {
        user_id: row.user_id.clone(),
        provider: row.provider.clone(),
        data_type: row.data_type.clone(),
        value: row.cursor_value.clone().unwrap_or_default(),
        last_sync_at,
        status,
        records_synced: row.records_synced as u64,
        error_message: row.error_message.clone(),
        retry_count: row.retry_count.cast_unsigned(),
        next_retry_at,
    }
}

/// Convert an enforme `SyncCursor` to a database `SyncCursorRow`.
fn sync_cursor_to_row(cursor: &SyncCursor, tenant_id: &TenantId) -> SyncCursorRow {
    let status_str = match cursor.status {
        SyncStatus::Pending => "pending",
        SyncStatus::InProgress => "in_progress",
        SyncStatus::Completed => "completed",
        SyncStatus::Failed => "failed",
        SyncStatus::Cancelled => "cancelled",
    };

    // Deterministic ID from user+tenant+provider+data_type
    let id = format!(
        "{}:{}:{}:{}",
        cursor.user_id, tenant_id, cursor.provider, cursor.data_type
    );

    SyncCursorRow {
        id,
        user_id: cursor.user_id.clone(),
        tenant_id: tenant_id.to_string(),
        provider: cursor.provider.clone(),
        data_type: cursor.data_type.clone(),
        cursor_value: if cursor.value.is_empty() {
            None
        } else {
            Some(cursor.value.clone())
        },
        last_sync_at: Some(cursor.last_sync_at),
        last_sync_status: status_str.to_owned(),
        records_synced: cursor.records_synced.cast_signed(),
        error_message: cursor.error_message.clone(),
        retry_count: cursor.retry_count.cast_signed(),
        next_retry_at: cursor.next_retry_at,
    }
}
