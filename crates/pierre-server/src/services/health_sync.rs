// ABOUTME: Adapter bridging dravr-enforme's store traits to Pierre's RepositoryRegistry
// ABOUTME: Implements enforme's 8 granular store traits by delegating to pierre-database repos
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dravr_enforme::error::{EnformeError, EnformeResult};
use dravr_enforme::models::connection::{ConnectedUser, ProviderCredentials};
use dravr_enforme::models::cursor::SyncCursor;
use dravr_enforme::models::deletion::DeletionPolicy;
use dravr_enforme::providers::build_provider_registry;
use dravr_enforme::traits::connection_store::UserConnectionStore;
use dravr_enforme::traits::credential_store::CredentialStore;
use dravr_enforme::traits::cursor_store::SyncCursorStore;
use dravr_enforme::traits::data_source_store::DataSourceStore;
use dravr_enforme::traits::health_store::HealthStore;
use dravr_enforme::traits::recovery_store::RecoveryStore;
use dravr_enforme::traits::sleep_store::SleepStore;
use dravr_enforme::traits::timeseries_store::TimeSeriesPointStore;
use dravr_equilibre_sync::SyncStatus;
use pierre_core::models::TenantId;
use pierre_database::repositories::SyncCursorRow;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Adapter bridging dravr-enforme's store traits to Pierre's repository layer.
///
/// Wraps `Arc<RepositoryRegistry>` and implements each of enforme's 8 granular
/// store traits by delegating to the corresponding repository method.
///
/// `dravr-equilibre` resolves to a single workspace-wide version (the patch
/// in the workspace `Cargo.toml` redirects the crates.io alias used by
/// enforme to the same git tag pierre-core consumes), so the model types
/// flow through this adapter without any cross-version translation.
pub struct PierreSyncStorage {
    repos: Arc<RepositoryRegistry>,
}

impl PierreSyncStorage {
    /// Create a new adapter wrapping the given repository registry.
    #[must_use]
    pub fn new(repos: Arc<RepositoryRegistry>) -> Self {
        Self { repos }
    }

    /// Build a `SyncOrchestrator` backed by this adapter with default configuration.
    ///
    /// The orchestrator is ready to run the scheduler or handle webhook events.
    #[must_use]
    pub fn into_orchestrator(self) -> Arc<dravr_enforme::SyncOrchestrator> {
        let storage: Arc<Self> = Arc::new(self);
        let deps = Arc::new(dravr_enforme::SyncDeps {
            sleep: storage.clone(),
            recovery: storage.clone(),
            health: storage.clone(),
            time_series: storage.clone(),
            data_sources: storage.clone(),
            cursors: storage.clone(),
            credentials: storage.clone(),
            connections: storage,
        });

        let config = dravr_enforme::SyncConfig::from_env();
        let providers = build_provider_registry();

        Arc::new(dravr_enforme::SyncOrchestrator::new(
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
            .repos
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
                token.tenant_id.parse::<TenantId>().map_err(|e| {
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
            self.repos
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
            .repos
            .sleep
            .find_sleep_session_tenant(id)
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to look up sleep session tenant: {e}"))
            })?
            .ok_or_else(|| {
                EnformeError::store(format!("Sleep session {id} not found for delete"))
            })?;

        self.repos
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
            self.repos
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
            .repos
            .recovery
            .find_recovery_metric_tenant(id)
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to look up recovery metric tenant: {e}"))
            })?
            .ok_or_else(|| {
                EnformeError::store(format!("Recovery metric {id} not found for delete"))
            })?;

        self.repos
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
            self.repos
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
            .repos
            .health_snapshots
            .find_health_snapshot_tenant(id)
            .await
            .map_err(|e| {
                EnformeError::store(format!("Failed to look up health snapshot tenant: {e}"))
            })?
            .ok_or_else(|| {
                EnformeError::store(format!("Health snapshot {id} not found for delete"))
            })?;

        self.repos
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
        self.repos
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
            .repos
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
        self.repos
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
        let token = self
            .repos
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
        // Token refresh is handled by Pierre's OAuth flow infrastructure.
        // Here we just return the current credentials and let the caller retry.
        self.get_credentials(user_id, provider)
            .await?
            .ok_or_else(|| EnformeError::CredentialsExpired {
                user_id: user_id.to_owned(),
                provider: provider.to_owned(),
            })
    }
}

// ============================================================================
// UserConnectionStore
// ============================================================================

#[async_trait]
impl UserConnectionStore for PierreSyncStorage {
    async fn list_connected_users(&self, provider: &str) -> EnformeResult<Vec<ConnectedUser>> {
        let rows = self
            .repos
            .sync_cursors
            .list_connected_provider_users(provider)
            .await
            .map_err(|e| EnformeError::store(format!("Failed to list connected users: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| ConnectedUser {
                user_id: r.user_id,
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
            let written = self
                .repos
                .time_series_points
                .insert_continuous_metrics_batch(source_id, batch.series_type_id, &batch.points)
                .await
                .map_err(|e| {
                    EnformeError::store(format!("Failed to insert continuous metrics: {e}"))
                })?;
            total += written;
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

    let last_sync_at = row
        .last_sync_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc));

    let next_retry_at = row
        .next_retry_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    SyncCursor {
        user_id: row.user_id.clone(),
        provider: row.provider.clone(),
        data_type: row.data_type.clone(),
        value: row.cursor_value.clone().unwrap_or_default(),
        last_sync_at,
        status,
        records_synced: row.records_synced as u64,
        error_message: row.error_message.clone(),
        retry_count: row.retry_count as u32,
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
        last_sync_at: Some(cursor.last_sync_at.to_rfc3339()),
        last_sync_status: status_str.to_owned(),
        records_synced: cursor.records_synced.cast_signed(),
        error_message: cursor.error_message.clone(),
        retry_count: i64::from(cursor.retry_count),
        next_retry_at: cursor.next_retry_at.map(|dt| dt.to_rfc3339()),
    }
}
