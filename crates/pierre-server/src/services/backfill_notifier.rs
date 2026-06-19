// ABOUTME: ServerBackfillNotifier — the concrete BackfillNotifier wired in the binary:
// ABOUTME: pushes a localized "your history is ready" notice back to the channel that triggered a backfill.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Binary-side implementation of [`BackfillNotifier`].
//!
//! A deep-history `get_activities` ask on a scrape-backed mirror provider warms
//! the durable activity cache off the request path (see
//! `pierre_tool_runtime::activity_backfill`) and returns silently. When the
//! originating turn came from a messaging channel, this notifier resolves that
//! channel from the Pierre conversation id and pings the user that their older
//! history is now loaded.
//!
//! Built once from the assembled [`ServerContext`] handles (repos +
//! messaging-strings registry) and stored on the context behind the
//! [`BackfillNotifier`] trait, so the detached backfill task can reach it
//! through [`pierre_tool_runtime::runtime::ToolRuntime::backfill_notifier`].
//!
//! Every step is best-effort — a missing session, an unconfigured channel, or a
//! send failure is logged and swallowed, never propagated, so a notification
//! can't fail (or block) the backfill itself.
//!
//! ## Staleness guard
//!
//! The session is recovered by reverse-looking-up the Pierre conversation id.
//! After a `/reset` the session is repointed at a fresh conversation, so the
//! lookup for the *old* conversation id returns `None` and the notice is
//! dropped — the user has moved on and a stale "your history is ready" ping
//! against an archived thread would be noise.

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, DEFAULT_LOCALE, KEY_BACKFILL_READY,
};
use pierre_core::models::messaging::{ChannelConfig, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_database::RepositoryRegistry;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use pierre_messaging::turn::ConversationTurnId;
use pierre_tool_runtime::runtime::BackfillNotifier;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

/// Resolves a `(tenant, channel)` to an outbound adapter + its config.
///
/// Extracted as a seam so the routing logic in
/// [`ServerBackfillNotifier::push_backfill_complete`] can be exercised against a
/// fake adapter that captures the [`OutgoingMessage`] without touching a channel
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
struct ConfigAdapterResolver {
    /// Shared repository registry — the channel-config lookup goes through
    /// `repos.messaging`. Arc so the resolver is cheap to share with the
    /// notifier and any future caller.
    repos: Arc<RepositoryRegistry>,
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
                warn!(channel = %channel_str, "No channel config for backfill-ready notice");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "Failed to load channel config for backfill-ready notice");
                return None;
            }
        };
        let adapter = create_adapter_from_config(channel_type, &raw_config)
            .inspect_err(|e| {
                warn!(error = %e, channel = %channel_str, "Failed to build adapter for backfill-ready notice");
            })
            .ok()?;
        let channel_config: ChannelConfig = serde_json::from_value(raw_config)
            .inspect_err(|e| {
                warn!(error = %e, "Failed to deserialize channel config for backfill-ready notice");
            })
            .ok()?;
        Some((adapter, channel_config))
    }
}

/// Backfill-completion notifier: pushes a localized "your history is ready"
/// notice back to the exact channel conversation that triggered the backfill.
pub struct ServerBackfillNotifier {
    /// Shared repository registry — the session reverse-lookup goes through
    /// `repos.messaging`. Arc so the notifier can be stored behind the
    /// `BackfillNotifier` trait on the shared `ServerContext`.
    repos: Arc<RepositoryRegistry>,
    /// Hot-reloadable user-facing string registry for the localized body.
    strings: Arc<MessagingStringsRegistry>,
    /// Adapter resolver (config-driven in production, faked in tests).
    resolver: Arc<dyn AdapterResolver>,
}

