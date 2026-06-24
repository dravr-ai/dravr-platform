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

use std::fmt::Write as _;
use std::str::FromStr;
use std::sync::{Arc, OnceLock, Weak};

use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use pierre_chat_pipeline::{PipelineHooks, TurnInput};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, DEFAULT_LOCALE, KEY_BACKFILL_LIST_HEADER, KEY_BACKFILL_LIST_MORE,
    KEY_BACKFILL_READY,
};
use pierre_core::models::messaging::{ChannelConfig, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::{Activity, ConversationTurnId as CoreTurnId, TenantId};
use pierre_database::backends::MessagingRepository;
use pierre_database::RepositoryRegistry;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use pierre_messaging::turn::ConversationTurnId;
use pierre_tool_runtime::runtime::BackfillNotifier;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress::build_messaging_profile;

/// Max activities rendered inline in the completion notice; the rest collapse
/// into a "… and N more" footer so a deep backfill can't flood the channel.
const BACKFILL_LIST_MAX: usize = 15;

/// Read limit for the warmed window — covers a deep backfill while staying well
/// above [`BACKFILL_LIST_MAX`] so the rendered list and the "and N more" count
/// reflect the true window size, not a truncated read.
const BACKFILL_WINDOW_READ_LIMIT: i64 = 500;

/// Lower bound (days) used when the job carries no `after` timestamp, so the
/// window read still has a finite floor. The backfill always passes the job's
/// `after`, so this is only a defensive fallback.
const BACKFILL_FALLBACK_WINDOW_DAYS: i64 = 90;

/// How many recent messages to scan when recovering the user's question for the
/// chat re-entry. The triggering question is the latest `user`-role turn; a
/// small lookback tolerates a trailing assistant/tool turn without an unbounded
/// read.
const REENTRY_HISTORY_LOOKBACK: i64 = 10;

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

/// Routing pieces needed to re-enter the chat pipeline for a completed
/// backfill — borrowed so the request allocates nothing.
pub struct ReentryRequest<'a> {
    /// User the turn runs as.
    pub user_id: Uuid,
    /// Tenant for both conversation/message lookups and tool execution — the
    /// user's own tenant. The cross-tenant-bot split (the channel config living
    /// on a separate bot tenant) is handled in `push_backfill_complete`'s
    /// channel resolution, not here: the re-entry only ever needs the user
    /// tenant, since the conversation, messages, and activity cache are all
    /// stored under it.
    pub tenant_id: TenantId,
    /// Pierre conversation id the synthesized answer is appended to.
    pub conversation_id: &'a str,
    /// Channel the originating turn came from — selects the channel profile.
    pub channel_type: ChannelType,
    /// BCP-47 short locale resolved from the messaging session.
    pub locale: &'a str,
    /// The user's own question, re-asked verbatim so the coach queries the
    /// same window (e.g. "2022") against the now-warm activity cache.
    pub prompt: &'a str,
}

