// ABOUTME: Resolves a tenant's messaging channel to an outbound adapter and its stored config
// ABOUTME: The seam every proactive sender (backfill, check-in, first-sync, group, commitments) shares

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use async_trait::async_trait;
use dravr_canot::channel::MessagingChannel;
use dravr_canot::factory::create_adapter_from_config;
use pierre_core::models::messaging::{ChannelConfig, ChannelType};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_database::RepositoryRegistry;
use tracing::warn;

/// Resolves a `(tenant, channel)` to an outbound adapter + its config.
///
/// Extracted as a seam so the routing logic in
/// `ServerBackfillNotifier::push_backfill_complete` can be exercised against a
/// fake adapter that captures the [`OutgoingMessage`](pierre_core::models::messaging::OutgoingMessage) without touching a channel
/// API. The production [`ConfigAdapterResolver`] loads the tenant's stored
/// channel config and builds the real adapter exactly like the approval
/// notifier does.
#[async_trait]
pub trait AdapterResolver: Send + Sync {
    /// Resolve the channel adapter and its config for an outbound send, or
    /// `None` (logged) when the channel is unconfigured or undeserializable.
    async fn resolve(
        &self,
        tenant_id: TenantId,
        channel_str: &str,
        channel_type: ChannelType,
    ) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)>;
}

/// Production [`AdapterResolver`]: loads the tenant's stored channel config and
/// builds the real channel adapter from it.
///
/// Shared with the commitment reporter, which needs the same
/// `(tenant, channel) -> adapter + config` resolution and the same fake-adapter
/// test seam. Reach it through [`config_adapter_resolver`].
pub(crate) struct ConfigAdapterResolver {
    /// Shared repository registry — the channel-config lookup goes through
    /// `repos.messaging`. Arc so the resolver is cheap to share with the
    /// notifier and any future caller.
    pub(crate) repos: Arc<RepositoryRegistry>,
}

/// Build the production adapter resolver.
pub(crate) fn config_adapter_resolver(repos: Arc<RepositoryRegistry>) -> Arc<dyn AdapterResolver> {
    Arc::new(ConfigAdapterResolver { repos })
}

#[async_trait]
impl AdapterResolver for ConfigAdapterResolver {
    async fn resolve(
        &self,
        tenant_id: TenantId,
        channel_str: &str,
        channel_type: ChannelType,
    ) -> Option<(Arc<dyn MessagingChannel>, ChannelConfig)> {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        let raw_config = match db.get_channel_config(tenant_id, channel_str).await {
            Ok(Some(cfg)) => cfg,
            Ok(None) => {
                warn!(channel = %channel_str, "No channel config for outbound notice");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "Failed to load channel config for outbound notice");
                return None;
            }
        };
        let adapter = create_adapter_from_config(channel_type, &raw_config)
            .inspect_err(|e| {
                warn!(error = %e, channel = %channel_str, "Failed to build adapter for outbound notice");
            })
            .ok()?;
        let channel_config: ChannelConfig = serde_json::from_value(raw_config)
            .inspect_err(|e| {
                warn!(error = %e, "Failed to deserialize channel config for outbound notice");
            })
            .ok()?;
        Some((adapter, channel_config))
    }
}
