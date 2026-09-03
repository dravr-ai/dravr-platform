// ABOUTME: The two ways a backfill-completion notice reaches an athlete — channel adapter, or in-app turn
// ABOUTME: Routing and body composition stay in backfill_notifier; this module only puts the notice where it goes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Delivery for [`ServerBackfillNotifier`](super::backfill_notifier).
//!
//! A messaging conversation is written to by handing an outgoing message to
//! that channel's adapter; a first-party (web/mobile) conversation is written
//! to by persisting a turn into the thread the client already reads. The
//! notifier decides which applies and what the notice says — these two types
//! only carry it there, so the decision and the mechanics stay separable.

use std::iter::once;

use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_BACKFILL_PUSH_BODY, KEY_BACKFILL_PUSH_TITLE,
};
use pierre_core::models::messaging::{ChannelConfig, ChannelType, OutgoingMessage};
use pierre_core::models::{AddMessageParams, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_messaging::channel::MessagingChannel;
use pierre_services::messaging_broadcast::proactive_text;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::services::backfill_notifier::AdapterResolver;
use crate::services::messaging_ingress::block_render::{channel_ceiling, fan_out};
use crate::services::messaging_ingress::outbound_retry::{enqueue_failed_outbound, FailedOutbound};

#[cfg(feature = "client-notifications")]
use pierre_notifications::{
    models::NotificationCategory as CommNotifCategory, DispatchRequest, NotificationService,
    PushTier, TenantId as CommTenantId,
};
#[cfg(feature = "client-notifications")]
use std::sync::Arc;

/// Routing pieces resolved for a completed backfill: where to deliver the
/// notice plus which tenant owns the channel config/adapter.
pub struct ResolvedRoute {
    /// Messaging-session id (the `id` column of `messaging_sessions`). Carried so
    /// a send failure can persist the dropped notice as an outbound message row
    /// under this session before queuing it for retry — exactly as the
    /// synchronous reply path does.
    pub session_id: String,
    /// Channel slug (e.g. `"telegram"`) for config load + logging.
    pub channel_str: String,
    /// Parsed channel type for adapter selection + the outgoing message.
    pub channel_type: ChannelType,
    /// Channel-native conversation id the notice routes to (the exact chat).
    pub recipient: String,
    /// Channel-native user id, the key for the per-channel locale override.
    pub channel_user_id: String,
    /// Tenant that owns the channel config + outbound adapter — the
    /// BOT/channel-owner tenant. Differs from the session's own tenant for a
    /// cross-tenant bot (a user DMs an admin-owned bot); equals it for a
    /// single-tenant self-host. Used ONLY for the config load + send; the
    /// session lookup, warmed-cache read, and chat re-entry all stay on the
    /// user's own tenant.
    pub channel_tenant_id: TenantId,
}

/// Sends a completion notice out through a messaging channel's adapter.
pub struct ChannelDelivery<'a> {
    /// Repository registry — the retry queue for a dropped notice.
    pub repos: &'a RepositoryRegistry,
    /// Resolves `(tenant, channel)` to an adapter plus its config.
    pub resolver: &'a dyn AdapterResolver,
}

impl ChannelDelivery<'_> {
    /// Hand the notice to the originating channel's adapter.
    ///
    /// A warmed history can be long — a coach answer over a deep window, or the
    /// templated list itself. The channel accepts a bounded message and rejects
    /// anything past it, so the notice goes out as ordered parts, each inside
    /// the ceiling, rather than being dropped whole. Every part is a fresh
    /// proactive turn, not a reply to the originating one.
    pub async fn send(
        &self,
        route: ResolvedRoute,
        user_id: Uuid,
        tenant_id: TenantId,
        body: String,
        count: usize,
    ) {
        let ResolvedRoute {
            session_id,
            channel_str,
            channel_type,
            recipient,
            channel_user_id: _,
            channel_tenant_id,
        } = route;

        let parts = fan_out(
            proactive_text(channel_type, recipient, body),
            channel_ceiling(channel_type),
        );

        // Load the channel config + adapter from the BOT/channel-owner tenant
        // (resolved via the channel link), NOT the user's own tenant — the
        // Telegram bot config lives under the bot tenant for a cross-tenant bot.
        let Some((adapter, channel_config)) = self
            .resolver
            .resolve(channel_tenant_id, &channel_str, channel_type)
            .await
        else {
            return;
        };

        let push_user_id = user_id.to_string();
        let failed = FailedOutbound {
            message_tenant_id: tenant_id,
            queue_tenant_id: channel_tenant_id,
            session_id: &session_id,
            user_id: Some(&push_user_id),
            channel: &channel_str,
        };
        if self
            .send_parts(parts, adapter.as_ref(), &channel_config, &failed)
            .await
        {
            info!(
                channel = %channel_str,
                count,
                "Sent backfill-ready notice on channel"
            );
        }
    }

    /// Send the parts in order, stopping at the first failure. Returns whether
    /// every part went out.
    ///
    /// Path parity with the synchronous reply: a failed channel send (e.g. Meta
    /// `WhatsApp` error 131047, out-of-24h-window/template-required) is
    /// persisted + queued for the background retry worker, never silently
    /// dropped. The message row lands on the user/session tenant, the queue row
    /// on the bot/channel-owner tenant so the worker resolves the right channel
    /// config on re-send. The parts after the failure are queued too: sending
    /// them now would put a continuation in front of its own opening.
    async fn send_parts(
        &self,
        parts: Vec<OutgoingMessage>,
        adapter: &dyn MessagingChannel,
        channel_config: &ChannelConfig,
        failed: &FailedOutbound<'_>,
    ) -> bool {
        let mut remaining = parts.into_iter();
        while let Some(outgoing) = remaining.next() {
            let Err(e) = adapter.send(&outgoing, channel_config).await else {
                continue;
            };
            warn!(error = %e, channel = %failed.channel, "Failed to send backfill-ready notice on channel");
            for dropped in once(outgoing).chain(remaining.by_ref()) {
                if let Err(enqueue_err) = enqueue_failed_outbound(
                    self.repos.messaging.as_ref(),
                    adapter,
                    &dropped,
                    failed,
                )
                .await
                {
                    error!(error = %enqueue_err, "Backfill push: failed to enqueue dropped notice for retry");
                }
            }
            return false;
        }
        true
    }
}

