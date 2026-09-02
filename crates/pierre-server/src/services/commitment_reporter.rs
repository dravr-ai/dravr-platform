// ABOUTME: ServerCommitmentReporter — delivers a swept commitment verdict back to the athlete
// ABOUTME: Per-channel proactive policy: free-form channels send now, window channels wait, else app push
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Binary-side implementation of [`CommitmentReporter`].
//!
//! The sweep in `pierre_services::commitment_sweep` decides *what* the verdict
//! is; this decides whether it can be said, where, and in which language. It
//! lives in `pierre-server` because that is where both delivery rails are: the
//! messaging adapters and the app-push stack. `pierre-services` can reach
//! neither.
//!
//! ## Route
//!
//! The promise was made in a conversation, so the verdict goes back to it. The
//! Pierre conversation id reverse-resolves to the messaging session that owns
//! it, which yields the channel, the exact chat to address, and when the
//! athlete last spoke. When there is no such session — a web-chat promise, or a
//! thread the athlete has since `/reset` — the verdict falls back to app push.
//!
//! ## Why a per-channel policy exists at all
//!
//! Proactive messaging is not uniformly available. Telegram, Slack and Discord
//! bots may message a user who has started them, whenever. `WhatsApp` and
//! Messenger both close a re-engagement window 24 hours after the user's last
//! inbound message, and outside it Meta rejects a plain text send outright
//! (`WhatsApp` error 131047) — delivering there requires a pre-approved template,
//! which the platform's channel adapters do not implement.
//!
//! So on those two channels the verdict is *held*, not forced and not dropped.
//! The sweep retries every tick and the window reopens the moment the athlete
//! next says anything; if nothing opens within the sweep's staleness horizon
//! the verdict ages out unsaid, which is the honest outcome. The outbound retry
//! queue is deliberately not used for this: its backoff is seconds against a
//! 24-hour window, so an enqueued out-of-window send is a guaranteed
//! dead-letter dressed up as a retry.
//!
//! ## What the athlete actually reads
//!
//! Composed from the verdict's numbers and the sanitized sport slug through the
//! localized string registry — never from the stored statement, and never from
//! anything a provider supplied. The sweep reads activity data, which is a
//! tainted source; an activity titled with an injection payload can move a
//! count, and moving a count is all it can ever do.

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, DEFAULT_LOCALE, KEY_COMMITMENT_ACTIVITY_ANY, KEY_COMMITMENT_MET,
    KEY_COMMITMENT_MISSED, KEY_COMMITMENT_PARTIAL, KEY_COMMITMENT_PUSH_TITLE,
};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_database::RepositoryRegistry;
use pierre_memory::commitments::{Commitment, CommitmentOutcome};
use pierre_services::commitment_sweep::CommitmentReporter;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::services::backfill_notifier::{config_adapter_resolver, AdapterResolver};
use crate::services::messaging_ingress::addressing::reply_recipient;
use pierre_services::messaging_broadcast::proactive_text;

#[cfg(feature = "client-notifications")]
use pierre_notifications::{
    models::NotificationCategory as CommNotifCategory, DispatchOutcome, DispatchRequest,
    NotificationService, PushTier, TenantId as CommTenantId,
};

/// Meta's re-engagement window for `WhatsApp` and Messenger. Outside it a plain
/// text send is rejected, so the verdict waits instead.
const REENGAGEMENT_WINDOW: Duration = Duration::hours(24);

/// Route label reported on the `commitment.reported` notify event when the
/// verdict went out as an app push rather than into a chat.
const PUSH_ROUTE: &str = "push";

/// Whether an unsolicited message may be sent on this channel right now.
///
/// `last_inbound` is the athlete's last message on the exact session the
/// promise was made in — per-session and not per-user, because one athlete can
/// hold a DM plus a group chat on the same channel and only one of them may be
/// open.
///
/// A missing or unparseable `last_inbound` reads as closed on the windowed
/// channels: silently failing a send is worse than waiting a tick.
#[must_use]
pub fn channel_allows_proactive(
    channel: ChannelType,
    last_inbound: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    match channel {
        // Bot-initiated messages are unrestricted for a user who started the bot.
        ChannelType::Telegram | ChannelType::Slack | ChannelType::Discord => true,
        // Meta's 24-hour customer-service window.
        ChannelType::WhatsApp | ChannelType::Messenger => {
            last_inbound.is_some_and(|at| now - at < REENGAGEMENT_WINDOW)
        }
    }
}

/// Where a verdict resolved to.
struct ChannelRoute {
    /// Channel slug for config load and the route label.
    channel_str: String,
    /// Parsed channel type for adapter selection.
    channel_type: ChannelType,
    /// Channel-native id of the exact chat to address.
    recipient: String,
    /// Channel-native user id, used for the locale override lookup.
    channel_user_id: String,
    /// The athlete's last inbound message on this session.
    last_inbound: Option<DateTime<Utc>>,
    /// Tenant that owns the channel config and the outbound adapter — the
    /// bot/channel-owner tenant, which differs from the session tenant when a
    /// user DMs an admin-owned bot.
    channel_tenant_id: TenantId,
}

