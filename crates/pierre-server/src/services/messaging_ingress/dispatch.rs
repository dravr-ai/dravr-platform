// ABOUTME: LLM dispatch + outbound delivery + retry queue for messaging turns
// ABOUTME: dispatch_and_respond is the per-turn orchestrator; helpers handle send/persist/enqueue

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt::Write as _;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Instant;

use futures_util::FutureExt;
use pierre_core::models::messaging::{ChannelConfig, MessageContent, OutgoingMessage};
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::tokens::estimate_chat_tokens;
use pierre_database::backends::{InsertMessageParams, MessagingRepository};
use pierre_llm::TokenUsage;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use pierre_chat_pipeline::{self, DispatchResult, PipelineHooks, TurnInput};
use pierre_contremaitre::messaging_strings::{
    format_template, MessagingStringsRegistry, KEY_COACH_PROPOSAL_FOOTER,
    KEY_COACH_PROPOSAL_WELCOME, KEY_COACH_PROPOSAL_WELCOME_GENERIC, KEY_EMPTY_REPLY,
    KEY_ERROR_GENERIC,
};
use pierre_core::error_helpers::panic_payload_str;
use pierre_core::errors::AppError;
use pierre_routes_coaches::coaches::{build_coach_proposal, ProposedCoach, SportProfileSummary};
use pierre_runtime_context::{default_admin_config, AdminConfigLookup};
use pierre_services::analytics::hash_id;
use pierre_services::usage_counter::UsageCounterService;

use super::addressing::reply_recipient;
use super::agui::{setup_messaging_agui, MessagingAgUiWiring};
use super::connect;
use super::viz_delivery::{
    plan_media, strip_viz_markers, target as viz_target, VizDelivery, VizMedia,
};
use super::{
    build_messaging_profile, compose_messaging_reply, content_body_text, outbound_retry,
    PendingDispatch, CONVERSATION_DISPATCH_LOCKS,
};
use pierre_services::onboarding_gate::user_has_connected_provider;

/// Auto-send the one-time onboarding coach proposal for this user, if it hasn't
/// been sent on this channel link yet.
///
/// Fires on the user's first provider-connected messaging turn: builds the
/// inferred-profile proposal (shared with the REST route via
/// [`build_coach_proposal`]), renders it to text, sends it through the channel
/// adapter, and stamps the link so it never re-sends. Entirely best-effort —
/// any failure is logged and the turn proceeds normally. When no coaches are
/// eligible yet (cold start, activities not synced) it returns *without*
/// stamping, so a later turn can propose once data lands.
/// Send the tappable "Connect your account" card alongside a served turn, for
/// as long as the athlete has no provider connected.
///
/// This card used to ride the refusal: a providerless user got the card
/// *instead of* a coach reply. Now they get the reply, so the card rides it
/// instead of replacing it — the nudge survives the gate it was attached to.
///
/// Every turn, deliberately. The alternative considered was once per
/// conversation, but a user who has not connected still cannot be given
/// grounded coaching on their next message either, and a tappable button is a
/// cheaper reminder than a coach repeatedly explaining what it cannot see.
///
/// Direct messages only, enforced inside [`connect::try_build_connect_card`]: a
/// connect link is user-scoped and must never be posted into a shared room.
/// Send failures are logged, never fatal — the coach's answer already went out
/// and is the thing the athlete asked for.
async fn maybe_send_connect_card(dispatch: &PendingDispatch, channel_config: &ChannelConfig) {
    let has_provider = user_has_connected_provider(
        &dispatch.resources.common.repos.provider_connections,
        dispatch.auth_result.user_id,
    )
    .await
    // Fail open: on a lookup error, say nothing rather than nag a connected
    // athlete with a card they do not need.
    .unwrap_or(true);
    if has_provider {
        return;
    }

    let Some(card) = connect::try_build_connect_card(
        &dispatch.resources,
        dispatch.channel_tenant_id,
        &dispatch.channel,
        dispatch.channel_type,
        &dispatch.sender_id,
        dispatch.thread_id.clone(),
        !dispatch.is_group_chat,
        &dispatch.locale,
    )
    .await
    else {
        return;
    };

    if let Err(e) = dispatch.adapter.send(&card, channel_config).await {
        warn!(error = %e, "connect card: send failed; the coach reply still went out");
    }
}

