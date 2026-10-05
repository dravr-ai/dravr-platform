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
//! channel from the Pierre conversation id and pushes the user the ACTUAL list
//! of activities it just loaded — read straight back from the now-warm durable
//! cache via the [`pierre_database::repositories::ActivityCacheRepository`] the
//! notifier already holds, so there is no chat-pipeline re-entry and no LLM
//! call. If that read comes back empty (it shouldn't, post-backfill) it falls
//! back to a templated "your history is ready, ask again" nudge.
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
//!
//! ## What a model may read of the notice
//!
//! The list is persisted in the conversation the model reads back, so the
//! warmed window is read as a read for a model ([`ai_scope::ai_read`]): each
//! provider's terms decide which sessions, and which of their fields, the
//! notice may carry (carnet#734). The athlete still sees every session in the
//! app's own activity views.

use std::fmt::Write as _;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use dravr_canot::channel::MessagingChannel;
use dravr_canot::factory::create_adapter_from_config;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_BACKFILL_LIST_HEADER, KEY_BACKFILL_LIST_MORE, KEY_BACKFILL_READY,
    KEY_PROVIDER_REAUTH_REQUIRED, KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::civil_time::format_local_day;
use pierre_core::models::messaging::{ChannelConfig, ChannelType, OutgoingMessage};
use pierre_core::models::{is_in_app_channel, Activity, ConversationRecord, TenantId};
use pierre_core::transport::Transport;
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_database::backends::MessagingRepository;
use pierre_database::repositories::shorten_url;
use pierre_database::repositories::OutboundReauthGuard;
use pierre_database::RepositoryRegistry;
use pierre_middleware::provider_link_token::{
    mint_link_token, MintProviderLinkTokenArgs, PROVIDER_LINK_TOKEN_TTL_MINUTES,
};
use pierre_providers::backend_resolver;
use pierre_providers::registry::global_registry;
use pierre_services::athlete_clock::athlete_zone;
use pierre_services::locale::{resolve_channel_locale, resolve_user_locale};
use pierre_tool_runtime::implementations::sport_labels::localized_sport_name;
use pierre_tool_runtime::runtime::BackfillNotifier;
use serde_json::Value;
use tracing::{error, info, warn};
use uuid::Uuid;

#[cfg(feature = "client-notifications")]
use pierre_notifications::NotificationService;

use crate::services::backfill_delivery::{ChannelDelivery, InAppDelivery, ResolvedRoute};
use crate::services::backfill_reentry::{ChatReentry, ReentryReply, ReentryRequest};
use crate::services::backfill_window::read_warmed_window;
use crate::services::messaging_ingress::addressing::reply_recipient;
use crate::services::messaging_ingress::block_render::{render_reply, RenderedReply};
use crate::services::messaging_ingress::outbound_retry::{
    enqueue_failed_outbound, FailedOutbound, TranscriptRecord,
};
use crate::services::messaging_ingress::surface::messaging_render_profile;
use pierre_services::messaging_broadcast::{
    proactive_text, resolve_linked_targets, LinkedChannelTarget,
};

/// Max activities rendered inline in the completion notice; the rest collapse
/// into a "… and N more" footer so a deep backfill can't flood the channel.
const BACKFILL_LIST_MAX: usize = 15;

/// How many recent messages to scan when recovering the user's question for the
/// chat re-entry. The triggering question is the latest `user`-role turn; a
/// small lookback tolerates a trailing assistant/tool turn without an unbounded
/// read.
const REENTRY_HISTORY_LOOKBACK: i64 = 10;

/// How long a reconnect link must still have to run for a queued notice to be
/// sent with it. A retry that would hand the athlete a link with less time
/// than this left is given up instead: a link that dies before they read it
/// is worse than the app notice they already have.
const RECONNECT_LINK_MIN_REMAINING_MINUTES: i64 = 60;

/// A one-time hosted-login link, and the last instant a message carrying it
/// may still be sent.
struct ReconnectLink {
    /// The (shortened) link.
    url: String,
    /// The link's expiry less [`RECONNECT_LINK_MIN_REMAINING_MINUTES`].
    send_by: DateTime<Utc>,
}

/// A rendered reconnect notice, and the instant after which it must not be
/// sent because the link in it expires; `None` when it carries no link.
struct ReconnectNotice {
    /// The localized notice text.
    body: String,
    /// See [`ReconnectLink::send_by`].
    send_by: Option<DateTime<Utc>>,
}

/// Resolves a `(tenant, channel)` to an outbound adapter + its config.
///
/// Extracted as a seam so the routing logic in
/// [`ServerBackfillNotifier::push_backfill_complete`] can be exercised against a
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