/// Writes a completion notice into a first-party conversation.
pub struct InAppDelivery<'a> {
    /// Repository registry — the conversation the notice is persisted into.
    pub repos: &'a RepositoryRegistry,
    /// Localized strings for the app-notification title and body.
    pub strings: &'a MessagingStringsRegistry,
    /// App-push service, or `None` when push is not configured. The persisted
    /// turn is the delivery either way.
    #[cfg(feature = "client-notifications")]
    pub notifications: Option<&'a Arc<NotificationService>>,
}

impl InAppDelivery<'_> {
    /// Deliver the notice into a first-party (web/mobile) conversation.
    ///
    /// There is no adapter to hand it to: those clients read a thread by
    /// fetching its messages, so persisting an assistant turn IS the delivery,
    /// and the participant read marker turns it into an unread badge on the
    /// conversation list. The app push that follows is the ping — it tells an
    /// athlete who closed the app that the answer landed, and a suppressed or
    /// unconfigured push therefore costs the notification, never the data.
    ///
    /// Written through `add_message` rather than the pipeline's
    /// `persist_assistant_response`, which does two extra things a proactive
    /// notice must not inherit. It advances the participant's read marker past
    /// the message — right for a reply the athlete is sitting in front of,
    /// wrong here, since it would mark the notice read before anyone saw it and
    /// take the unread badge with it. And it fans the reply out to a bound
    /// group's shared transcript, where one athlete's own activity history is
    /// not what the room asked for.
    pub async fn deliver(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        locale: &str,
        body: String,
        count: usize,
    ) {
        let owner = user_id.to_string();
        let params = AddMessageParams {
            tenant_id,
            conversation_id,
            user_id: &owner,
            role: "assistant",
            content: &body,
            // Platform-authored, like a command reply: no model answered and no
            // tokens were spent, so nothing is attributed to one.
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        };
        if let Err(e) = self.repos.chat.add_message(&params).await {
            warn!(error = %e, "Backfill push: failed to persist the in-app completion turn");
            return;
        }
        info!(
            count,
            "Delivered backfill-ready notice into the in-app conversation"
        );
        self.push_app_notification(user_id, tenant_id, locale, count)
            .await;
    }

    /// Raise the app notification that points at the turn just persisted.
    ///
    /// The body is a short count line, not the activity list: the list is in
    /// the thread, and a fifteen-line notification is unreadable on a lock
    /// screen. The frequency cap is deliberately NOT bypassed — the data is
    /// already delivered, so a capped or quiet-hours push loses only the ping.
    #[cfg(feature = "client-notifications")]
    async fn push_app_notification(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        locale: &str,
        count: usize,
    ) {
        let Some(service) = self.notifications else {
            return;
        };
        let rendered_count = count.to_string();
        let request = DispatchRequest {
            user_id,
            tenant_id: CommTenantId(tenant_id.as_uuid()),
            category: CommNotifCategory::Training,
            notification_type: "backfill_complete".to_owned(),
            title: self.strings.get(KEY_BACKFILL_PUSH_TITLE, locale),
            body: self
                .strings
                .render(KEY_BACKFILL_PUSH_BODY, locale, &[&rendered_count]),
            data: None,
            image_url: None,
            actions: None,
            bypass_frequency_cap: false,
        };
        // P2: the athlete asked a question and is waiting on the answer, which
        // is more than the ambient sync confirmation P3 describes, and less
        // than the break-glass tiers above.
        match service.dispatch_with_tier(&request, PushTier::P2).await {
            Ok(outcome) => {
                info!(?outcome, "Backfill push: in-app notification dispatched");
            }
            Err(e) => {
                warn!(error = %e, "Backfill push: in-app notification failed");
            }
        }
    }

    /// App push is compiled out of this build. The completion turn is already
    /// in the conversation, so the athlete finds it on their next visit.
    #[cfg(not(feature = "client-notifications"))]
    async fn push_app_notification(
        &self,
        _user_id: Uuid,
        _tenant_id: TenantId,
        _locale: &str,
        _count: usize,
    ) {
    }
}