async fn maybe_send_coach_proposal(dispatch: &PendingDispatch, channel_config: &ChannelConfig) {
    let Some((outgoing, offered_ids)) = build_coach_proposal_message(dispatch).await else {
        return;
    };

    if let Err(e) = dispatch.adapter.send(&outgoing, channel_config).await {
        warn!(error = %e, "coach proposal: send failed; will retry next turn");
        return;
    }

    stamp_coach_proposal_sent(dispatch, &offered_ids).await;

    info!(
        hashed_user = %hash_id(&dispatch.session.user_id),
        "coach proposal auto-sent"
    );
}

/// Stamp the channel link so the proposal is never re-sent. Best-effort: a
/// failure here only risks a duplicate proposal on a later turn, never the turn.
async fn stamp_coach_proposal_sent(dispatch: &PendingDispatch, offered_ids: &[String]) {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();
    if let Err(e) = db
        .mark_coach_proposal_sent(
            dispatch.channel_tenant_id,
            &dispatch.channel,
            &dispatch.sender_id,
            offered_ids,
        )
        .await
    {
        warn!(error = %e, "coach proposal: sent but failed to stamp link; may re-send next turn");
    }
}

/// Decide whether to auto-send and, if so, build the outbound proposal message.
///
/// Returns `None` when the proposal was already sent (or the idempotency read
/// errored — fail closed), when the user already has an active coach (they are
/// past onboarding), when the build fails, or when no coaches are eligible yet
/// (cold start). In the cold-start case the link is intentionally left
/// un-stamped so a later turn can propose once activities sync.
async fn build_coach_proposal_message(
    dispatch: &PendingDispatch,
) -> Option<(OutgoingMessage, Vec<String>)> {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();

    let already_sent = db
        .coach_proposal_sent(
            dispatch.channel_tenant_id,
            &dispatch.channel,
            &dispatch.sender_id,
        )
        .await
        .unwrap_or(true); // fail closed: never risk double-sending on a read error
    if already_sent {
        return None;
    }

    // Never onboard a user who already has an active coach. The idempotency
    // flag alone is insufficient: a user can acquire a coach on the web before
    // ever receiving a messaging proposal, leaving the flag NULL — and the
    // "Welcome!" lead-in is jarring for someone mid-plan. Re-checked each turn
    // (cheap, indexed); left un-stamped so a transient read error can't
    // permanently suppress a genuinely new user's proposal.
    let has_active_coach = dispatch
        .resources
        .common
        .repos
        .coaches
        .get_active_coach(dispatch.auth_result.user_id, dispatch.user_tenant_id)
        .await
        .map_or(true, |coach| coach.is_some()); // fail closed: never onboard a possibly-coached user
    if has_active_coach {
        return None;
    }

    let (profile, coaches) = build_coach_proposal(
        &dispatch.resources,
        dispatch.auth_result.user_id,
        dispatch.user_tenant_id,
        &dispatch.locale,
    )
    .await
    .inspect_err(|e| warn!(error = %e, "coach proposal: build failed; skipping"))
    .ok()?;

    if coaches.is_empty() {
        return None;
    }

    let body = render_coach_proposal_text(
        &profile,
        &coaches,
        &dispatch.resources.mcp.messaging_strings_registry,
        &dispatch.locale,
    );
    // Captured in the SAME order the user reads, because that ordering is what a
    // numeric reply indexes into.
    let offered_ids: Vec<String> = coaches.iter().map(|c| c.coach.id.clone()).collect();
    Some((
        OutgoingMessage {
            channel_type: dispatch.channel_type,
            recipient_id: dispatch.sender_id.clone(),
            content: MessageContent::Text { body },
            // A fresh turn id: the proposal is a proactive message, not a reply to
            // the user's inbound turn.
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: dispatch.thread_id.clone(),
        },
        offered_ids,
    ))
}

