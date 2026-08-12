// ABOUTME: Adapter bridging dravr-enforme's store traits to Pierre's RepositoryRegistry
// ABOUTME: Implements enforme's 8 granular store traits by delegating to pierre-database repos
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::Utc;
use dravr_equilibre_sync::SyncStatus;
use dravr_riviere::DataPoint;
use pierre_core::constants::oauth_providers;
use pierre_core::models::TenantId;
use pierre_database::repositories::SyncCursorRow;
use pierre_database::{AuthRepos, FitnessRepos, RepositoryRegistry};
use pierre_enforme::error::{EnformeError, EnformeResult};
use pierre_enforme::models::connection::{ConnectedUser, ProviderCredentials};
use pierre_enforme::models::cursor::SyncCursor;
use pierre_enforme::models::deletion::DeletionPolicy;
use pierre_enforme::providers::build_provider_registry;
use pierre_enforme::traits::connection_store::UserConnectionStore;
use pierre_enforme::traits::credential_store::CredentialStore;
use pierre_enforme::traits::cursor_store::SyncCursorStore;
use pierre_enforme::traits::data_source_store::DataSourceStore;
use pierre_enforme::traits::health_store::HealthStore;
use pierre_enforme::traits::recovery_store::RecoveryStore;
use pierre_enforme::traits::sleep_store::SleepStore;
use pierre_enforme::traits::timeseries_store::TimeSeriesPointStore;
use uuid::Uuid;

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
    /// `None` means no usable token exists (absent, or refresh failed).
    async fn valid_credentials(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &str,
    ) -> EnformeResult<Option<ProviderCredentials>>;

    /// Refresh the stored token regardless of its recorded expiry — the
    /// reactive path after the provider rejected the current token (e.g. a
    /// 401 despite a DB-valid `expires_at`).
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
    #[must_use]
    pub fn build_orchestrator(self: &Arc<Self>) -> Arc<pierre_enforme::SyncOrchestrator> {
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
        let providers = build_provider_registry();

        Arc::new(pierre_enforme::SyncOrchestrator::new(
            deps, providers, config,
        ))
    }

    /// Resolve `tenant_id` for a user by querying OAuth tokens for the given provider.
    ///
    /// Falls back to querying all tokens if provider-specific lookup yields nothing.
    async fn resolve_tenant_id(&self, user_id: &str, provider: &str) -> EnformeResult<TenantId> {
        let user_uuid = user_id
            .parse::<Uuid>()
            .map_err(|e| EnformeError::store(format!("Invalid user_id UUID: {e}")))?;

        // Look up tenant from the user's OAuth token for this provider
        let tokens = self
            .auth
            .oauth_tokens
            .get_tokens(user_uuid, None)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to look up user tokens: {e}")))?;

        // Find token matching this provider
        let matching = tokens
            .iter()
            .find(|t| t.provider == provider)
            .or_else(|| tokens.first());

        matching.map_or_else(
            || {
                Err(EnformeError::store(format!(
                    "No OAuth token found for user '{user_id}' provider '{provider}'"
                )))
            },
            |token| {
                TenantId::parse_str(&token.tenant_id).map_err(|e| {
                    EnformeError::store(format!(
                        "Invalid tenant_id UUID '{}': {e}",
                        token.tenant_id
                    ))
                })
            },
        )
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
        for session in sessions {
            let tenant_id = self
                .resolve_tenant_id(&session.user_id, &session.source_name)
                .await?;
            self.fitness
                .sleep
                .upsert_sleep_session(&tenant_id, session)
                .await
                .map_err(|e| EnformeError::store(format!("Failed to upsert sleep session: {e}")))?;
            count += 1;
        }
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
        for metric in metrics {
            let tenant_id = self
                .resolve_tenant_id(&metric.user_id, &metric.source_name)
                .await?;
            self.fitness
                .recovery
                .upsert_recovery_metrics(&tenant_id, metric)
                .await
                .map_err(|e| {
                    EnformeError::store(format!("Failed to upsert recovery metrics: {e}"))
                })?;
            count += 1;
        }
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
        for snapshot in snapshots {
            let tenant_id = self
                .resolve_tenant_id(&snapshot.user_id, &snapshot.source_name)
                .await?;
            self.fitness
                .health_snapshots
                .upsert_health_snapshot(&tenant_id, snapshot)
                .await
                .map_err(|e| {
                    EnformeError::store(format!("Failed to upsert health snapshot: {e}"))
                })?;
            count += 1;
        }
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

        // The injected refresher routes through AuthService::get_valid_token,
        // which transparently refreshes near-expiry tokens and persists the
        // result — the same path live tool calls use.
        if let Some(refresher) = self.refresher.get() {
            return refresher
                .valid_credentials(user_uuid, &tenant_id.to_string(), provider)
                .await;
        }

        // No refresher yet (startup window before injection, storage-only
        // tests): return the stored token row as-is.
        let token = self
            .auth
            .oauth_tokens
            .get_token(user_uuid, tenant_id, provider)
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
                .force_refresh(user_uuid, &tenant_id.to_string(), provider)
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
            .list_connected_provider_users(provider)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to list connected users: {e}")))?;

        // dravr-enforme's ConnectedUser.user_id is `String` (leaf-dep API).
        // ConnectedUserRow.user_id is the UserId newtype, so we render to the
        // canonical hyphenated form via Display at the boundary.
        //
        // enforme's Strava provider is the sciotte TSB scraper: it restores
        // the stored access token as a browser session, which only exists on
        // rows the sciotte connect flow wrote (`token_type = "session"`).
        // OAuth-connected Strava rows hold an opaque Bearer token the scraper
        // can never use — a sync for them is a guaranteed no-op that stamps
        // `last_sync` and logs a misleading "Scheduled sync completed", so
        // they are excluded from the sync roster here (their activities are
        // fetched on demand via `get_activities`; freshness comes from the
        // activity cache).
        Ok(rows
            .into_iter()
            .filter(|r| {
                provider != oauth_providers::STRAVA
                    || r.token_type == oauth_providers::TOKEN_TYPE_SESSION
            })
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