impl ServerBackfillNotifier {
    /// Build the production notifier from the shared repository registry and the
    /// messaging-strings registry. The adapter resolver loads each tenant's
    /// stored channel config on demand.
    #[must_use]
    pub fn from_handles(
        repos: Arc<RepositoryRegistry>,
        strings: Arc<MessagingStringsRegistry>,
    ) -> Arc<dyn BackfillNotifier> {
        let resolver: Arc<dyn AdapterResolver> = Arc::new(ConfigAdapterResolver {
            repos: repos.clone(),
        });
        Arc::new(Self {
            repos,
            strings,
            resolver,
        })
    }

    /// Build a notifier with an explicit adapter resolver. Test seam so the
    /// resolve + route logic can run against a fake adapter that captures the
    /// outbound message instead of hitting a channel API.
    #[must_use]
    pub fn with_resolver(
        repos: Arc<RepositoryRegistry>,
        strings: Arc<MessagingStringsRegistry>,
        resolver: Arc<dyn AdapterResolver>,
    ) -> Self {
        Self {
            repos,
            strings,
            resolver,
        }
    }

    /// Resolve the originating channel for a Pierre conversation id and build the
    /// outbound notice, or `None` when the conversation has moved/gone (the
    /// staleness guard) or the row is missing channel routing.
    ///
    /// Returns the `(channel_str, channel_type, outgoing)` triple so a caller can
    /// resolve the adapter and send. Kept separate from the send so the routing
    /// decision is unit-testable without a channel adapter.
    async fn build_notice(
        &self,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
        activity_count: usize,
    ) -> Option<(String, ChannelType, OutgoingMessage)> {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        let session = match db
            .get_session_by_pierre_conversation_id(tenant_id, pierre_conversation_id)
            .await
        {
            Ok(Some(session)) => session,
            Ok(None) => {
                // Inherent staleness guard: after a `/reset` the session no
                // longer points at this conversation id, so the lookup misses
                // and we drop the notice rather than ping an archived thread.
                info!("Backfill push skipped: session moved or gone (user likely /reset)");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "Backfill push: session lookup failed");
                return None;
            }
        };

        let channel_str = session.get("channel_type").and_then(Value::as_str)?;
        // Route to the EXACT originating chat (DM or group), never a broadcast
        // by user_id: the channel-native conversation id is the recipient.
        let recipient = session
            .get("channel_conversation_id")
            .and_then(Value::as_str)?;
        let locale = session
            .get("locale")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_LOCALE);

        let Ok(channel_type) = ChannelType::from_str(channel_str) else {
            warn!(channel = %channel_str, "Unknown channel type for backfill-ready notice");
            return None;
        };

        let count = activity_count.to_string();
        let body = self.strings.render(KEY_BACKFILL_READY, locale, &[&count]);
        let outgoing = OutgoingMessage {
            channel_type,
            recipient_id: recipient.to_owned(),
            content: MessageContent::Text { body },
            // Fresh turn — this is a proactive push, not a reply to the
            // originating turn.
            turn_id: ConversationTurnId::new(),
            reply_to: None,
            thread_id: None,
        };

        Some((channel_str.to_owned(), channel_type, outgoing))
    }
}

#[async_trait]
impl BackfillNotifier for ServerBackfillNotifier {
    async fn push_backfill_complete(
        &self,
        _user_id: Uuid,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
        _provider: &str,
        activity_count: usize,
    ) {
        let Some((channel_str, channel_type, outgoing)) = self
            .build_notice(tenant_id, pierre_conversation_id, activity_count)
            .await
        else {
            return;
        };

        let Some((adapter, channel_config)) = self
            .resolver
            .resolve(tenant_id, &channel_str, channel_type)
            .await
        else {
            return;
        };

        if let Err(e) = adapter.send(&outgoing, &channel_config).await {
            warn!(error = %e, channel = %channel_str, "Failed to send backfill-ready notice on channel");
        } else {
            info!(
                channel = %channel_str,
                count = activity_count,
                "Sent backfill-ready notice on channel"
            );
        }
    }
}
