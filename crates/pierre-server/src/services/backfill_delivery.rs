// ABOUTME: The two ways a proactive notice reaches an athlete — channel adapter, or in-app turn
// ABOUTME: Routing and body composition stay with the caller; this module only puts the notice where it goes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Delivery for [`ServerBackfillNotifier`](super::backfill_notifier) and for
//! the messaging copy of the coach's first post
//! ([`FirstSyncPoster`](super::first_sync_post::FirstSyncPoster)).
//!
//! A messaging conversation is written to by handing an outgoing message to
//! that channel's adapter; a first-party (web/mobile) conversation is written
//! to by persisting a turn into the thread the client already reads. The
//! caller decides which applies and what the notice says — these two types
//! only carry it there, so the decision and the mechanics stay separable.
//!
//! The caller also checks the channel's window before it hands a notice to
//! [`ChannelDelivery`] ([`ResolvedRoute::window_open`]), the check the
//! check-in and the commitment verdict pass: a notice the channel may not
//! carry unprompted goes to the app instead (carnet#1672).

use pierre_core::transport::TransportPolicy;
use std::iter::once;

use chrono::{DateTime, Utc};
use dravr_canot::channel::MessagingChannel;
use dravr_canot::turn::ConversationTurnId;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_BACKFILL_PUSH_BODY, KEY_BACKFILL_PUSH_TITLE,
};
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, MessageContent, OutgoingMessage,
};
use pierre_core::models::{AddMessageParams, TenantId};
use pierre_database::RepositoryRegistry;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::services::adapter_resolver::AdapterResolver;
use crate::services::commitment_reporter::channel_allows_proactive;
use crate::services::messaging_ingress::block_render::{channel_ceiling, fan_out, RenderedReply};
use crate::services::messaging_ingress::outbound_persist::{
    persist_outbound_row, OutboundRowParams,
};
use crate::services::messaging_ingress::outbound_retry::{
    enqueue_failed_outbound, FailedOutbound, TranscriptRecord,
};

#[cfg(feature = "client-notifications")]
use pierre_notifications::{
    events::push_policy, models::NotificationCategory as CommNotifCategory, DispatchRequest,
    NotificationService, PushTier, TenantId as CommTenantId,
};
#[cfg(feature = "client-notifications")]
use std::sync::Arc;

/// What `messaging.proactive_held` and a notice's ledger rows call a
/// backfill-completion notice: the answer to a question a background read
/// held up, or the one line saying its data landed.
pub(crate) const PURPOSE_BACKFILL_COMPLETE: &str = "backfill_complete";

/// What `messaging.proactive_held` and a notice's ledger rows call a
/// provider-reconnect notice.
pub(crate) const PURPOSE_PROVIDER_REAUTH: &str = "provider_reauth";

/// Whether `channel` takes a free-form send outside its window and fails it
/// only afterwards: `WhatsApp` answers HTTP 200 and reports error 131047 in a
/// status webhook. A proactive notice sent there leaves a ledger row per
/// message ([`NoticeLedger`]) so that webhook can find it.
pub(crate) fn reports_window_failure_late(channel: ChannelType) -> bool {
    matches!(channel, ChannelType::WhatsApp)
}

/// Routing pieces resolved for a proactive notice: where to deliver it plus
/// which tenant owns the channel config/adapter.
pub(crate) struct ResolvedRoute {
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
    /// The athlete's last inbound message on the session, which opens the
    /// channel's window for a message they did not ask for.
    pub last_inbound: Option<DateTime<Utc>>,
}

impl ResolvedRoute {
    /// Whether the channel may be written to unprompted on this route at
    /// `now` — [`channel_allows_proactive`], the check the check-in and the
    /// commitment verdict pass: Meta's 24-hour window on `WhatsApp` and
    /// Messenger, always on the bot channels.
    pub(crate) fn window_open(&self, now: DateTime<Utc>) -> bool {
        channel_allows_proactive(self.channel_type, self.last_inbound, now)
    }
}

/// Where the messages of one proactive notice a channel took are recorded:
/// one outbound `messaging_messages` row each, keyed by the channel's own id
/// for it and stamped with the notice's purpose. A delivery failure the
/// channel reports afterwards names only that id; the row leads it back to
/// the notice, its athlete and its thread.
pub(crate) struct NoticeLedger<'send> {
    /// Repository registry — the message log the rows land in.
    pub repos: &'send RepositoryRegistry,
    /// The session's own tenant, which the rows are filed under.
    pub session_tenant_id: TenantId,
    /// The messaging session the notice was sent on.
    pub session_id: &'send str,
    /// Channel slug for the rows.
    pub channel: &'send str,
    /// What the notice says.
    pub purpose: &'send str,
    /// The assistant chat row the notice is stored as in the conversation,
    /// when it is: a re-delivery then has nothing to write.
    pub chat_message_id: Option<&'send str>,
}