/// Where a completed backfill's notice is delivered.
///
/// The two arms are different delivery mechanisms, not two channels: a
/// messaging conversation is written to by handing an outgoing message to that
/// channel's adapter, while a first-party conversation is written to by
/// persisting a turn into the thread the client already reads.
enum Destination {
    /// The conversation came in over a messaging app — send through its adapter.
    Channel(Box<ResolvedRoute>),
    /// A web or mobile conversation. These clients create no
    /// `messaging_sessions` row and no `ChannelType` names them, so every
    /// notice for one used to die in the session lookup.
    InApp,
}

/// Backfill-completion notifier: pushes a localized "your history is ready"
/// notice back to the exact conversation that triggered the backfill.
pub struct ServerBackfillNotifier {
    /// Shared repository registry — the session reverse-lookup goes through
    /// `repos.messaging`. Arc so the notifier can be stored behind the
    /// `BackfillNotifier` trait on the shared `ServerContext`.
    repos: Arc<RepositoryRegistry>,
    /// Hot-reloadable user-facing string registry for the localized body.
    strings: Arc<MessagingStringsRegistry>,
    /// Adapter resolver (config-driven in production, faked in tests).
    resolver: Arc<dyn AdapterResolver>,
    /// Chat-pipeline re-entry handle, shared (same `Arc`) with the context's
    /// `mcp.backfill_reentry` slot and installed post-`Arc` by
    /// [`install_backfill_reentry`]. Empty until then (and in tests that don't
    /// inject one) — the push then falls back to the templated activity list.
    reentry: Arc<OnceLock<Arc<dyn ChatReentry>>>,
    /// Admin JWT secret for signing the one-time hosted-login link in the
    /// provider-reauth nudge. Supplied from the server config via `from_handles`.
    admin_jwt_secret: Arc<str>,
    /// Server root URL for building the hosted-login link in the reauth nudge.
    base_url: String,
    /// Each provider's terms: what of the warmed window the notice, which a
    /// model reads back, may carry.
    terms: Arc<dyn ProviderTerms>,
    /// App-push service, used only for the in-app arm: the persisted turn is
    /// the delivery, this is the ping that tells the athlete it landed. `None`
    /// in tests and when push is not configured — the turn is written either
    /// way, so a missing service costs the notification, never the data.
    #[cfg(feature = "client-notifications")]
    notifications: Option<Arc<NotificationService>>,
}

impl ServerBackfillNotifier {
    /// Build the production notifier from the shared repository registry and the
    /// messaging-strings registry. The adapter resolver loads each tenant's
    /// stored channel config on demand.
    ///
    /// `reentry` is the slot shared with `ServerContext::mcp.backfill_reentry`;
    /// it is empty here (the pipeline-re-entry handle needs the composition-root
    /// `Arc`, installed later by [`install_backfill_reentry`]).
    #[must_use]
    pub fn from_handles(
        repos: Arc<RepositoryRegistry>,
        strings: Arc<MessagingStringsRegistry>,
        reentry: Arc<OnceLock<Arc<dyn ChatReentry>>>,
        admin_jwt_secret: Arc<str>,
        base_url: String,
        #[cfg(feature = "client-notifications")] notifications: Option<Arc<NotificationService>>,
    ) -> Arc<dyn BackfillNotifier> {
        let resolver = config_adapter_resolver(repos.clone());
        Arc::new(Self {
            repos,
            strings,
            resolver,
            reentry,
            admin_jwt_secret,
            base_url,
            terms: global_registry(),
            #[cfg(feature = "client-notifications")]
            notifications,
        })
    }