/// Render the onboarding coach proposal as a channel text message: a short
/// profile-aware lead-in, then a numbered list of `title — reason` lines.
///
/// The lead-in and footer are resolved from the messaging-strings `registry`
/// for `locale`; the numbered list is locale-neutral formatting and the
/// per-coach reasons arrive already localized from [`build_coach_proposal`].
fn render_coach_proposal_text(
    profile: &SportProfileSummary,
    coaches: &[ProposedCoach],
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> String {
    let count = coaches.len().to_string();
    let mut body = profile
        .primary_sport
        .as_deref()
        .filter(|_| profile.has_profile)
        .map_or_else(
            || registry.render(KEY_COACH_PROPOSAL_WELCOME_GENERIC, locale, &[&count]),
            |primary| registry.render(KEY_COACH_PROPOSAL_WELCOME, locale, &[primary, &count]),
        );
    for (index, proposed) in coaches.iter().enumerate() {
        let reason = if proposed.reason.is_empty() {
            String::new()
        } else {
            format!(" — {}", proposed.reason)
        };
        let _ = writeln!(
            body,
            "{number}. {title}{reason}",
            number = index + 1,
            title = proposed.coach.title,
        );
    }
    body.push_str(&registry.get(KEY_COACH_PROPOSAL_FOOTER, locale));
    body
}

/// Route the assistant reply back to the user.
///
/// Prefers finalizing the in-channel status placeholder when the
/// bridge is active (so the user sees status and reply collapse into
/// a single chat message). Falls back to the standard outbound send
/// path when the bridge is disabled (`WhatsApp`/Messenger), inactive
/// (credentials missing), or errored mid-turn (edit rejected).
async fn deliver_reply(
    dispatch: &PendingDispatch,
    messaging_agui: Option<&MessagingAgUiWiring>,
    channel_config: &ChannelConfig,
    content: String,
    turn_id: ConversationTurnId,
    charts: Vec<VizMedia>,
) {
    // Markers are for clients that interleave prose and charts. A channel gets
    // either an image or nothing, so the placeholder is noise either way.
    let content = strip_viz_markers(&content);

    if let Some(wiring) = messaging_agui {
        if wiring.finalize_reply(&content).await {
            return;
        }
    }

    // Use conversation_id (channel/chat/thread) as the reply target
    // when available; fall back to sender_id for DM-only platforms
    // (e.g., WhatsApp).
    let reply_target =
        reply_recipient(dispatch.conversation_id.as_deref(), &dispatch.sender_id).to_owned();

    // The outbound reply carries the turn id from the inbound utterance,
    // so a consumer inspecting the `DeliveryReceipt` can look up the full
    // turn trace via `/internal/conversation-turn`. `.into()` bridges
    // pierre-core's newtype to canot's.
    let outgoing = OutgoingMessage {
        channel_type: dispatch.channel_type,
        recipient_id: reply_target,
        content: MessageContent::Text { body: content },
        turn_id: turn_id.into(),
        reply_to: Some(dispatch.channel_message_id.clone()),
        thread_id: dispatch.thread_id.clone(),
    };
    send_outbound_response(dispatch, channel_config, &outgoing).await;

    // Charts follow the prose, in the order the coach placed them. Sent as
    // separate messages because no channel here renders several images inside
    // one text bubble, and the prose must land first — it is what makes the
    // pictures mean something.
    for chart in charts {
        let media = OutgoingMessage {
            channel_type: dispatch.channel_type,
            recipient_id: reply_recipient(dispatch.conversation_id.as_deref(), &dispatch.sender_id)
                .to_owned(),
            content: chart.into_content(),
            turn_id: turn_id.into(),
            // Threaded under the prose rather than the athlete's message, so a
            // client that groups replies keeps the chart with its explanation.
            reply_to: Some(dispatch.channel_message_id.clone()),
            thread_id: dispatch.thread_id.clone(),
        };
        send_outbound_response(dispatch, channel_config, &media).await;
    }
}

/// Log the pipeline failure, track analytics, and send a localized
/// generic-error reply with a short correlation id.
///
/// Extracted from `dispatch_and_respond` to keep the orchestrator's
/// cognitive complexity inside the workspace lint budget; the body
/// is otherwise a straight line of effects (log, track, template,
/// send) without branching.
async fn report_dispatch_failure(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    err: &AppError,
) {
    // Correlation ID is surfaced in the user-facing reply and the
    // log record so an operator receiving a Slack alert can grep
    // Cloud Logging for the full error chain without access to
    // conversation IDs (which are PII-adjacent).
    let correlation_id = Uuid::new_v4();
    error!(
        correlation_id = %correlation_id,
        error = %err,
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
        "LLM dispatch failed for messaging"
    );
    info!(
        target: "notify",
        event = "messaging.error",
        tenant_id = %dispatch.channel_tenant_id,
        channel = %dispatch.channel,
        error_type = "llm_dispatch_failed",
        "messaging error"
    );
    let short_id = correlation_id.to_string()[..8].to_owned();
    let template = dispatch
        .resources
        .mcp
        .messaging_strings_registry
        .get(KEY_ERROR_GENERIC, &dispatch.locale);
    let user_message = format_template(&template, &[&short_id]);
    send_error_reply(dispatch, channel_config, &user_message).await;
}

/// Maximum ambient-transcript lines injected into a group turn's prompt.
const AMBIENT_TRANSCRIPT_MAX_LINES: usize = 25;

/// Maximum characters kept per ambient-transcript line (grapheme-unaware
/// char truncation is fine for prompt context).
const AMBIENT_TRANSCRIPT_MAX_LINE_CHARS: usize = 240;

/// Build the speaker-labeled ambient transcript for a group turn.
///
/// Group chat history is per member — each member's `chat_messages` holds
/// only their own exchanges with the coach — so this reassembles the room
/// view from `messaging_messages` across every member's session: inbound
/// rows labeled with the sender's display name, outbound rows labeled
/// "Coach". The triggering message itself is excluded (it arrives as the
/// turn's user content). Returns `None` when the room has no other recent
/// messages, so DM-shaped groups cost no prompt tokens.
async fn build_group_ambient_context(dispatch: &PendingDispatch) -> Option<String> {
    let chat_id = dispatch
        .conversation_id
        .as_deref()
        .filter(|c| !c.is_empty())?;
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();

    let limit = i64::try_from(AMBIENT_TRANSCRIPT_MAX_LINES).unwrap_or(25) + 1;
    let rows = match db
        .list_recent_chat_messages(
            dispatch.channel_tenant_id,
            &dispatch.channel,
            chat_id,
            limit,
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            warn!(error = %e, "ambient transcript load failed; dispatching without it");
            return None;
        }
    };

    // Newest-first from the query; drop the triggering message, restore
    // chronological order, cap the line count.
    let mut lines: Vec<String> = Vec::new();
    let mut label_cache: HashMap<String, String> = HashMap::new();
    for row in rows
        .iter()
        .filter(|r| r["channel_message_id"].as_str() != Some(dispatch.channel_message_id.as_str()))
        .take(AMBIENT_TRANSCRIPT_MAX_LINES)
    {
        let Some(body) = row["content_body"].as_str().filter(|b| !b.is_empty()) else {
            continue;
        };
        let label = if row["direction"].as_str() == Some("outbound") {
            "Coach".to_owned()
        } else {
            let user_id = row["user_id"].as_str().unwrap_or_default();
            speaker_label(dispatch, user_id, &mut label_cache).await
        };
        let truncated: String = body
            .chars()
            .take(AMBIENT_TRANSCRIPT_MAX_LINE_CHARS)
            .collect();
        lines.push(format!("{label}: {truncated}"));
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();

    Some(format!(
        "## Recent group chat\n\
         This conversation happens inside a group chat. The lines below are \
         the room's most recent messages, oldest first, for context only — \
         answer the current message, which follows the conversation history. \
         Never prefix your reply with a name label.\n\n{}",
        lines.join("\n")
    ))
}

/// Resolve a member's display label for the ambient transcript, caching per
/// build. Falls back to the email local-part, then a neutral "Member".
async fn speaker_label(
    dispatch: &PendingDispatch,
    user_id: &str,
    cache: &mut HashMap<String, String>,
) -> String {
    if let Some(cached) = cache.get(user_id) {
        return cached.clone();
    }
    let label = match Uuid::parse_str(user_id) {
        Ok(uuid) => match dispatch.resources.common.repos.users.get_global(uuid).await {
            Ok(Some(user)) => user
                .display_name
                .unwrap_or_else(|| user.email.split('@').next().unwrap_or("Member").to_owned()),
            _ => "Member".to_owned(),
        },
        Err(_) => "Member".to_owned(),
    };
    cache.insert(user_id.to_owned(), label.clone());
    label
}

/// Dispatch a message through the LLM pipeline and send the response back via the channel
///
/// Runs as a background task after the webhook has returned HTTP 200.
/// Acquires a per-conversation lock to ensure messages are processed in order.
///
/// The `#[instrument]` span pins `turn_id`, `channel`, and `conversation_id`
/// onto every downstream log line (chat pipeline stages, embacle HTTP call)
/// so an operator can grep a single `turn_id=...` across the whole flow.
#[tracing::instrument(
    skip(dispatch),
    fields(
        turn_id = %dispatch.turn_id,
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
    )
)]
pub async fn dispatch_and_respond(dispatch: PendingDispatch) {
    let lock = CONVERSATION_DISPATCH_LOCKS
        .entry(dispatch.session.conversation.clone())
        .or_insert_with(|| Arc::new(TokioMutex::new(())))
        .clone();
    let dispatch_guard = lock.lock().await;

    let start = Instant::now();
    let hashed_user = hash_id(&dispatch.session.user_id);

    // Log the inbound user message at debug. The full body is dumped at
    // trace level so an operator can run `RUST_LOG=...=trace` to follow a
    // typed message all the way to the LLM call without needing to enable
    // payload events at the ingress layer in prod.
    info!(
        text_len = dispatch.text_content.len(),
        hashed_user = %hashed_user,
        "messaging dispatch starting"
    );
    tracing::trace!(text = %dispatch.text_content, "messaging dispatch user message");

    let profile = build_messaging_profile(&dispatch.resources, dispatch.channel_type);
    // Reuse the turn id canot generated at the webhook boundary
    // (stored on `dispatch.turn_id`). The inbound webhook is the
    // boundary for platform-side observability: a single inbound
    // message plus its full LLM/tool chain is one turn, and canot's
    // log spans already key off this id.
    let turn_id = dispatch.turn_id;
    // Group turns get the room's recent cross-member transcript; DMs skip
    // the lookup entirely.
    let ambient_context = if dispatch.is_group_chat {
        build_group_ambient_context(&dispatch).await
    } else {
        None
    };
    let turn_input = TurnInput {
        conversation_id: dispatch.session.conversation.clone(),
        user_id: dispatch.session.user_id.clone(),
        // The conversation lives under the session tenant (user's own for DMs);
        // the pipeline reads/writes it with this tenant. Tools still execute
        // under user_tenant_id (the same value for DMs).
        conversation_tenant_id: dispatch.session_tenant_id,
        tool_tenant_id: dispatch.user_tenant_id,
        content: dispatch.text_content.clone(),
        locale: Some(dispatch.locale.clone()),
        turn_id,
        ambient_context,
    };

    // Load the per-tenant channel config exactly once per turn.
    //
    // Both the AG-UI status bridge (placeholder open + finalize) and
    // the fallback outbound send need it; threading the same snapshot
    // through avoids the 2-3 DB round-trips a naive wiring would
    // incur, and keeps every reply-path consistent with the
    // credentials live at the moment the dispatch started.
    //
    // `None` means the tenant has no configured channel — we cannot
    // reply at all, so log and bail without spending compute on the
    // LLM pipeline.
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();
    let Some(channel_config) =
        load_channel_config(db, dispatch.channel_tenant_id, &dispatch.channel).await
    else {
        warn!(
            channel = %dispatch.channel,
            tenant_id = %dispatch.channel_tenant_id,
            "channel config unavailable at dispatch time; dropping turn with no reply"
        );
        drop(dispatch_guard);
        evict_idle_dispatch_lock(&dispatch.session.conversation, &lock);
        return;
    };

    // One-time onboarding coach proposal: on the user's first provider-connected
    // turn, lead with the inferred-profile coach suggestions before processing
    // their message. Best-effort — never blocks or fails the turn.
    maybe_send_coach_proposal(&dispatch, &channel_config).await;
    maybe_send_connect_card(&dispatch, &channel_config).await;

    // Register an AG-UI run for this messaging turn so in-process
    // consumers (channel-side status adapters, ops dashboards) can
    // subscribe via `resources.agui_registry.subscribe_self(run_id)`
    // and render pipeline progress to Telegram/Slack/Discord.
    // Without this wiring the pipeline's `hooks.agui` stayed `None` on
    // every messaging turn and AG-UI events were produced only for
    // HTTP web-chat callers that passed `agui_run_id`.
    //
    // The run is owned by `(session.user_id, user_tenant_id)` so any
    // cross-user HTTP subscriber (e.g. canot's `AgUiConsumer` against
    // the platform SSE route) still goes through the owner check in
    // `RunRegistry::authorize_and_subscribe`. Scope drops at function
    // exit, auto-unregistering on success, error, or panic.
    //
    // `setup_messaging_agui` also opens an in-channel status adapter
    // (Telegram editMessageText / Slack chat.update / Discord PATCH
    // messages) and spawns a background consumer that mirrors each
    // AG-UI event as a `set_status` call so the user sees the pipeline
    // stage in real time.
    let messaging_agui = setup_messaging_agui(&dispatch, &channel_config).await;
    let hooks = PipelineHooks {
        agui: messaging_agui.as_ref().map(|w| w.run()),
        ..PipelineHooks::none()
    };

    let mut ctx = dispatch.resources.chat_pipeline_context();
    // Use the tenant's own LLM key (BYO) for this turn when one is stored.
    if let Some(provider) = dispatch
        .resources
        .resolve_byo_chat_provider(dispatch.user_tenant_id, dispatch.auth_result.user_id)
        .await
    {
        ctx.chat_provider = Some(provider);
    }
    // Panic boundary: a bug in any pipeline stage must unwind into a structured
    // failure for *this* turn (graceful user reply + correlation-id log), never
    // escape the spawned task. `AssertUnwindSafe` is sound because a caught
    // panic aborts the whole turn — none of the borrowed state is touched
    // afterwards; we report and return.
    let run = AssertUnwindSafe(pierre_chat_pipeline::run(
        &ctx, turn_input, &profile, &hooks,
    ));
    let dispatch_result = match run.catch_unwind().await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            report_dispatch_failure(&dispatch, &channel_config, &e).await;
            return;
        }
        Err(panic) => {
            let err = AppError::internal(format!(
                "chat pipeline panicked: {}",
                panic_payload_str(panic.as_ref())
            ));
            report_dispatch_failure(&dispatch, &channel_config, &err).await;
            return;
        }
    };

    // Safe cast: execution time will never exceed u64::MAX milliseconds (~584 million years)
    #[allow(clippy::cast_possible_truncation)]
    let execution_time_ms = start.elapsed().as_millis() as u64;

    info!(
        target: "notify",
        event = "messaging.response_sent",
        user_id = %dispatch.session.user_id,
        tenant_id = %dispatch.channel_tenant_id,
        channel = %dispatch.channel,
        response_type = "llm",
        execution_time_ms = execution_time_ms,
        model = %dispatch_result.model,
        "messaging response sent"
    );

    // Security signal: the coach reply identified as the underlying model/
    // provider and was withheld at the response boundary (the chat pipeline's
    // identity-leak stage). The user received the canned withheld string, not
    // the leak; this event surfaces the withhold on #dravr-signal so a
    // recurrence is visible instead of silent in the logs. The `pattern_*`
    // labels (extra fields beyond the catalogue's required set) identify which
    // pattern class fired — never the reply text, which is never persisted.
    //
    // The tenant is `session_tenant_id` — the SAME field this path hands the
    // pipeline as `conversation_tenant_id` (see the `TurnInput` construction
    // below), which is what the detection side logs. It used to be
    // `channel_tenant_id`, and the two are documented to differ whenever a
    // messaging user belongs to a different tenant than the bot owning the
    // webhook. On the 2026-08-05 Telegram break that is exactly what happened:
    // detection recorded 0f09bd31 and this alert recorded 988c4bbc for one
    // turn. Since the event dedups per tenant, keying it on the bot's tenant
    // collapses leaks from different athletes into one another, and an operator
    // triaging from #dravr-signal cannot reach the conversation — so
    // `conversation_id` and `turn_id` ride along for that.
    if let Some(leak) = dispatch_result.identity_leak {
        info!(
            target: "notify",
            event = "messaging.identity_leak",
            tenant_id = %dispatch.session_tenant_id,
            conversation_id = %dispatch.session.conversation,
            turn_id = %dispatch_result.turn_id,
            channel = %dispatch.channel,
            model = %dispatch_result.model,
            pattern_class = leak.class.as_str(),
            pattern_locale = leak.locale,
            pattern_index = leak.pattern_index,
            "coach reply identified as the underlying model/provider; withheld at the response boundary"
        );
    }

    // Record LLM usage for cost tracking and quota enforcement
    // Per-LLM-call rows are written inline by the chat pipeline's
    // `LlmCallRecorder`; the turn-summary marker row has been removed
    // now that `llm_usage` is a pure per-call ledger.
    let _ = execution_time_ms;

    // Increment usage counters (message count, token count, tool call count)
    increment_messaging_usage_counters(&dispatch, &dispatch_result).await;

    // Prepend the activity list to the coach's analysis so the user actually
    // SEES their activities. The web/mobile UI auto-renders this list as a card;
    // text channels have no such card, so without this the coach's "the list is
    // already shown" assumption leaves messaging users with analysis only. The
    // list follows the request's `sort_by` order and is adaptively capped for
    // small screens.
    let reply_body = compose_messaging_reply(
        dispatch_result.activity_list.as_deref(),
        dispatch_result.content,
        &dispatch.resources.mcp.messaging_strings_registry,
        &dispatch.locale,
    );

    // Guard: skip sending empty responses. The LLM occasionally returns empty
    // content (e.g., when the input is too technical or the context is exhausted)
    // and no list — Telegram rejects empty message text with HTTP 400.
    if reply_body.trim().is_empty() {
        warn!(
            conversation_id = %dispatch.session.conversation,
            "LLM returned empty response, sending fallback"
        );
        let empty_reply = dispatch
            .resources
            .mcp
            .messaging_strings_registry
            .get(KEY_EMPTY_REPLY, &dispatch.locale);
        send_error_reply(&dispatch, &channel_config, &empty_reply).await;
        return;
    }

    // Fidelity negotiation: channels that publish media get the charts as
    // images, everyone else keeps the prose that explains them.
    let charts = plan_media(
        &VizDelivery {
            target: viz_target(
                dispatch.session.conversation.clone(),
                dispatch.auth_result.user_id.to_string(),
                dispatch.user_tenant_id,
                dispatch_result.assistant_message.id.clone(),
            ),
            stored_blocks: dispatch_result.assistant_message.content_blocks.as_deref(),
            channel_type: dispatch.channel_type,
            locale: &dispatch.locale,
            base_url: &dispatch.resources.common.config.base_url,
            press_enabled: dispatch.resources.common.photograveur.is_enabled(),
        },
        &dispatch.resources.auth.admin_jwt_secret,
    );

    deliver_reply(
        &dispatch,
        messaging_agui.as_ref(),
        &channel_config,
        reply_body,
        dispatch_result.turn_id,
        charts,
    )
    .await;

    // Dropping the wiring here aborts the consumer task (if still
    // live) and releases the RunScope so the registry entry is
    // cleaned up. Held until after `deliver_reply` so any
    // last events the pipeline emitted on the way out still render.
    drop(messaging_agui);

    // Held until here to serialize dispatches for the same conversation
    drop(dispatch_guard);
    evict_idle_dispatch_lock(&dispatch.session.conversation, &lock);
}