impl NoticeLedger<'_> {
    /// Record one message of the notice the channel took, under the id its
    /// receipt carries. Best-effort, logged by the shared insert.
    pub(crate) async fn record(&self, outgoing: &OutgoingMessage, receipt: &DeliveryReceipt) {
        persist_outbound_row(
            self.repos.messaging.as_ref(),
            &OutboundRowParams {
                session_tenant_id: self.session_tenant_id,
                session_id: self.session_id,
                channel: self.channel,
                receipt_id: receipt.channel_message_id.as_deref(),
                delivered: true,
                chat_message_id: self.chat_message_id,
                proactive_purpose: Some(self.purpose),
            },
            outgoing,
        )
        .await;
    }
}

/// Sends a proactive notice out through a messaging channel's adapter.
pub(crate) struct ChannelDelivery<'a> {
    /// Repository registry — the retry queue for a dropped notice, and the
    /// ledger for one a channel fails afterwards.
    pub repos: &'a RepositoryRegistry,
    /// Resolves `(tenant, channel)` to an adapter plus its config.
    pub resolver: &'a dyn AdapterResolver,
    /// What the notice says, as `messaging.proactive_held` and its ledger
    /// rows name it.
    pub purpose: &'a str,
}

impl ChannelDelivery<'_> {
    /// Hand the notice to the channel's adapter. Returns whether every part
    /// went out; a part that failed is queued for the retry worker, and one
    /// with nowhere to go is logged.
    ///
    /// A notice can be long — an agent answer over a deep window, or a
    /// templated list. The channel accepts a bounded message and rejects
    /// anything past it, so the notice goes out as ordered parts, each inside
    /// the ceiling, rather than being dropped whole. The prose lands first and
    /// the reply's attachments — the charts the agent drew — follow it, one
    /// message each, exactly as the live reply path orders them. Every part
    /// carries one fresh proactive turn id, not the originating turn's, so a
    /// split notice still reads as one turn in the transcript.
    ///
    /// `chat_message_id` is the assistant row the notice is already stored as
    /// in the conversation (an agent answer the platform post wrote), `None`
    /// for a templated notice the conversation does not hold.
    pub(crate) async fn send(
        &self,
        route: ResolvedRoute,
        user_id: Uuid,
        tenant_id: TenantId,
        reply: RenderedReply,
        chat_message_id: Option<&str>,
    ) -> bool {
        let ResolvedRoute {
            session_id,
            channel_str,
            channel_type,
            recipient,
            channel_user_id: _,
            channel_tenant_id,
            last_inbound: _,
        } = route;

        let turn_id = ConversationTurnId::new();
        let ceiling = channel_ceiling(channel_type);
        let outgoing = |content: MessageContent| OutgoingMessage {
            channel_type,
            recipient_id: recipient.clone(),
            content,
            turn_id,
            reply_to: None,
            thread_id: None,
        };
        let mut parts: Vec<OutgoingMessage> = reply
            .prose
            .into_iter()
            .flat_map(|body| fan_out(outgoing(MessageContent::Text { body }), ceiling))
            .collect();
        parts.extend(reply.attachments.into_iter().map(outgoing));
        if parts.is_empty() {
            warn!(
                channel = %channel_str,
                "Proactive push: the reply rendered to nothing for this channel; nothing sent"
            );
            return false;
        }

        // Load the channel config + adapter from the BOT/channel-owner tenant
        // (resolved via the channel link), NOT the user's own tenant — the
        // Telegram bot config lives under the bot tenant for a cross-tenant bot.
        let Some((adapter, channel_config)) = self
            .resolver
            .resolve(channel_tenant_id, &channel_str, channel_type)
            .await
        else {
            return false;
        };

        let push_user_id = user_id.to_string();
        let failed = FailedOutbound {
            transcript: Some(TranscriptRecord {
                tenant_id,
                session_id: &session_id,
            }),
            queue_tenant_id: channel_tenant_id,
            user_id: Some(&push_user_id),
            channel: &channel_str,
            expires_at: None,
            reauth: None,
        };
        let ledger = reports_window_failure_late(channel_type).then(|| NoticeLedger {
            repos: self.repos,
            session_tenant_id: tenant_id,
            session_id: &session_id,
            channel: &channel_str,
            purpose: self.purpose,
            chat_message_id,
        });
        self.send_parts(
            parts,
            adapter.as_ref(),
            &channel_config,
            &failed,
            ledger.as_ref(),
        )
        .await
    }

    /// Send the parts in order, stopping at the first failure. Returns whether
    /// every part went out.
    ///
    /// The window is checked before a notice gets here: the caller sends a
    /// proactive notice only on a route whose window is open
    /// ([`ResolvedRoute::window_open`]) and delivers it in the app otherwise.
    /// What fails here fails synchronously — a network error, a rate limit, a
    /// 5xx — and is persisted and queued for the background retry worker,
    /// never silently dropped, exactly as the synchronous reply does. The
    /// message row lands on the user/session tenant, the queue row on the
    /// bot/channel-owner tenant so the worker resolves the right channel
    /// config on re-send. The parts after the failure are queued too: sending
    /// them now would put a continuation in front of its own opening.
    ///
    /// `WhatsApp`'s out-of-window refusal (error 131047) never arrives here:
    /// Meta accepts the send with HTTP 200 and fails it afterwards in a status
    /// webhook, and a free-form re-send outside the window would fail the same
    /// way, so the retry queue is no answer to it. On such a channel each part
    /// the channel took is recorded in `ledger` instead, which is how that
    /// webhook finds the notice and re-delivers it in the app
    /// ([`crate::services::messaging_ingress::window_failure`]).
    async fn send_parts(
        &self,
        parts: Vec<OutgoingMessage>,
        adapter: &dyn MessagingChannel,
        channel_config: &ChannelConfig,
        failed: &FailedOutbound<'_>,
        ledger: Option<&NoticeLedger<'_>>,
    ) -> bool {
        let mut remaining = parts.into_iter();
        while let Some(outgoing) = remaining.next() {
            let e = match adapter.send(&outgoing, channel_config).await {
                Ok(receipt) => {
                    if let Some(ledger) = ledger {
                        ledger.record(&outgoing, &receipt).await;
                    }
                    continue;
                }
                Err(e) => e,
            };
            warn!(error = %e, channel = %failed.channel, "Failed to send a proactive notice on channel");
            for dropped in once(outgoing).chain(remaining.by_ref()) {
                if let Err(enqueue_err) = enqueue_failed_outbound(
                    self.repos.messaging.as_ref(),
                    adapter,
                    &dropped,
                    failed,
                )
                .await
                {
                    error!(error = %enqueue_err, "Proactive push: failed to enqueue dropped notice for retry");
                }
            }
            return false;
        }
        true
    }
}