/// Delivers commitment verdicts back to the athlete.
pub struct ServerCommitmentReporter {
    /// Shared repository registry — session reverse-lookup, channel config,
    /// locale override and the user profile all go through it.
    repos: Arc<RepositoryRegistry>,
    /// Hot-reloadable localized strings for the verdict body.
    strings: Arc<MessagingStringsRegistry>,
    /// Adapter resolver (config-driven in production, faked in tests). Shared
    /// with the backfill notifier so both outbound paths build channels the
    /// same way.
    resolver: Arc<dyn AdapterResolver>,
    /// App-push fallback for athletes with no open channel session.
    #[cfg(feature = "client-notifications")]
    notifications: Option<Arc<NotificationService>>,
}

impl ServerCommitmentReporter {
    /// Build the production reporter.
    #[must_use]
    pub fn from_handles(
        repos: Arc<RepositoryRegistry>,
        strings: Arc<MessagingStringsRegistry>,
        #[cfg(feature = "client-notifications")] notifications: Option<Arc<NotificationService>>,
    ) -> Arc<dyn CommitmentReporter> {
        let resolver = config_adapter_resolver(repos.clone());
        Arc::new(Self {
            repos,
            strings,
            resolver,
            #[cfg(feature = "client-notifications")]
            notifications,
        })
    }

