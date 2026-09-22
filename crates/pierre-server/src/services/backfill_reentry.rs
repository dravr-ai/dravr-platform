// ABOUTME: Re-runs one chat-pipeline turn so a backfill-completion push can answer in the coach's voice
// ABOUTME: The ChatReentry seam, its production impl, and the rule for trusting what it produced
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chat re-entry for the backfill-completion push.
//!
//! A deep-history question warms the durable activity cache off the request
//! path and answers with a "give me a minute".
//!
//! Once the cache is warm, the notifier re-asks that same question through the
//! ordinary chat pipeline so the athlete gets an in-persona answer — and the
//! pipeline runs exactly as a live messaging turn does, scene publisher
//! included, so a chart the agent draws leaves as an image rather than as its
//! positional marker.
//!
//! The re-entry is a best-effort enhancement, never a gate on delivery:
//! [`engaged_with_activities`] decides whether the turn actually answered from
//! the athlete's data, and the notifier renders the deterministic list itself
//! when it did not.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use pierre_chat_pipeline::{
    AssistantTurn, CommandPersistence, PipelineHooks, ServedTurn, TurnOrigin, TurnRequest,
    TurnTelemetry,
};
use pierre_contremaitre::messaging_strings::{KEY_CAPABILITY_REFUSAL, KEY_SCOPE_REFUSAL};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{ConversationTurnId as CoreTurnId, TenantId};
use tracing::warn;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress::build_messaging_profile;
use crate::services::messaging_ingress::scene_publisher::{
    athlete_color_scheme, MessagingScenePublisher,
};

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
    /// The user's own question, re-asked verbatim so the agent queries the
    /// same window (e.g. "2022") against the now-warm activity cache.
    pub prompt: &'a str,
}

/// A synthesized backfill-push reply.
///
/// Carries the agent's analysis plus the rendered activity list the re-entry's
/// `get_activities` produced (sorted per the re-asked question). The list is
/// prepended to the analysis on delivery so the user SEES their activities,
/// mirroring the live messaging path.
pub struct ReentryReply {
    /// The turn's blocks, as the pipeline laid them out for the channel.
    ///
    /// A messaging surface has no activity panel, so the athlete's list is
    /// already folded into the prose block — and a chart the agent drew rides
    /// as its own `SceneImage` block, minted against the durable message. The
    /// push lays the blocks out through the same egress a live reply takes
    /// rather than reading the prose off and composing a second time; that
    /// shortcut is how `⟦viz:0⟧` reached a Telegram bubble as literal text
    /// with the chart it stood for never sent (2026-09-22).
    pub assistant: AssistantTurn,
    /// Whether the re-entry turn answered from the athlete's activities — see
    /// [`engaged_with_activities`].
    ///
    /// The push exists to deliver a freshly backfilled history, so an agent
    /// reply that never had it, or had it and refused, is discarded in favour
    /// of the templated list rather than surfacing an analysis of nothing.
    pub fetched_activities: bool,
}