    /// Build a notifier with an explicit adapter resolver and no re-entry handle.
    /// Test seam so the resolve + route + templated-list path can run against a
    /// fake adapter that captures the outbound message instead of hitting a
    /// channel API.
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
            reentry: Arc::new(OnceLock::new()),
            admin_jwt_secret: Arc::from("test-jwt-secret"),
            base_url: "https://app.test".to_owned(),
            terms: global_registry(),
            #[cfg(feature = "client-notifications")]
            notifications: None,
        }
    }

    /// Build a notifier with an explicit adapter resolver and a pre-installed
    /// re-entry handle. Test seam for the agent-synthesis path: inject a fake
    /// [`ChatReentry`] and assert the notifier sends its reply instead of the
    /// templated list.
    #[must_use]
    pub fn with_resolver_and_reentry(
        repos: Arc<RepositoryRegistry>,
        strings: Arc<MessagingStringsRegistry>,
        resolver: Arc<dyn AdapterResolver>,
        reentry: Arc<dyn ChatReentry>,
    ) -> Self {
        let slot = Arc::new(OnceLock::new());
        // Infallible: a freshly-created slot is empty.
        let _ = slot.set(reentry);
        Self {
            repos,
            strings,
            resolver,
            reentry: slot,
            admin_jwt_secret: Arc::from("test-jwt-secret"),
            base_url: "https://app.test".to_owned(),
            terms: global_registry(),
            #[cfg(feature = "client-notifications")]
            notifications: None,
        }
    }

    /// A one-time hosted-login link for the scrape mirror `provider`, minted
    /// for `channel` and shortened to a dot-free `<base>/r/<code>` (the raw
    /// JWT's dots make `WhatsApp` truncate linkification mid-token; the full
    /// URL is kept when the shortener store write fails). Mirrors
    /// `auth_recovery::mint_reconnect_url`'s hosted-login arm.
    ///
    /// `None`, logged, when `provider` has no hosted login or the token cannot
    /// be minted.
    async fn hosted_login_url(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        channel: &str,
    ) -> Option<ReconnectLink> {
        let target = backend_resolver::hosted_login_target(provider)?;
        // Taken before the mint, so it is never later than the token's own
        // expiry; the short link is created after the token and outlives it.
        let send_by = Utc::now() + Duration::minutes(PROVIDER_LINK_TOKEN_TTL_MINUTES)
            - Duration::minutes(RECONNECT_LINK_MIN_REMAINING_MINUTES);
        let token = match mint_link_token(
            &MintProviderLinkTokenArgs {
                user_id,
                tenant_id: tenant_id.as_uuid(),
                provider: "sciotte",
                target,
                channel,
                channel_thread: None,
            },
            &self.admin_jwt_secret,
        ) {
            Ok(token) => token,
            Err(e) => {
                warn!(error = %e, provider = %provider, "Reconnect link: link-token mint failed");
                return None;
            }
        };
        let full_url = format!(
            "{}/providers/sciotte/login?token={}",
            self.base_url,
            urlencoding::encode(&token)
        );
        let url = shorten_url(
            self.repos.short_links.as_ref(),
            &self.base_url,
            &full_url,
            &tenant_id.as_uuid().to_string(),
            &user_id.to_string(),
        )
        .await;
        Some(ReconnectLink { url, send_by })
    }

    /// The reconnect notice for one linked chat on `channel`, in `locale`.
    ///
    /// A scrape mirror signs in again on the hosted login page, through a
    /// one-time link minted for this channel. An OAuth provider has no link
    /// this notice can carry: its authorization URL and the connect picker's
    /// token both expire within the hour, well before a notice read the next
    /// morning, and the sentence promises a day. It is named without a link,
    /// and reconnected from the app's settings — as is a mirror whose link
    /// could not be minted.
    async fn reauth_notice_body(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        channel: &str,
        locale: &str,
    ) -> ReconnectNotice {
        let display =
            backend_resolver::brand_name(&global_registry(), provider).unwrap_or(provider);
        let link = if backend_resolver::hosted_login_target(provider).is_some() {
            self.hosted_login_url(user_id, tenant_id, provider, channel)
                .await
        } else {
            None
        };
        link.map_or_else(
            || ReconnectNotice {
                body: self
                    .strings
                    .render(KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK, locale, &[display]),
                send_by: None,
            },
            |link| ReconnectNotice {
                body: self.strings.render(
                    KEY_PROVIDER_REAUTH_REQUIRED,
                    locale,
                    &[display, &link.url],
                ),
                send_by: Some(link.send_by),
            },
        )
    }

    /// Queue a reconnect notice whose send to a linked chat failed, for the
    /// outbound retry worker.
    ///
    /// Only this chat's notice is queued: the app notice and the chats that
    /// were reached are already done. A linked chat has no messaging session,
    /// so the entry carries no message row; it is keyed by the link's own
    /// tenant (whose bot holds the chat), its channel and the rendered payload,
    /// which already addresses the recipient. The worker re-sends it with
    /// backoff while `guard`'s connection is still `needs_reauth`, and never
    /// after `send_by`, when the link in it would be about to expire.
    async fn queue_linked_reauth(
        &self,
        adapter: &dyn MessagingChannel,
        outgoing: &OutgoingMessage,
        target: &LinkedChannelTarget,
        user_id: Uuid,
        guard: OutboundReauthGuard<'_>,
        send_by: Option<DateTime<Utc>>,
    ) {
        let channel_str = target.channel_type.to_string();
        let queued_user_id = user_id.to_string();
        if let Err(e) = enqueue_failed_outbound(
            self.repos.messaging.as_ref(),
            adapter,
            outgoing,
            &FailedOutbound {
                transcript: None,
                queue_tenant_id: target.tenant_id,
                user_id: Some(&queued_user_id),
                channel: &channel_str,
                expires_at: send_by,
                reauth: Some(guard),
            },
        )
        .await
        {
            error!(error = %e, channel = %channel_str, "Reconnect notice: failed to queue the linked channel send for retry");
        }
    }

    /// Resolve the originating channel for a Pierre conversation id, or `None`
    /// when the conversation has moved/gone (the staleness guard) or the row is
    /// missing channel routing.
    ///
    /// Returns a [`ResolvedRoute`] so the caller can read the warmed cache,
    /// compose the body, and send. Kept separate from body composition + send so
    /// the routing decision is unit-testable without a channel adapter.
    async fn resolve_route(
        &self,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
    ) -> Option<ResolvedRoute> {
        let session = self
            .lookup_session(tenant_id, pierre_conversation_id)
            .await?;

        // `id` is the `messaging_sessions.id` column (see
        // `get_session_by_pierre_conversation_id_impl`'s JSON projection) — the
        // session a retry-queued notice must be persisted under on a send failure.
        let session_id = session.get("id").and_then(Value::as_str)?;
        let channel_str = session.get("channel_type").and_then(Value::as_str)?;
        let channel_user_id = session.get("channel_user_id").and_then(Value::as_str)?;
        // Route to the EXACT originating chat. `channel_conversation_id` keys the
        // group/DM split: it holds a group's native conversation id but is NULL
        // for a DM (one DM per user — the lookup projects it as JSON null, and
        // `reply_recipient` also treats an empty string as absent). For a DM
        // the recipient is the channel-native user id (e.g. the WhatsApp phone),
        // exactly how the synchronous reply addresses a private reply
        // (messaging_ingress send_private_reply). Requiring the conversation id
        // dropped EVERY DM notice silently — the backfill push never reached a
        // single direct-message user.
        let recipient = reply_recipient(
            session
                .get("channel_conversation_id")
                .and_then(Value::as_str),
            channel_user_id,
        );
        let Ok(channel_type) = ChannelType::from_str(channel_str) else {
            warn!(channel = %channel_str, "Unknown channel type for backfill-ready notice");
            return None;
        };

        let channel_tenant_id = self
            .resolve_channel_owner_tenant(channel_str, channel_user_id, tenant_id)
            .await;

        // Observability: the route resolved, so the notice is about to send.
        // The prior silent `?`-drop on a NULL conversation id left zero trace
        // when every DM push died — log the success path so a future drop is
        // diagnosable from the absence of this line, not guesswork.
        // Neutral wording: this route resolver serves both the completion
        // notice and the reauth nudge.
        info!(
            channel = %channel_str,
            is_dm = recipient == channel_user_id,
            "Backfill push: routing notice to originating chat"
        );
        Some(ResolvedRoute {
            session_id: session_id.to_owned(),
            channel_str: channel_str.to_owned(),
            channel_type,
            recipient: recipient.to_owned(),
            channel_user_id: channel_user_id.to_owned(),
            channel_tenant_id,
        })
    }

    /// Decide how this conversation is reachable, or `None` when it is not.
    ///
    /// A messaging session wins: that conversation is read in the channel app,
    /// so the notice belongs there. Absent one the conversation's own
    /// `channel_type` separates the two remaining cases, which used to be
    /// indistinguishable and were both dropped: a first-party thread, which is
    /// read by fetching the conversation and so is delivered into, and a
    /// messaging thread whose session was repointed by a `/reset`, where a
    /// notice against the archived thread would be noise.
    async fn resolve_destination(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
    ) -> Option<Destination> {
        if let Some(route) = self.resolve_route(tenant_id, pierre_conversation_id).await {
            return Some(Destination::Channel(Box::new(route)));
        }

        let conversation = self
            .lookup_conversation(user_id, tenant_id, pierre_conversation_id)
            .await?;

        if !is_in_app_channel(&conversation.channel_type) {
            info!(
                channel = %conversation.channel_type,
                "Backfill push skipped: messaging session moved or gone (user likely /reset)"
            );
            return None;
        }
        info!("Backfill push: routing notice into the in-app conversation");
        Some(Destination::InApp)
    }

    /// Read the conversation the backfill was triggered from, or `None` when
    /// it is gone (deleted, or the athlete is no longer a participant) and
    /// there is no thread left to answer into.
    async fn lookup_conversation(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
    ) -> Option<ConversationRecord> {
        match self
            .repos
            .chat
            .get_conversation(pierre_conversation_id, &user_id.to_string(), tenant_id)
            .await
        {
            Ok(Some(conversation)) => Some(conversation),
            Ok(None) => {
                info!("Backfill push skipped: the conversation is gone");
                None
            }
            Err(e) => {
                warn!(error = %e, "Backfill push: conversation lookup failed");
                None
            }
        }
    }

    /// Resolve the athlete's locale: the per-channel override when the notice
    /// is going to a messaging channel, then their profile, then the default.
    ///
    /// Walked against the repositories rather than read off the session
    /// projection. `messaging_sessions` has no `locale` in either backend's
    /// `SELECT`, so the previous `session.get("locale")` was always `None` and
    /// pinned every completion notice and reconnect nudge to `DEFAULT_LOCALE`
    /// whatever the athlete reads in. The chain is the shared
    /// [`resolve_channel_locale`].
    async fn resolve_locale(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        route: Option<&ResolvedRoute>,
    ) -> String {
        match route {
            Some(route) => {
                resolve_channel_locale(
                    self.repos.messaging.as_ref(),
                    self.repos.users.as_ref(),
                    tenant_id,
                    &route.channel_str,
                    &route.channel_user_id,
                    Some(user_id),
                )
                .await
            }
            None => resolve_user_locale(self.repos.users.as_ref(), user_id).await,
        }
    }

    /// Reverse-look-up the messaging session by Pierre conversation id. Returns
    /// `None` — dropping the notice — when the session moved (after a `/reset` it
    /// no longer points at this conversation id, so we don't ping an archived
    /// thread) or the lookup errored.
    async fn lookup_session(
        &self,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
    ) -> Option<Value> {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        match db
            .get_session_by_pierre_conversation_id(tenant_id, pierre_conversation_id)
            .await
        {
            Ok(Some(session)) => Some(session),
            // Not a skip on its own: an in-app conversation never has a session
            // row. `resolve_destination` reads the conversation to tell that
            // from a messaging thread the athlete reset, and logs the verdict.
            Ok(None) => None,
            Err(e) => {
                warn!(error = %e, "Backfill push: session lookup failed");
                None
            }
        }
    }

    /// Resolve the tenant that owns the channel config + outbound adapter. When a
    /// user DMs an admin-owned bot this is the BOT/channel-owner tenant (from the
    /// channel link — the authoritative channel-identity -> tenant map),
    /// distinct from the session's own tenant (the user's, post the
    /// one-tenant-per-DM-unit fix). Absent a link (self-host where the user owns
    /// the bot), the session tenant owns the config, so fall back to it.
    async fn resolve_channel_owner_tenant(
        &self,
        channel_str: &str,
        channel_user_id: &str,
        session_tenant: TenantId,
    ) -> TenantId {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        db.get_channel_link_tenant(channel_str, channel_user_id)
            .await
            .inspect_err(|e| {
                warn!(error = %e, "Backfill push: channel-link tenant lookup failed");
            })
            .ok()
            .flatten()
            .unwrap_or(session_tenant)
    }

    /// Compose the completion-notice body: a localized header followed by a
    /// compact, Rust-rendered list of up to [`BACKFILL_LIST_MAX`] activities,
    /// then a localized "… and N more" footer when the window is larger.
    ///
    /// The header and footer are catalogue phrases; each activity line (name ·
    /// day · sport · distance) is dated on the athlete's calendar (`zone`) and
    /// names the sport in `locale`.
    fn render_list_body(&self, locale: &str, zone: Tz, activities: &[Activity]) -> String {
        let total = activities.len();
        let count = total.to_string();
        let mut body = self
            .strings
            .render(KEY_BACKFILL_LIST_HEADER, locale, &[&count]);

        for activity in activities.iter().take(BACKFILL_LIST_MAX) {
            body.push('\n');
            body.push_str(&format_activity_line(activity, locale, zone));
        }

        if total > BACKFILL_LIST_MAX {
            let remaining = (total - BACKFILL_LIST_MAX).to_string();
            body.push('\n');
            body.push_str(
                &self
                    .strings
                    .render(KEY_BACKFILL_LIST_MORE, locale, &[&remaining]),
            );
        }

        body
    }

    /// Fetch the user's most recent question in this conversation so the
    /// re-entry can re-ask it verbatim — carrying the same window (e.g. "2022")
    /// the agent must query against the now-warm cache.
    ///
    /// `get_recent_messages` selects the newest rows and hands them back in
    /// chronological order — the shape a prompt wants — so the question that
    /// triggered the backfill is the LAST `user` row, and the scan runs from
    /// the end. Read from the front it picks the oldest question in the
    /// lookback instead: on 2026-09-21 an athlete asked for monthly elevation
    /// totals and the completion push answered whether to ride that morning, a
    /// question from six hours earlier.
    ///
    /// A slash-command line is not a question — re-asking `/status` would answer
    /// a command nobody typed — so command rows are skipped. `None` when the
    /// read fails or there is no user turn — the caller falls back to the list.
    async fn recent_user_question(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
    ) -> Option<String> {
        let messages = self
            .repos
            .chat
            .get_recent_messages(
                conversation_id,
                &user_id.to_string(),
                tenant_id,
                REENTRY_HISTORY_LOOKBACK,
            )
            .await
            .inspect_err(|e| {
                warn!(error = %e, "Backfill push: recent-message read failed for re-entry");
            })
            .ok()?;
        messages
            .into_iter()
            .rev()
            .find(|m| m.role == "user" && !m.is_command_turn() && !m.content.trim().is_empty())
            .map(|m| m.content)
    }

    /// Try to synthesize a real agent answer by re-entering the chat pipeline
    /// with the user's own question, now that the activity cache is warm.
    ///
    /// Loop-safe: re-asking a deep window after the cache is warmed serves
    /// inline from the durable cache (`covered == true`) and never re-spawns a
    /// backfill, so the re-entry cannot retrigger another completion push.
    /// Returns `None` (caller falls back to the templated list) when no re-entry
    /// handle is installed, the conversation has no re-askable question, or the
    /// pipeline produced nothing.
    async fn try_synthesize_agent_reply(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        channel_type: ChannelType,
        locale: &str,
    ) -> Option<ReentryReply> {
        let reentry = self.reentry.get()?;
        let prompt = self
            .recent_user_question(user_id, tenant_id, conversation_id)
            .await?;
        reentry
            .synthesize_reply(ReentryRequest {
                user_id,
                tenant_id,
                conversation_id,
                channel_type,
                locale,
                prompt: &prompt,
            })
            .await
    }
}

