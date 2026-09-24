// ABOUTME: The one-time push telling a user a provider connection needs reconnecting
// ABOUTME: Deduped per active-to-needs_reauth transition; a no-op without the notification backend
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#[cfg(feature = "client-notifications")]
use pierre_core::models::NotificationScreen;
use pierre_core::models::TenantId;
#[cfg(feature = "client-notifications")]
use pierre_notifications::models::NotificationCategory;
#[cfg(feature = "client-notifications")]
use pierre_notifications::{DispatchRequest, PushTier, TenantId as CommTenantId};
#[cfg(feature = "client-notifications")]
use pierre_providers::backend_resolver::{brand_name, user_facing_name};
#[cfg(feature = "client-notifications")]
use serde_json::Value as JsonValue;
#[cfg(not(feature = "client-notifications"))]
use std::future::ready;
#[cfg(feature = "client-notifications")]
use tracing::warn;
use uuid::Uuid;

use crate::protocol::auth::AuthService;

impl AuthService {
    /// Send a one-time out-of-band push telling the user a provider disconnected and to
    /// reconnect.
    ///
    /// Deduped via the `notified_at` claim so a user is nudged exactly once per
    /// `active`→`needs_reauth` transition (re-armed on reconnect). Best-effort; cfg-gated on
    /// `client-notifications` so builds without the notification backend compile to a no-op.
    #[cfg(feature = "client-notifications")]
    pub(crate) async fn notify_provider_disconnected(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) {
        let claimed = self
            .runtime()
            .repos()
            .provider_connections
            .claim_reauth_notification(user_id, tenant_id, provider)
            .await
            .unwrap_or(false);
        if !claimed {
            return;
        }
        let Some(service) = self.runtime().notification_service() else {
            return;
        };
        let data = [
            ("type", "provider_needs_reauth"),
            ("provider", provider),
            // Named through the shared vocabulary, not as a loose string: the
            // clients resolve `data.screen` to a surface through the same
            // enum, so a token nothing routes cannot be emitted here.
            ("screen", NotificationScreen::Connections.as_str()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), JsonValue::String(v.to_owned())))
        .collect();
        // The body names the provider as the user knows it: a mirror
        // backend's slug (`sciotte_trainingpeaks`) is internal.
        let brand = brand_name(self.runtime().provider_registry(), provider)
            .unwrap_or_else(|| user_facing_name(provider));
        let request = DispatchRequest {
            user_id,
            tenant_id: CommTenantId(tenant_id.as_uuid()),
            category: NotificationCategory::System,
            notification_type: "provider_needs_reauth".to_owned(),
            title: "Reconnect needed".to_owned(),
            body: format!(
                "Your {brand} connection expired. Reconnect it so I can keep using your data."
            ),
            data: Some(JsonValue::Object(data)),
            image_url: None,
            actions: None,
            bypass_frequency_cap: false,
        };
        // P1: an expired connection blocks every downstream feature, so it
        // outranks advisories — but it is recoverable, not break-glass P0.
        if let Err(e) = service.dispatch_with_tier(&request, PushTier::P1).await {
            warn!("Failed to dispatch reauth push for user {user_id} provider {provider}: {e}");
        }
    }

    /// No-op variant when the notification backend is not compiled in.
    #[cfg(not(feature = "client-notifications"))]
    pub(crate) async fn notify_provider_disconnected(
        &self,
        _user_id: Uuid,
        _tenant_id: TenantId,
        _provider: &str,
    ) {
        // Nothing to dispatch without a backend; await a ready future so the
        // signature stays `async` for the unconditional caller.
        ready(()).await;
    }
}