/// Remove the per-conversation lock from the shared map if no other task still holds it.
///
/// Prevents unbounded growth of `CONVERSATION_DISPATCH_LOCKS` under
/// high conversation cardinality while staying safe: if a concurrent
/// dispatch cloned the `Arc` before we got here, the strong count
/// exceeds 2 and we leave the entry in place. The next waiter will
/// simply reinsert on a later call if it was already evicted.
fn evict_idle_dispatch_lock(conversation_id: &str, local: &Arc<TokioMutex<()>>) {
    // Strong references: the one in the DashMap entry + `local` held here.
    // Any higher count means another dispatch task is waiting on this lock.
    CONVERSATION_DISPATCH_LOCKS.remove_if(conversation_id, |_, stored| {
        Arc::ptr_eq(stored, local) && Arc::strong_count(stored) <= 2
    });
}

/// Send a user-facing error message when LLM dispatch fails or returns empty content.
///
/// Ensures the user always gets feedback instead of silence when something goes wrong.
async fn send_error_reply(dispatch: &PendingDispatch, channel_config: &ChannelConfig, body: &str) {
    let reply_target =
        reply_recipient(dispatch.conversation_id.as_deref(), &dispatch.sender_id).to_owned();

    let outgoing = OutgoingMessage {
        channel_type: dispatch.channel_type,
        recipient_id: reply_target,
        content: MessageContent::Text {
            body: body.to_owned(),
        },
        // Error replies emit a fresh turn id — the chat pipeline never
        // reached the point where a turn would have been recorded, so
        // there's no upstream id to thread. The platform records a
        // terminal failed turn under this id so operators can correlate.
        turn_id: CanotTurnId::new(),
        reply_to: Some(dispatch.channel_message_id.clone()),
        thread_id: dispatch.thread_id.clone(),
    };

    send_outbound_response(dispatch, channel_config, &outgoing).await;
}