    /// Build a reporter with an explicit adapter resolver.
    ///
    /// Test seam: lets the route + policy + composition path run against a fake
    /// adapter that captures the outgoing message instead of calling a channel
    /// API.
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
            #[cfg(feature = "client-notifications")]
            notifications: None,
        }
    }

    /// Resolve the channel the promise was made in, if it is still reachable.
    async fn resolve_route(&self, commitment: &Commitment) -> Option<ChannelRoute> {
        let conversation_id = commitment.conversation_id.as_deref()?;
        let tenant_id = TenantId::parse_str(&commitment.tenant_id).ok()?;

        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        let session = match db
            .get_session_by_pierre_conversation_id(tenant_id, conversation_id)
            .await
        {
            Ok(Some(session)) => session,
            // No session: a web-chat promise, or the athlete reset the thread.
            // Either way the verdict goes to app push rather than nowhere.
            Ok(None) => return None,
            Err(e) => {
                warn!(error = %e, "commitment verdict: session lookup failed");
                return None;
            }
        };

        let channel_str = session.get("channel_type").and_then(Value::as_str)?;
        let channel_user_id = session.get("channel_user_id").and_then(Value::as_str)?;
        // Address the exact chat: a group's native conversation id, or the
        // channel-native user id for a DM (where it is NULL). Requiring the
        // conversation id is what once dropped every direct-message push.
        let recipient = reply_recipient(
            session
                .get("channel_conversation_id")
                .and_then(Value::as_str),
            channel_user_id,
        );
        let last_inbound = session
            .get("last_message_at")
            .and_then(Value::as_str)
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let Ok(channel_type) = ChannelType::from_str(channel_str) else {
            warn!(channel = %channel_str, "commitment verdict: unknown channel type");
            return None;
        };

        let channel_tenant_id = db
            .get_channel_link_tenant(channel_str, channel_user_id)
            .await
            .inspect_err(
                |e| warn!(error = %e, "commitment verdict: channel-link tenant lookup failed"),
            )
            .ok()
            .flatten()
            .unwrap_or(tenant_id);

        Some(ChannelRoute {
            channel_str: channel_str.to_owned(),
            channel_type,
            recipient: recipient.to_owned(),
            channel_user_id: channel_user_id.to_owned(),
            last_inbound,
            channel_tenant_id,
        })
    }

    /// Resolve the athlete's locale: per-channel override, then profile, then
    /// the default.
    ///
    /// Walks the chain against the repositories directly rather than reading a
    /// `locale` key off the session projection — that key is not in either
    /// backend's `SELECT`, so reading it there silently pins every message to
    /// the default locale forever.
    async fn resolve_locale(
        &self,
        commitment: &Commitment,
        route: Option<&ChannelRoute>,
    ) -> String {
        if let (Some(route), Ok(tenant_id)) = (route, TenantId::parse_str(&commitment.tenant_id)) {
            if let Ok(Some(override_locale)) = self
                .repos
                .messaging
                .get_channel_link_locale(tenant_id, &route.channel_str, &route.channel_user_id)
                .await
            {
                if !override_locale.trim().is_empty() {
                    return override_locale;
                }
            }
        }
        if let Ok(user_id) = Uuid::parse_str(&commitment.user_id) {
            if let Ok(Some(user)) = self.repos.users.get_global(user_id).await {
                if !user.locale.trim().is_empty() {
                    return user.locale;
                }
            }
        }
        DEFAULT_LOCALE.to_owned()
    }

    /// Render the verdict body from counts alone.
    fn compose(&self, commitment: &Commitment, locale: &str) -> String {
        let completed = commitment.completed_sessions.unwrap_or(0).to_string();
        let target = commitment.target_sessions.to_string();
        // The sport slug is bounded to `[a-z0-9_]` at write time; when the
        // commitment counts any activity there is no slug to name, so a
        // localized generic noun stands in.
        let activity = commitment
            .sport
            .clone()
            .unwrap_or_else(|| self.strings.get(KEY_COMMITMENT_ACTIVITY_ANY, locale));

        match commitment.outcome {
            Some(CommitmentOutcome::Met) => self.strings.render(
                KEY_COMMITMENT_MET,
                locale,
                &[&completed, &target, &activity],
            ),
            Some(CommitmentOutcome::Partial) => self.strings.render(
                KEY_COMMITMENT_PARTIAL,
                locale,
                &[&completed, &target, &activity],
            ),
            // A verdict with no outcome cannot happen — the sweep writes both in
            // one statement — but reading it as "missed" would accuse the
            // athlete on the strength of a bug.
            Some(CommitmentOutcome::Missed) | None => {
                self.strings
                    .render(KEY_COMMITMENT_MISSED, locale, &[&target, &activity])
            }
        }
    }

    /// Send into the originating chat. Returns the route label on success.
    async fn send_to_channel(&self, route: &ChannelRoute, body: String) -> Option<String> {
        let (adapter, config) = self
            .resolver
            .resolve(
                route.channel_tenant_id,
                &route.channel_str,
                route.channel_type,
            )
            .await?;
        let outgoing = proactive_text(route.channel_type, route.recipient.clone(), body);

        // A failed send is held, not queued. The outbound retry worker backs off
        // in seconds and dead-letters after three attempts, which cannot outlast
        // anything that would actually block a verdict; the hourly sweep is the
        // retry, and queueing as well would risk delivering twice.
        if let Err(e) = adapter.send(&outgoing, &config).await {
            warn!(channel = %route.channel_str, error = %e, "commitment verdict send failed; holding for the next sweep");
            return None;
        }
        info!(channel = %route.channel_str, "commitment verdict delivered to the originating chat");
        Some(route.channel_str.clone())
    }

    /// Send as an app push. Returns the route label on success.
    #[cfg(feature = "client-notifications")]
    async fn send_as_push(
        &self,
        commitment: &Commitment,
        locale: &str,
        body: String,
    ) -> Option<String> {
        let service = self.notifications.as_ref()?;
        let user_id = Uuid::parse_str(&commitment.user_id).ok()?;
        let tenant_uuid = Uuid::parse_str(&commitment.tenant_id).ok()?;

        let request = DispatchRequest {
            user_id,
            tenant_id: CommTenantId(tenant_uuid),
            category: CommNotifCategory::Coach,
            notification_type: "commitment_verdict".to_owned(),
            title: self.strings.get(KEY_COMMITMENT_PUSH_TITLE, locale),
            body,
            data: None,
            image_url: None,
            actions: None,
            // The sweep already caps itself at one verdict per athlete per day,
            // so the generic daily cap can only silently drop a message the
            // athlete asked to be held to. Category-disabled and quiet hours
            // still apply, and both are read below.
            bypass_frequency_cap: true,
        };

        // P2: the verdict is alert-shaped accountability the athlete opted
        // into, not break-glass — a Casual floor holds it for the digest.
        match service.dispatch_with_tier(&request, PushTier::P2).await {
            // No device tokens still lands the notification row, so it is
            // waiting in the app — that is a delivery, not a drop.
            Ok(DispatchOutcome::Delivered { .. } | DispatchOutcome::PersistedNoDevices { .. }) => {
                Some(PUSH_ROUTE.to_owned())
            }
            // The athlete muted the category, or it is the middle of their
            // night. Hold and let the next sweep try again rather than burning
            // the verdict on a message nobody saw.
            Ok(DispatchOutcome::Suppressed(reason)) => {
                info!(
                    ?reason,
                    "commitment verdict push suppressed; holding for the next sweep"
                );
                None
            }
            Err(e) => {
                warn!(error = %e, "commitment verdict push failed; holding for the next sweep");
                None
            }
        }
    }

    /// App push is compiled out of this build, so a verdict with no open chat
    /// route has nowhere to go and waits for one.
    #[cfg(not(feature = "client-notifications"))]
    async fn send_as_push(
        &self,
        _commitment: &Commitment,
        _locale: &str,
        _body: String,
    ) -> Option<String> {
        None
    }
}

#[async_trait]
impl CommitmentReporter for ServerCommitmentReporter {
    async fn report(&self, commitment: &Commitment) -> Option<String> {
        let route = self.resolve_route(commitment).await;
        let locale = self.resolve_locale(commitment, route.as_ref()).await;
        let body = self.compose(commitment, &locale);

        if let Some(route) = route {
            if channel_allows_proactive(route.channel_type, route.last_inbound, Utc::now()) {
                return self.send_to_channel(&route, body).await;
            }
            // The window is shut. Push is the athlete's own app, which has no
            // such restriction, so try it before giving up on the tick.
            info!(
                channel = %route.channel_str,
                "commitment verdict: re-engagement window closed; trying app push"
            );
        }
        self.send_as_push(commitment, &locale, body).await
    }
}