/// Writes a completion notice into a first-party conversation.
pub(crate) struct InAppDelivery<'a> {
    /// Repository registry — the conversation the notice is persisted into.
    pub repos: &'a RepositoryRegistry,
    /// Localized strings for the app-notification title and body.
    pub strings: &'a MessagingStringsRegistry,
    /// App-push service, or `None` when push is not configured. The persisted
    /// turn is the delivery either way.
    #[cfg(feature = "client-notifications")]
    pub notifications: Option<&'a Arc<NotificationService>>,
    /// The stamp of the backfilled data the notice lists or counts
    /// (carnet#769), carried by the persisted turn and the app push alike.
    pub transport_policy: TransportPolicy,
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
    pub(crate) async fn deliver(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        locale: &str,
        body: String,
        count: usize,
    ) {
        if self
            .persist(user_id, tenant_id, conversation_id, body, count)
            .await
            .is_some()
        {
            self.push_app_notification(user_id, tenant_id, locale, count, self.transport_policy)
                .await;
        }
    }

    /// Persist the notice into a first-party conversation without the app
    /// push [`Self::deliver`] raises after it: for a notice that loaded no
    /// activities, which the push's "N activities loaded" would misstate.
    /// The stored turn and its unread badge are the delivery. Answers the
    /// stored turn's id; `None` (logged) when it could not be stored.
    pub(crate) async fn persist(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        body: String,
        count: usize,
    ) -> Option<String> {
        let transport_policy = self.transport_policy;
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
            transport_policy,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        };
        match self.repos.chat.add_message(&params).await {
            Ok(stored) => {
                info!(
                    count,
                    "Delivered a proactive notice into the in-app conversation"
                );
                Some(stored.id)
            }
            Err(e) => {
                warn!(error = %e, "Proactive notice: failed to persist the in-app turn");
                None
            }
        }
    }

    /// Raise only the app notification, for a notice the chat pipeline has
    /// already stored in the conversation as the agent's own answer.
    pub(crate) async fn ping(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        locale: &str,
        count: usize,
    ) {
        self.push_app_notification(user_id, tenant_id, locale, count, self.transport_policy)
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
        transport_policy: TransportPolicy,
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
            // The count is an aggregate of the backfilled provider's data.
            data: None,
            transport_policy: push_policy(transport_policy),
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
        _transport_policy: TransportPolicy,
    ) {
    }
}