/// Render one activity as a compact `• name · day · sport · 10.0 km` line.
/// Distance is shown in kilometres (one decimal) only when present. Pure
/// formatting over the cheaply-available `Activity` accessors — no allocation
/// beyond the returned line.
fn format_activity_line(activity: &Activity, locale: &str, zone: Tz) -> String {
    // The day on the athlete's calendar and the sport in their language, from
    // the same helpers the activity list the agent reads uses: the notice is
    // stored in the conversation the model reads back, so a UTC date or an
    // English sport here would contradict that list.
    let date = format_local_day(activity.start_date(), zone, locale);
    let sport = localized_sport_name(activity.sport_type(), locale);
    // The list is persisted in the conversation the model reads back, and the
    // name is whatever the provider account's writers typed.
    let name = display_line(activity.name(), ACTIVITY_NAME_MAX_CHARS);
    let mut line = format!("• {name} · {date} · {sport}");
    if let Some(meters) = activity.distance_meters() {
        if meters > 0.0 {
            // Infallible write into a String; the trait import is `as _` so the
            // method resolves without exposing `Write` to the rest of the file.
            let _ = write!(line, " · {:.1} km", meters / 1000.0);
        }
    }
    line
}

#[async_trait]
impl BackfillNotifier for ServerBackfillNotifier {
    async fn push_backfill_complete(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
        provider: &str,
        after_ts: i64,
        activity_count: usize,
    ) {
        let Some(destination) = self
            .resolve_destination(user_id, tenant_id, pierre_conversation_id)
            .await
        else {
            return;
        };
        let route = match &destination {
            Destination::Channel(route) => Some(route.as_ref()),
            Destination::InApp => None,
        };
        let locale = self.resolve_locale(user_id, tenant_id, route).await;
        let zone = athlete_zone(&self.repos, user_id).await;

        // Durable cross-replica dedup. The resolution above has already
        // confirmed there is somewhere to deliver; claim the
        // `(tenant, user, provider, window)` BEFORE reading the cache, resolving
        // the adapter, or sending so a race between replicas resolves to exactly
        // one send. A `false` claim means another replica (or an earlier
        // attempt) already sent the notice for this window, so we skip silently.
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        match db
            .claim_backfill_push(tenant_id, &user_id.to_string(), provider, after_ts)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                info!(
                    provider = %provider,
                    "backfill push already sent (dedup), skipping"
                );
                return;
            }
            Err(e) => {
                warn!(error = %e, "Backfill push: dedup claim failed");
                return;
            }
        }

        // Read the warmed window straight from the durable cache the backfill
        // just populated. Used both to confirm the data landed (empty => the
        // "ask me again" nudge) and to render the templated-list fallback.
        let transport = match &destination {
            Destination::Channel(_) => Transport::Messaging,
            Destination::InApp => Transport::PlatformJob,
        };
        let (warmed, withheld) = read_warmed_window(
            &self.repos,
            self.terms.as_ref(),
            user_id,
            tenant_id,
            provider,
            after_ts,
            transport,
        )
        .await;
        let reply = if warmed.is_empty() {
            // Cache read came back empty (it shouldn't, post-backfill): fall back
            // to the templated "your history is ready, ask again" nudge so the
            // user still hears that their history loaded. The backfill's own
            // count includes sessions the terms withhold from a model, so it is
            // only quoted when nothing was withheld.
            let count = if withheld.is_empty() {
                activity_count
            } else {
                warmed.len()
            }
            .to_string();
            RenderedReply::plain(self.strings.render(KEY_BACKFILL_READY, &locale, &[&count]))
        } else if let Destination::Channel(route) = &destination {
            // DECOUPLING (data delivery ≠ LLM judgment): the deterministic list,
            // rendered from the warmed cache the backfill just wrote, is the SPINE
            // — the user ALWAYS receives the data they asked for, regardless of the
            // model. We still attempt an in-persona agent answer via chat re-entry,
            // but only TRUST it when the turn was grounded in the data and did
            // not refuse — `engaged_with_activities` holds the rule. A re-entry
            // that refused ("Ça sort de ce que je peux t'aider…") or never had
            // the activities in front of it is dropped and we render the list
            // ourselves, never surfacing a refusal on top of (or instead of) the
            // user's own activities. This makes the LLM a best-effort
            // enhancement, not a gate on delivery.
            match self
                .try_synthesize_agent_reply(
                    user_id,
                    tenant_id,
                    pierre_conversation_id,
                    route.channel_type,
                    &locale,
                )
                .await
                .filter(|r| r.fetched_activities)
            {
                // The same layout the live reply path applies: the prose is
                // reduced to what a channel shows as typed, with the chart
                // markers stripped, and each chart follows as its own message.
                Some(agent_reply) => render_reply(
                    &messaging_render_profile(route.channel_type, &locale).render,
                    &agent_reply.assistant,
                    &self.strings,
                    &locale,
                ),
                None => RenderedReply::plain(self.render_list_body(&locale, zone, &warmed)),
            }
        } else {
            // In-app: the list, never a re-entry turn. `pierre_chat_pipeline::execute`
            // persists the prompt it is handed as a `user` row before it answers —
            // the model reads the question back out of the conversation history, so
            // the write is load-bearing, not incidental. On a channel that row is
            // invisible; in the athlete's own thread it would appear as them asking
            // the same question a second time, which they never did. The
            // deterministic list is the spine either way, so the in-app arm ships
            // the spine and leaves the fabricated turn unwritten. The fabricated
            // row is itself a defect on the channel surfaces, where the unified
            // chat list shows it: carnet#246.
            RenderedReply::plain(self.render_list_body(&locale, zone, &warmed))
        };

        // The window's true size: `warmed` when the cache read landed, else the
        // count the backfill reported.
        let count = warmed.len().max(activity_count);

        match destination {
            Destination::Channel(route) => {
                ChannelDelivery {
                    repos: &self.repos,
                    resolver: self.resolver.as_ref(),
                }
                .send(*route, user_id, tenant_id, reply, count)
                .await;
            }
            Destination::InApp => {
                InAppDelivery {
                    repos: &self.repos,
                    strings: &self.strings,
                    #[cfg(feature = "client-notifications")]
                    notifications: self.notifications.as_ref(),
                }
                // The in-app arm always ships the plain list — one text, no
                // attachments — so the persisted turn is that single part.
                .deliver(
                    user_id,
                    tenant_id,
                    pierre_conversation_id,
                    &locale,
                    reply.prose.join("\n\n"),
                    count,
                )
                .await;
            }
        }
    }

    async fn push_provider_reauth(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        pierre_conversation_id: &str,
        provider: &str,
    ) {
        // Deliberately NO dedup claim: this answers the conversation that asked,
        // and the link is re-sent on every expired-session backfill until the
        // user actually reconnects. A user whose first link was broken or never
        // clicked must not be permanently silenced — so this fires each time,
        // matching the synchronous in-chat reconnect path (which also
        // re-injects every turn). The flag and the once-per-transition notice
        // were written by the backfill before it called here.

        // Resolve the originating channel — cross-channel, DM-correct (the same
        // routing the backfill-ready notice uses).
        let Some(route) = self.resolve_route(tenant_id, pierre_conversation_id).await else {
            return;
        };
        let locale = self.resolve_locale(user_id, tenant_id, Some(&route)).await;
        let ResolvedRoute {
            session_id,
            channel_str,
            channel_type,
            recipient,
            channel_user_id: _,
            channel_tenant_id,
        } = route;

        // The historical backfill only ever runs on the scrape-backed mirrors,
        // so this is always the sciotte hosted-login path. A slug with no hosted
        // login means that gate widened: a wrong-brand link is worse than none,
        // so nothing is sent and the operator hears of it.
        if backend_resolver::hosted_login_target(provider).is_none() {
            error!(
                provider = %provider,
                "Reauth nudge: provider has no hosted login; no reconnect link minted"
            );
            return;
        }
        let Some(link) = self
            .hosted_login_url(user_id, tenant_id, provider, &channel_str)
            .await
        else {
            return;
        };

        // Render the localized reconnect message ({0}=provider brand, {1}=link)
        // and send via the shared adapter rail.
        let display =
            backend_resolver::brand_name(&global_registry(), provider).unwrap_or(provider);
        let body =
            self.strings
                .render(KEY_PROVIDER_REAUTH_REQUIRED, &locale, &[display, &link.url]);

        let outgoing = proactive_text(channel_type, recipient, body);
        let Some((adapter, channel_config)) = self
            .resolver
            .resolve(channel_tenant_id, &channel_str, channel_type)
            .await
        else {
            return;
        };
        if let Err(e) = adapter.send(&outgoing, &channel_config).await {
            warn!(error = %e, channel = %channel_str, "Reauth nudge: send failed");
            // Path parity with the completion push + synchronous reply: a failed
            // reauth-nudge send (e.g. Meta WhatsApp 131047, out-of-24h-window) must
            // be persisted + queued for the background retry worker, not silently
            // dropped — otherwise the user never learns their provider session
            // expired and keeps re-asking. The message row lands on the
            // user/session tenant; the queue row lands on the bot/channel-owner
            // tenant so the worker resolves the right channel config on re-send.
            // The retry is bounded by the link's life and stops once the
            // connection is reconnected.
            let reauth_user_id = user_id.to_string();
            if let Err(enqueue_err) = enqueue_failed_outbound(
                self.repos.messaging.as_ref(),
                adapter.as_ref(),
                &outgoing,
                &FailedOutbound {
                    transcript: Some(TranscriptRecord {
                        tenant_id,
                        session_id: &session_id,
                    }),
                    queue_tenant_id: channel_tenant_id,
                    user_id: Some(&reauth_user_id),
                    channel: &channel_str,
                    expires_at: Some(link.send_by),
                    reauth: Some(OutboundReauthGuard {
                        tenant_id,
                        provider,
                    }),
                },
            )
            .await
            {
                error!(error = %enqueue_err, "Reauth nudge: failed to enqueue dropped notice for retry");
            }
        } else {
            info!(channel = %channel_str, provider = %provider, "Sent provider-reauth nudge on channel");
        }
    }

    async fn push_reauth_to_linked_channels(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> usize {
        let mut delivered = 0;
        for target in resolve_linked_targets(self.repos.messaging.as_ref(), user_id).await {
            let channel_str = target.channel_type.to_string();
            let locale = resolve_channel_locale(
                self.repos.messaging.as_ref(),
                self.repos.users.as_ref(),
                target.tenant_id,
                &channel_str,
                &target.recipient_id,
                Some(user_id),
            )
            .await;
            let notice = self
                .reauth_notice_body(user_id, tenant_id, provider, &channel_str, &locale)
                .await;
            let outgoing = proactive_text(
                target.channel_type,
                target.recipient_id.clone(),
                notice.body,
            );
            // The link's own tenant holds the bot that can post into its chat.
            let Some((adapter, channel_config)) = self
                .resolver
                .resolve(target.tenant_id, &channel_str, target.channel_type)
                .await
            else {
                continue;
            };
            match adapter.send(&outgoing, &channel_config).await {
                Ok(_) => {
                    info!(channel = %channel_str, provider = %provider, "Sent reconnect notice on linked channel");
                    delivered += 1;
                }
                Err(e) => {
                    warn!(error = %e, channel = %channel_str, provider = %provider, "Reconnect notice: linked channel send failed; queued for retry");
                    self.queue_linked_reauth(
                        adapter.as_ref(),
                        &outgoing,
                        &target,
                        user_id,
                        OutboundReauthGuard {
                            tenant_id,
                            provider,
                        },
                        notice.send_by,
                    )
                    .await;
                }
            }
        }
        delivered
    }
}