/// Extract real token counts from provider usage, or estimate from content length.
///
/// Returns `(prompt_tokens, completion_tokens)` as i64 for direct use in usage recording.
/// When real usage is unavailable (CLI-based providers), estimates from the user message
/// and completion text using a character-based heuristic.
fn estimate_or_extract_messaging_tokens(
    usage: Option<&TokenUsage>,
    user_text: &str,
    completion_text: &str,
) -> (i64, i64) {
    usage.map_or_else(
        || {
            let (est_prompt, est_completion) = estimate_chat_tokens(user_text, completion_text);
            debug!(
                est_prompt,
                est_completion,
                "Using estimated token counts for messaging (provider returned no usage)"
            );
            (i64::from(est_prompt), i64::from(est_completion))
        },
        |u| (i64::from(u.prompt_tokens), i64::from(u.completion_tokens)),
    )
}

/// Increment usage counters (messages, tokens, tool calls) after a messaging dispatch
async fn increment_messaging_usage_counters(dispatch: &PendingDispatch, result: &DispatchResult) {
    // Record against tier defaults even when admin config is absent, so
    // counters stay consistent with the always-on web enforcement path.
    let admin_config: &dyn AdminConfigLookup =
        match dispatch.resources.coach.admin_config.as_deref() {
            Some(c) => c,
            None => default_admin_config(),
        };

    let tenant_id_str = dispatch.channel_tenant_id.to_string();
    let usage_svc = UsageCounterService::new(
        dispatch.resources.common.repos.usage_counters.as_ref(),
        admin_config,
    );

    // Use real token counts when available, fall back to estimation for CLI providers
    let (est_prompt, est_completion) = estimate_or_extract_messaging_tokens(
        result.usage.as_ref(),
        &dispatch.text_content,
        &result.content,
    );
    let total_tokens = est_prompt + est_completion;

    // Build list of (counter_type, amount) pairs to increment
    let mut counters: Vec<(&str, i64)> = vec![("daily_messages", 1), ("weekly_messages", 1)];
    if total_tokens > 0 {
        counters.push(("daily_tokens", total_tokens));
        counters.push(("weekly_tokens", total_tokens));
    }

    for (counter_type, amount) in counters {
        if let Err(e) = usage_svc
            .increment(
                &tenant_id_str,
                &dispatch.session.user_id,
                counter_type,
                amount,
            )
            .await
        {
            error!("Failed to increment {counter_type} counter for messaging: {e}");
        }
    }
}