/// Re-runs one chat-pipeline turn so the backfill-completion push can deliver a
/// real in-persona agent answer instead of a templated activity list.
///
/// Extracted as a trait — mirroring
/// [`AdapterResolver`](super::backfill_notifier::AdapterResolver) — so the notifier's
/// "synthesize, else fall back to the templated list" branch is unit-testable
/// against a fake that returns a canned reply without booting the pipeline.
#[async_trait]
pub trait ChatReentry: Send + Sync {
    /// Run the turn and return the assistant reply (analysis + any activity
    /// list), or `None` when synthesis is unavailable or failed (the caller then
    /// falls back to the templated list). Implementations log their own failures.
    async fn synthesize_reply(&self, req: ReentryRequest<'_>) -> Option<ReentryReply>;
}

/// Production [`ChatReentry`]: drives `pierre_chat_pipeline::execute` through the
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

/// Whether a re-entry turn engaged with the athlete's activities, and so may be
/// trusted to answer about them.
///
/// A turn is grounded in activities two ways. The tool loop captured a list
/// (`activity_list_captured`): the model asked and got them. Or the platform
/// prefetched a non-empty window into the prompt (`activities_prefetched`), and
/// the prompt contract told the agent to answer from it WITHOUT re-fetching —
/// so the well-behaved turn calls nothing. Trusting only the first threw away
/// every answer the second produced: on 2026-09-21 an athlete asked for monthly
/// elevation totals, the re-entry turn answered from the prefetched window, and
/// the push delivered the templated list in its place.
///
/// What a prefetched turn must not be allowed to push is a refusal: the model
/// had the data and declined the question anyway, which would land in place of
/// the history the athlete asked for. Refusals are canonical — the system
/// prompt interpolates the exact sentence per locale (`canonical_refusals`) —
/// so a reply carrying one is recognised rather than guessed at.
///
/// Deliberately does NOT scan `tools_called` for `get_activities`: that name is
/// left behind by the prefetch and by a model call that failed alike, so it
/// says nothing about whether any data reached the model.
#[must_use]
pub fn engaged_with_activities(
    telemetry: &TurnTelemetry,
    reply: &str,
    canonical_refusals: &[String],
) -> bool {
    if telemetry.activity_list_captured {
        return true;
    }
    telemetry.activities_prefetched && !carries_canonical_refusal(reply, canonical_refusals)
}

/// Whether `reply` contains one of the platform's canonical refusal sentences.
///
/// Compared case-insensitively with whitespace collapsed, because a channel
/// fold or the model's own line breaks can re-wrap the sentence without
/// changing it. An empty sentence — a locale the catalogue does not carry —
/// is skipped: it would otherwise be contained in every reply.
fn carries_canonical_refusal(reply: &str, canonical_refusals: &[String]) -> bool {
    let normalize = |text: &str| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let reply = normalize(reply);
    canonical_refusals
        .iter()
        .map(|sentence| normalize(sentence))
        .any(|sentence| !sentence.is_empty() && reply.contains(&sentence))
}

#[async_trait]
impl ChatReentry for PipelineChatReentry {
    async fn synthesize_reply(&self, req: ReentryRequest<'_>) -> Option<ReentryReply> {
        let resources = self.ctx.upgrade()?;
        let profile = build_messaging_profile(&resources, req.channel_type, req.locale.to_owned());
        // Charts: this channel cannot draw a spec, so the pipeline asks the
        // publisher for a signed image URL per block once the assistant row is
        // durable — the same seam the live reply wires, so a re-asked question
        // that draws a chart delivers it rather than its marker.
        let scene_publisher = MessagingScenePublisher::new(
            Arc::clone(&resources),
            profile.render,
            athlete_color_scheme(&resources, req.user_id).await,
        );
        let ctx = resources.chat_pipeline_context();
        let channel_slug = req.channel_type.to_string();
        let request = TurnRequest {
            // The platform re-asks the athlete's own earlier question once their
            // history has loaded. They did not send it a second time, so it is
            // never written to their transcript as though they had — carnet#246.
            origin: TurnOrigin::Platform,
            conversation_id: req.conversation_id.to_owned(),
            user_id: req.user_id,
            conversation_tenant_id: req.tenant_id,
            tool_tenant_id: req.tenant_id,
            content: req.prompt.to_owned(),
            // Fresh correlation id — this is a new (proactive) turn.
            turn_id: CoreTurnId::new(),
            // Proactive pushes land in the user's own DM conversation.
            ambient_context: None,
            channel_type: &channel_slug,
            is_direct_message: true,
            // The messaging DM's answers; the re-asked prompt is never a
            // command (command rows are skipped when it is chosen), so these
            // only keep the surface's contract intact.
            ambient_group_fallback: true,
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            // No AG-UI wiring: this is a detached background turn, not a live
            // request with a status placeholder to edit.
            hooks: PipelineHooks {
                scene_publisher: Some(&scene_publisher),
                ..PipelineHooks::none()
            },
        };
        // Through the same turn service a live turn takes, so this synthesized
        // reply spends the athlete's budget, honours their BYO LLM key and is
        // refused by the same caps — a proactive push is a real model call, and
        // a ladder of its own is how one stops being any of those things.
        match pierre_chat_pipeline::execute(&ctx, request, &profile).await {
            // The envelope's prose already carries the list the re-asked
            // question produced (sorted per the user's wording), folded in
            // exactly as a live messaging turn folds it.
            Ok(ServedTurn::Pipeline(result)) if !result.assistant.prose().trim().is_empty() => {
                let canonical_refusals = [KEY_SCOPE_REFUSAL, KEY_CAPABILITY_REFUSAL]
                    .map(|key| ctx.messaging_strings_registry.get(key, req.locale));
                let fetched_activities = engaged_with_activities(
                    &result.telemetry,
                    result.assistant.prose(),
                    &canonical_refusals,
                );
                Some(ReentryReply {
                    assistant: result.assistant,
                    fetched_activities,
                })
            }
            // An empty reply, a slash command (the prompt is platform-authored
            // and never one) or a caller-served turn (never requested here) all
            // mean there is nothing to push: the notifier falls back to the
            // templated activity list.
            Ok(_) => None,
            Err(e) => {
                warn!(error = %e, "Backfill push: chat re-entry turn failed");
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
