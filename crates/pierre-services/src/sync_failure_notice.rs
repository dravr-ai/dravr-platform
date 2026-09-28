// ABOUTME: The one notice an athlete gets when a provider sync fails, claimed once per failing provider
// ABOUTME: Deduped on the connection's notified_at marker, re-armed when a sync lands, silent on a dead credential

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Telling an athlete a provider sync failed, once.
//!
//! Every path that syncs a provider on the athlete's behalf without them
//! asking — the scheduled cycle, a provider's webhook, the backfill after a
//! connect — reports its outcome here. A failure claims the connection's
//! `notified_at` marker (the same one the disconnect notice claims) and only
//! the claim's winner dispatches, so a provider failing every fifteen minutes
//! costs the athlete one notice, not one per cycle. A sync that lands clears
//! the claim, so the next failure is told again.
//!
//! A dead credential is not a sync failure: the refresh that found it flips
//! the connection to `needs_reauth` and sends the disconnect notice, and the
//! claim here only ever succeeds on an `active` connection, so the athlete is
//! never told twice. The provider's own error text never reaches the athlete;
//! it is logged where the failure happened.

use std::sync::Arc;

use pierre_core::models::TenantId;
use pierre_database::repositories::ProviderConnectionRepository;
#[cfg(feature = "health-sync")]
use pierre_enforme::EnformeError;
use pierre_notifications::triggers::trigger_sync_failure;
use pierre_notifications::{NotificationService, TenantId as CommTenantId};
use pierre_providers::backend_resolver::{brand_name, user_facing_name};
use pierre_providers::registry::global_registry;
use tracing::{debug, warn};
use uuid::Uuid;

/// Reports sync outcomes for the athlete's connections and sends the one
/// sync-failure notice each failing provider is owed.
#[derive(Clone)]
pub struct SyncFailureNotices {
    /// The connection rows whose `notified_at` marker dedupes the notice.
    connections: Arc<dyn ProviderConnectionRepository>,
    /// Where the notice is dispatched. `None` when the deployment runs without
    /// the notification backend: failures are then only claimed and logged.
    service: Option<Arc<NotificationService>>,
}

impl SyncFailureNotices {
    /// Notices over `connections`, dispatched through `service`.
    #[must_use]
    pub fn new(
        connections: Arc<dyn ProviderConnectionRepository>,
        service: Option<Arc<NotificationService>>,
    ) -> Self {
        Self {
            connections,
            service,
        }
    }

    /// A sync of `backend` for the athlete failed: tell them, unless they were
    /// already told since its last sync landed or the connection is not active.
    ///
    /// `backend` is the connection row's provider name — the token row the
    /// sync read. Best-effort: a claim that cannot be written sends nothing.
    pub async fn sync_failed(&self, user_id: Uuid, tenant_id: TenantId, backend: &str) {
        let claimed = match self
            .connections
            .claim_sync_failure_notification(user_id, tenant_id, backend)
            .await
        {
            Ok(claimed) => claimed,
            Err(e) => {
                warn!(%user_id, provider = backend, error = %e, "Failed to claim the sync-failure notice");
                return;
            }
        };
        if !claimed {
            debug!(%user_id, provider = backend, "Sync failure already told, or the connection is not active");
            return;
        }
        let Some(service) = &self.service else {
            return;
        };
        let name =
            brand_name(&global_registry(), backend).unwrap_or_else(|| user_facing_name(backend));
        trigger_sync_failure(service, user_id, CommTenantId(tenant_id.as_uuid()), name);
    }

    /// A sync of `backend` for the athlete landed: re-arm the notice so the
    /// next failure is told. Best-effort.
    pub async fn sync_landed(&self, user_id: Uuid, tenant_id: TenantId, backend: &str) {
        if let Err(e) = self
            .connections
            .rearm_sync_failure_notification(user_id, tenant_id, backend)
            .await
        {
            warn!(%user_id, provider = backend, error = %e, "Failed to re-arm the sync-failure notice");
        }
    }
}

/// Whether a failed health sync is one the athlete is told about.
///
/// Expired credentials are the disconnect notice's: the refresh that found
/// them flips the connection and tells the athlete to reconnect. A rate limit
/// is the shared app's quota at the provider, which the next cycle retries on
/// its own and the athlete can do nothing about. Every other failure is one.
#[cfg(feature = "health-sync")]
#[must_use]
pub const fn health_sync_failure_is_told(error: &EnformeError) -> bool {
    !matches!(
        error,
        EnformeError::CredentialsExpired { .. } | EnformeError::RateLimited { .. }
    )
}