/// Load channel config, send outbound message, and persist the result
async fn send_outbound_response(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    outgoing: &OutgoingMessage,
) {
    let db: &dyn MessagingRepository = dispatch.resources.common.repos.messaging.as_ref();

    match dispatch.adapter.send(outgoing, channel_config).await {
        Ok(receipt) => {
            let channel_msg_id = receipt.channel_message_id.as_deref().unwrap_or("");
            info!(
                channel_message_id = %channel_msg_id,
                channel = %dispatch.channel,
                "Outbound message sent successfully"
            );
            persist_outbound_message(db, dispatch, channel_msg_id, outgoing).await;
        }
        Err(e) => {
            warn!(
                error = %e,
                channel = %dispatch.channel,
                "Failed to send outbound message, enqueuing for retry"
            );
            enqueue_failed_outbound(db, dispatch, outgoing).await;
        }
    }
}

/// Load and deserialize a channel config for outbound sending
pub(super) async fn load_channel_config(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
) -> Option<ChannelConfig> {
    let config = match db.get_channel_config(tenant_id, channel).await {
        Ok(Some(cfg)) => cfg,
        Ok(None) => {
            warn!(channel = %channel, "No channel config found for outbound send");
            return None;
        }
        Err(e) => {
            error!(error = %e, "Failed to load channel config for outbound");
            return None;
        }
    };

    match serde_json::from_value::<ChannelConfig>(config) {
        Ok(c) => Some(c),
        Err(e) => {
            error!(error = %e, "Failed to deserialize channel config");
            None
        }
    }
}