/// Re-runs one chat-pipeline turn so the backfill-completion push can deliver a
/// real in-persona coach answer instead of a templated activity list.
///
/// Extracted as a trait — mirroring [`AdapterResolver`] — so the notifier's
/// "synthesize, else fall back to the templated list" branch is unit-testable
/// against a fake that returns a canned reply without booting the pipeline.
#[async_trait]
pub trait ChatReentry: Send + Sync {
    /// Run the turn and return the assistant reply text, or `None` when
    /// synthesis is unavailable or failed (the caller then falls back to the
    /// templated list). Implementations log their own failures.
    async fn synthesize_reply(&self, req: ReentryRequest<'_>) -> Option<String>;
}

/// Production [`ChatReentry`]: drives `pierre_chat_pipeline::run` through the
/// composition-root [`ServerContext`].
///
/// Holds a [`Weak`] handle to break the DI-graph cycle — the context owns the
/// backfill notifier (via `ToolRuntime`), which shares the slot this handle is
/// installed into, so a strong reference back to the context would leak the
/// whole container. On a missed upgrade (shutdown in flight) synthesis is
/// skipped and the notifier falls back to the templated list.
pub struct PipelineChatReentry {
    /// Weak handle to the composition root; upgraded per call.
    ctx: Weak<ServerContext>,
}

impl PipelineChatReentry {
    /// Wrap a weak handle to the composition root.
    #[must_use]
    pub fn new(ctx: Weak<ServerContext>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl ChatReentry for PipelineChatReentry {
    async fn synthesize_reply(&self, req: ReentryRequest<'_>) -> Option<String> {
        let resources = self.ctx.upgrade()?;
        let profile = build_messaging_profile(&resources, req.channel_type);
        let mut ctx = resources.chat_pipeline_context();
        // Honor the tenant's BYO LLM key for the synthesized turn, exactly as a
        // live messaging turn does.
        if let Some(provider) = resources
            .resolve_byo_chat_provider(req.tenant_id, req.user_id)
            .await
        {
            ctx.chat_provider = Some(provider);
        }
        let turn_input = TurnInput {
            conversation_id: req.conversation_id.to_owned(),
            user_id: req.user_id.to_string(),
            conversation_tenant_id: req.tenant_id,
            tool_tenant_id: req.tenant_id,
            content: req.prompt.to_owned(),
            locale: Some(req.locale.to_owned()),
            // Fresh correlation id — this is a new (proactive) turn.
            turn_id: CoreTurnId::new(),
        };
        // No AG-UI hooks: this is a detached background turn, not a live
        // request with a status placeholder to edit.
        let hooks = PipelineHooks::none();
        match pierre_chat_pipeline::run(&ctx, turn_input, &profile, &hooks).await {
            Ok(result) if !result.content.trim().is_empty() => Some(result.content),
            Ok(_) => None,
            Err(e) => {
                warn!(error = %e, "Backfill push: chat re-entry pipeline run failed");
                None
            }
        }
    }
}

/// Install the production [`PipelineChatReentry`] on the context's backfill
/// notifier slot, now that the composition-root `Arc<ServerContext>` exists.
///
/// Called once from the binary's post-`Arc` wiring (mirroring the SSE
/// protocol-factory install): the notifier is built inside `ServerContext::new`
/// — before the `Arc` — so the pipeline-re-entry handle, which needs the `Arc`,
/// is plumbed in afterward through the shared [`OnceLock`] slot. A second call
/// is a startup logic bug, logged not fatal.
#[cfg(feature = "client-messaging")]
pub fn install_backfill_reentry(resources: &Arc<ServerContext>) {
    let reentry: Arc<dyn ChatReentry> =
        Arc::new(PipelineChatReentry::new(Arc::downgrade(resources)));
    if resources.mcp.backfill_reentry.set(reentry).is_err() {
        warn!("Backfill re-entry handle already installed; skipping");
    }
}

/// Routing pieces resolved for a completed backfill: where to deliver the
/// notice plus which tenant owns the channel config/adapter.
struct ResolvedRoute {
    /// Channel slug (e.g. `"telegram"`) for config load + logging.
    channel_str: String,
    /// Parsed channel type for adapter selection + the outgoing message.
    channel_type: ChannelType,
    /// Channel-native conversation id the notice routes to (the exact chat).
    recipient: String,
    /// BCP-47 short locale for the notice body.
    locale: String,
    /// Tenant that owns the channel config + outbound adapter — the
    /// BOT/channel-owner tenant. Differs from the session's own tenant for a
    /// cross-tenant bot (a user DMs an admin-owned bot); equals it for a
    /// single-tenant self-host. Used ONLY for the config load + send; the
    /// session lookup, warmed-cache read, and chat re-entry all stay on the
    /// user's own tenant.
    channel_tenant_id: TenantId,
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
    /// Chat-pipeline re-entry handle, shared (same `Arc`) with the context's
    /// `mcp.backfill_reentry` slot and installed post-`Arc` by
    /// [`install_backfill_reentry`]. Empty until then (and in tests that don't
    /// inject one) — the push then falls back to the templated activity list.
    reentry: Arc<OnceLock<Arc<dyn ChatReentry>>>,
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
    ) -> Arc<dyn BackfillNotifier> {
        let resolver: Arc<dyn AdapterResolver> = Arc::new(ConfigAdapterResolver {
            repos: repos.clone(),
        });
        Arc::new(Self {
            repos,
            strings,
            resolver,
            reentry,
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
        }
    }

    /// Build a notifier with an explicit adapter resolver and a pre-installed
    /// re-entry handle. Test seam for the coach-synthesis path: inject a fake
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
        let channel_user_id = session.get("channel_user_id").and_then(Value::as_str)?;
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

        // The channel config + outbound adapter live on the BOT/channel-owner
        // tenant, which differs from `tenant_id` (the user's own tenant the
        // session now lives under, post the one-tenant-per-DM-unit fix) whenever
        // the user DMs an admin-owned bot. Resolve that owner from the channel
        // link — the authoritative channel-identity -> tenant map. Absent a link
        // (single-tenant self-host where the user owns the bot), the session's
        // own tenant owns the config, so fall back to `tenant_id`.
        let channel_tenant_id = db
            .get_channel_link_tenant(channel_str, channel_user_id)
            .await
            .inspect_err(|e| {
                warn!(error = %e, "Backfill push: channel-link tenant lookup failed");
            })
            .ok()
            .flatten()
            .unwrap_or(tenant_id);

        Some(ResolvedRoute {
            channel_str: channel_str.to_owned(),
            channel_type,
            recipient: recipient.to_owned(),
            locale: locale.to_owned(),
            channel_tenant_id,
        })
    }

    /// Read the warmed window's cached activities straight from the durable
    /// activity cache the backfill just populated.
    ///
    /// Reads through the [`ActivityCacheRepository`] the notifier already holds
    /// on `repos` — no `ToolRuntime` dependency is pulled into the notifier. The
    /// window mirrors the job: `[after_ts, now]`, newest first, capped at
    /// [`BACKFILL_WINDOW_READ_LIMIT`]. Returns `None`/empty (then the caller
    /// falls back to the templated nudge) when the cache read misses or fails.
    async fn read_warmed_window(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        after_ts: i64,
    ) -> Vec<Activity> {
        let now = Utc::now();
        let start = Utc
            .timestamp_opt(after_ts, 0)
            .single()
            .unwrap_or_else(|| now - Duration::days(BACKFILL_FALLBACK_WINDOW_DAYS));
        match self
            .repos
            .activity_cache
            .get_cached_activities(
                user_id,
                &tenant_id,
                Some(provider),
                start,
                now,
                BACKFILL_WINDOW_READ_LIMIT,
            )
            .await
        {
            Ok(activities) => activities,
            Err(e) => {
                warn!(error = %e, provider = %provider, "Backfill push: warmed-window cache read failed");
                Vec::new()
            }
        }
    }

    /// Compose the completion-notice body: a localized header followed by a
    /// compact, Rust-rendered list of up to [`BACKFILL_LIST_MAX`] activities,
    /// then a localized "… and N more" footer when the window is larger.
    ///
    /// Only the header/footer phrases are localized; the per-activity lines
    /// (name · date · sport · distance) are formatted here from the `Activity`
    /// fields so the rendering stays in Rust.
    fn render_list_body(&self, locale: &str, activities: &[Activity]) -> String {
        let total = activities.len();
        let count = total.to_string();
        let mut body = self
            .strings
            .render(KEY_BACKFILL_LIST_HEADER, locale, &[&count]);

        for activity in activities.iter().take(BACKFILL_LIST_MAX) {
            body.push('\n');
            body.push_str(&format_activity_line(activity));
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
    /// the coach must query against the now-warm cache.
    ///
    /// The chat repo returns newest-first, so the latest non-empty `user`-role
    /// message is the question that triggered the backfill. `None` when the read
    /// fails or there is no user turn — the caller falls back to the list.
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
            .find(|m| m.role == "user" && !m.content.trim().is_empty())
            .map(|m| m.content)
    }

    /// Try to synthesize a real coach answer by re-entering the chat pipeline
    /// with the user's own question, now that the activity cache is warm.
    ///
    /// Loop-safe: re-asking a deep window after the cache is warmed serves
    /// inline from the durable cache (`covered == true`) and never re-spawns a
    /// backfill, so the re-entry cannot retrigger another completion push.
    /// Returns `None` (caller falls back to the templated list) when no re-entry
    /// handle is installed, the conversation has no re-askable question, or the
    /// pipeline produced nothing.
    async fn try_synthesize_coach_reply(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        channel_type: ChannelType,
        locale: &str,
    ) -> Option<String> {
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

/// Render one activity as a compact `• name · YYYY-MM-DD · Sport · 10.0 km`
/// line. Distance is shown in kilometres (one decimal) only when present; sport
/// uses the canonical display name. Pure formatting over the cheaply-available
/// `Activity` accessors — no allocation beyond the returned line.
fn format_activity_line(activity: &Activity) -> String {
    let date = activity.start_date().format("%Y-%m-%d");
    let sport = activity.sport_type().display_name();
    let mut line = format!("• {} · {date} · {sport}", activity.name());
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
        let Some(ResolvedRoute {
            channel_str,
            channel_type,
            recipient,
            locale,
            channel_tenant_id,
        }) = self.resolve_route(tenant_id, pierre_conversation_id).await
        else {
            return;
        };

        // Durable cross-replica dedup. The staleness reverse-lookup above has
        // already confirmed there is a live channel to notify; claim the
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
        let warmed = self
            .read_warmed_window(user_id, tenant_id, provider, after_ts)
            .await;
        let body = if warmed.is_empty() {
            // Cache read came back empty (it shouldn't, post-backfill): fall back
            // to the templated "your history is ready, ask again" nudge so the
            // user still hears that their history loaded.
            let count = activity_count.to_string();
            self.strings.render(KEY_BACKFILL_READY, &locale, &[&count])
        } else if let Some(coach_reply) = self
            .try_synthesize_coach_reply(
                user_id,
                tenant_id,
                pierre_conversation_id,
                channel_type,
                &locale,
            )
            .await
        {
            // Best path: re-enter the chat pipeline with the user's own question
            // (the cache is now warm) so they get a real in-persona coach answer.
            coach_reply
        } else {
            // No re-entry handle, no re-askable question, or the pipeline run
            // produced nothing — degrade gracefully to the Rust-rendered list.
            self.render_list_body(&locale, &warmed)
        };

        let outgoing = OutgoingMessage {
            channel_type,
            recipient_id: recipient,
            content: MessageContent::Text { body },
            // Fresh turn — this is a proactive push, not a reply to the
            // originating turn.
            turn_id: ConversationTurnId::new(),
            reply_to: None,
            thread_id: None,
        };

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

        if let Err(e) = adapter.send(&outgoing, &channel_config).await {
            warn!(error = %e, channel = %channel_str, "Failed to send backfill-ready notice on channel");
        } else {
            info!(
                channel = %channel_str,
                count = warmed.len().max(activity_count),
                "Sent backfill-ready notice on channel"
            );
        }
    }
}