/// Persist an outbound message after successful delivery
async fn persist_outbound_message(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    channel_message_id: &str,
    outgoing: &OutgoingMessage,
) {
    let out_msg_id = Uuid::new_v4().to_string();
    let body = content_body_text(&outgoing.content);
    let correlation_str = outgoing.turn_id.to_string();
    let out_params = InsertMessageParams {
        id: &out_msg_id,
        // Outbound row shares the session/conversation tenant so the whole turn
        // (inbound + assistant) is readable as one unit under one tenant.
        tenant_id: dispatch.session_tenant_id,
        session_id: &dispatch.session.session_id,
        direction: "outbound",
        channel_type: &dispatch.channel,
        channel_message_id,
        sender_id: "pierre",
        content_type: "text",
        content_body: body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: None,
    };
    if let Err(e) = db.insert_message(&out_params).await {
        error!(error = %e, "Failed to persist outbound message");
    }
}

/// Enqueue a failed outbound message for retry delivery.
///
/// Thin wrapper over the shared [`outbound_retry::enqueue_failed_outbound`] helper
/// (the single source of truth, also driving the backfill-completion push): it
/// supplies this dispatch's tenants / session / channel and logs a failure. The
/// message row lands on the session tenant; the queue row lands on the
/// channel/bot tenant so the retry worker loads the right channel config.
async fn enqueue_failed_outbound(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    outgoing: &OutgoingMessage,
) {
    if let Err(e) = outbound_retry::enqueue_failed_outbound(
        db,
        dispatch.adapter.as_ref(),
        outgoing,
        &outbound_retry::FailedOutbound {
            message_tenant_id: dispatch.session_tenant_id,
            queue_tenant_id: dispatch.channel_tenant_id,
            session_id: &dispatch.session.session_id,
            user_id: Some(dispatch.session.user_id.as_str()),
            channel: &dispatch.channel,
        },
    )
    .await
    {
        error!(error = %e, channel = %dispatch.channel, "Failed to enqueue outbound for retry");
    }
}
