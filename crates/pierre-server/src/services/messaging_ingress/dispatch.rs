// ABOUTME: LLM dispatch + outbound delivery + retry queue for messaging turns
// ABOUTME: dispatch_and_respond is the per-turn orchestrator; helpers handle send/persist/enqueue

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::env;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pierre_core::models::messaging::{ChannelConfig, MessageContent, OutgoingMessage};
use pierre_core::models::{ColorScheme, ConversationTurnId, TenantId, TranscriptSpeaker};
use pierre_database::backends::{InsertMessageParams, MessagingRepository};
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::scene_publisher::MessagingScenePublisher;
use pierre_chat_pipeline::{self, CommandPersistence, PipelineHooks, ServedTurn, TurnRequest};
use pierre_contremaitre::messaging_strings::{
    format_template, MessagingStringsRegistry, KEY_COACH_PROPOSAL_FOOTER,
    KEY_COACH_PROPOSAL_WELCOME, KEY_COACH_PROPOSAL_WELCOME_GENERIC, KEY_EMPTY_REPLY,
    KEY_ERROR_GENERIC, KEY_QUOTA_EXCEEDED, KEY_TURN_INTERRUPTED,
};
use pierre_core::errors::AppError;
use pierre_routes_coaches::coaches::{build_coach_proposal, ProposedCoach, SportProfileSummary};
use pierre_services::analytics::hash_id;

use super::addressing::reply_recipient;
use super::agui::{setup_messaging_agui, MessagingAgUiWiring};
use super::block_render::{render_reply, RenderedReply};
use super::connect;
use super::identity_leak_notify::{emit_identity_leak, LeakContext};
use super::intake;
use super::turn_guard::{
    acquire_dispatch_lock, evict_idle_dispatch_lock, new_correlation_id, run_bounded, run_guarded,
    TurnInterruption, TurnOutcome,
};
use super::{build_messaging_profile, content_body_text, outbound_retry, PendingDispatch};
use pierre_services::onboarding_gate::user_has_connected_provider;
use pierre_services::user_status_gate::messaging_key_for_status;

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

/// Open the messaging intake behind a served turn, if one is owed.
///
/// Sits beside the connect card and the coach proposal because it is the same
/// kind of thing: something the platform wants to say, appended to the reply
/// the athlete actually asked for rather than replacing it. Once the first
/// question is out, the athlete's answers are handled inline by
/// `intake::try_handle_intake`, which replies with the next question directly —
/// an answer deserves the next question, not a coaching turn.
///
/// Best-effort throughout: a failed send costs the intake this turn, and the
/// next conversation opens it again.
async fn maybe_send_intake_question(dispatch: &PendingDispatch, channel_config: &ChannelConfig) {
    let Some(question) = intake::try_build_first_question(intake::FirstQuestionParams {
        resources: &dispatch.resources,
        tenant_id: dispatch.session_tenant_id,
        conversation_id: &dispatch.session.conversation,
        channel_type: dispatch.channel_type,
        sender_id: &dispatch.sender_id,
        user_id: dispatch.auth_result.user_id,
        locale: &dispatch.locale,
        is_direct_message: !dispatch.is_group_chat,
    })
    .await
    else {
        return;
    };

    if let Err(e) = dispatch.adapter.send(&question, channel_config).await {
        warn!(error = %e, "intake: opening question failed to send; will re-open next conversation");
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
///
/// `prose` arrives already split to what one message on this channel carries,
/// so a long answer is several bubbles rather than a truncated one. They go
/// out in order, and the status placeholder — which is a single editable
/// message — settles the first of them.
async fn deliver_reply(
    dispatch: &PendingDispatch,
    messaging_agui: Option<&MessagingAgUiWiring>,
    channel_config: &ChannelConfig,
    prose: Vec<String>,
    turn_id: ConversationTurnId,
    attachments: Vec<MessageContent>,
    assistant_message_id: &str,
) {
    let mut parts = prose.into_iter();
    let Some(first) = parts.next() else {
        return;
    };

    // AG-UI streams the reply by editing the status message it already posted,
    // so when it finalizes, the first part is delivered and a second send would
    // duplicate it. It settles that PART only — the continuation and the
    // attachments below are different messages and still have to go out.
    //
    // This used to `return` here, which silently dropped every chart on any
    // channel with status streaming. Slack has it on always, so a granted coach
    // could emit a perfectly valid chart, have it lifted and stored, and the
    // athlete would still only ever see the paragraph (observed 2026-08-20).
    let first_sent_by_agui = match messaging_agui {
        Some(wiring) => {
            wiring
                .finalize_reply(&first, dispatch, Some(assistant_message_id))
                .await
        }
        None => false,
    };

    if !first_sent_by_agui {
        send_outbound_response(
            dispatch,
            channel_config,
            &reply_message(dispatch, turn_id, MessageContent::Text { body: first }),
            Some(assistant_message_id),
        )
        .await;
    }

    for continuation in parts {
        send_outbound_response(
            dispatch,
            channel_config,
            &reply_message(
                dispatch,
                turn_id,
                MessageContent::Text { body: continuation },
            ),
            Some(assistant_message_id),
        )
        .await;
    }

    // Charts and controls follow the prose, in the order the coach placed
    // them. Sent as separate messages because no channel here renders several
    // images inside one text bubble, and the prose must land first — it is
    // what makes the pictures mean something.
    for attachment in attachments {
        send_outbound_response(
            dispatch,
            channel_config,
            &reply_message(dispatch, turn_id, attachment),
            Some(assistant_message_id),
        )
        .await;
    }
}

/// Address one piece of the assistant reply back at the conversation it came
/// from.
///
/// Uses `conversation_id` (channel/chat/thread) as the reply target when
/// available; falls back to `sender_id` for DM-only platforms (e.g.
/// `WhatsApp`).
///
/// Every piece carries the turn id from the inbound utterance, so a consumer
/// inspecting the `DeliveryReceipt` can look up the full turn trace via
/// `/internal/conversation-turn` — and a split reply reads as one turn rather
/// than several. `.into()` bridges pierre-core's newtype to canot's. Each is
/// threaded under the athlete's message so a client that groups replies keeps
/// the whole answer together.
pub(super) fn reply_message(
    dispatch: &PendingDispatch,
    turn_id: ConversationTurnId,
    content: MessageContent,
) -> OutgoingMessage {
    OutgoingMessage {
        channel_type: dispatch.channel_type,
        recipient_id: reply_recipient(dispatch.conversation_id.as_deref(), &dispatch.sender_id)
            .to_owned(),
        content,
        turn_id: turn_id.into(),
        reply_to: Some(dispatch.channel_message_id.clone()),
        thread_id: dispatch.thread_id.clone(),
    }
}

/// Default wall-clock ceiling on one messaging turn.
///
/// Not a latency target — it is the ceiling above every bound the turn runs
/// under, so that reaching it means the turn found something with no bound of
/// its own. The deepest of those is the whole-turn ACP prompt cap
/// (`EMBACLE_ACP_PROMPT_TIMEOUT_SECS`, 300s on dev), and a turn legitimately
/// opens more than one ACP session: the tool loop runs in one, the final
/// answer in another, and embacle retries an empty answer on a third. Three
/// full-length prompts plus delivery is the honest worst case a healthy turn
/// can reach, so the ceiling sits just above it.
///
/// The cost of setting it too low is worse than setting it too high: too low
/// replaces an answer that was still coming with a notice saying it is not,
/// while too high only delays a placeholder that was already going to close.
const DEFAULT_TURN_WATCHDOG_SECS: u64 = 960;

/// Resolve the per-turn watchdog from `MESSAGING_TURN_WATCHDOG_SECS`, falling
/// back to [`DEFAULT_TURN_WATCHDOG_SECS`].
///
/// Env-driven for the same reason its siblings under it are
/// (`EMBACLE_ACP_PROMPT_TIMEOUT_SECS`, `EMBACLE_ACP_MESSAGE_TIMEOUT_SECS`):
/// the number that is right depends on the deployed provider chain, and
/// re-tuning it should not need a binary. Zero or unparseable falls back
/// rather than disarming the watchdog — an unbounded turn is the defect this
/// exists to end.
fn turn_watchdog() -> Duration {
    let secs = env::var("MESSAGING_TURN_WATCHDOG_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_TURN_WATCHDOG_SECS);
    Duration::from_secs(secs)
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
    let (correlation_id, short_id) = new_correlation_id();
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
    let template = dispatch
        .resources
        .mcp
        .messaging_strings_registry
        .get(KEY_ERROR_GENERIC, &dispatch.locale);
    let user_message = format_template(&template, &[&short_id]);
    send_plain_reply(dispatch, channel_config, &user_message).await;
}

/// Close a turn that was cut short before it produced anything.
///
/// The placeholder is the whole point. On a channel with status streaming the
/// athlete is looking at "génération de la réponse…", and that text is only
/// ever *replaced* — by the finished reply, at the end of a turn that reaches
/// its end. A turn that does not reach its end leaves it standing, and a
/// standing placeholder is indistinguishable from a slow answer, so the
/// athlete waits for something that is never coming. That is what the
/// 2026-08-26 group chart ask still shows (registre#109).
///
/// So the notice goes *into* the placeholder when there is one, which both
/// tells the athlete and retires the lie. Where there is no placeholder
/// (`WhatsApp`, Messenger — neither can edit a sent message) it is a plain
/// reply, which is the same thing those channels do for every other notice.
async fn close_interrupted_turn(
    dispatch: &PendingDispatch,
    messaging_agui: Option<&MessagingAgUiWiring>,
    channel_config: &ChannelConfig,
    cause: TurnInterruption,
    elapsed_ms: u64,
) {
    warn!(
        cause = cause.as_str(),
        elapsed_ms = elapsed_ms,
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
        turn_id = %dispatch.turn_id,
        "messaging turn interrupted before it produced a reply"
    );
    info!(
        target: "notify",
        event = "messaging.error",
        tenant_id = %dispatch.channel_tenant_id,
        channel = %dispatch.channel,
        error_type = cause.as_str(),
        "messaging error"
    );

    let notice = dispatch
        .resources
        .mcp
        .messaging_strings_registry
        .get(KEY_TURN_INTERRUPTED, &dispatch.locale);

    // `None` for the assistant message id: no assistant row exists — the turn
    // never got far enough to write one — so an emoji on this notice resolves
    // to nothing to rate, exactly as for every other non-coaching reply.
    let closed_placeholder = match messaging_agui {
        Some(wiring) => {
            // Silence progress first: this notice is terminal, and a queued
            // status event rendered after it would put the athlete back to
            // waiting on an answer that is not coming.
            wiring.stop_status_updates();
            wiring.finalize_reply(&notice, dispatch, None).await
        }
        None => false,
    };
    if !closed_placeholder {
        send_plain_reply(dispatch, channel_config, &notice).await;
    }
}

/// Send the localized denial for a quota or rate-limit refusal.
///
/// WARN, not ERROR, and no notify event: a budget refusing a turn is the
/// user's plan working as designed, and paging on-call for it teaches them to
/// tune the channel out. The reply key comes from the same status→string map
/// the channel-auth denial path uses (`messaging_key_for_status`), so both
/// refusal surfaces speak with one voice; the generic-quota key is the
/// fallback only for a quota-shaped error the map somehow does not know.
async fn send_quota_denial_reply(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    err: &AppError,
) {
    // `user_tenant_id`, not `channel_tenant_id`: the gate checks — and
    // `increment_messaging_usage_counters` records — under the requester's own
    // tenant, so that is where the `usage_counters` row that tripped lives. In
    // a group the bot's tenant owns the channel and differs, and naming it here
    // (as the neighbouring dispatch-failure event does) sends an operator
    // resetting a dev counter to a tenant whose budget was never touched.
    warn!(
        error = %err,
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
        tenant_id = %dispatch.user_tenant_id,
        "messaging turn refused by quota/rate limit"
    );
    let key = messaging_key_for_status(err.code).unwrap_or(KEY_QUOTA_EXCEEDED);
    let body = dispatch
        .resources
        .mcp
        .messaging_strings_registry
        .get(key, &dispatch.locale);
    send_plain_reply(dispatch, channel_config, &body).await;
}

/// Maximum ambient-transcript lines injected into a group turn's prompt.
const AMBIENT_TRANSCRIPT_MAX_LINES: usize = 25;

/// Maximum characters kept per ambient-transcript line (grapheme-unaware
/// char truncation is fine for prompt context).
const AMBIENT_TRANSCRIPT_MAX_LINE_CHARS: usize = 240;

/// Build the speaker-labeled ambient transcript for a group turn.
///
/// Reads the group's shared room transcript (`group_transcript_entries`) —
/// the same surface-neutral read model web and mobile members read — through
/// the consent-gated visibility query, with the requesting member as the
/// viewer: an unconsented peer's content never enters this member's prompt.
/// Member rows are labeled with the sender's display name, coach rows
/// "Coach". The triggering message is not yet in the transcript (the turn
/// pipeline fans it out at persistence), so nothing is excluded here.
/// Returns `None` when the room has no other recent messages, so DM-shaped
/// groups cost no prompt tokens.
async fn build_group_ambient_context(dispatch: &PendingDispatch) -> Option<String> {
    let conversation = dispatch
        .resources
        .common
        .repos
        .chat
        .get_conversation(
            &dispatch.session.conversation,
            &dispatch.session.user_id,
            dispatch.session_tenant_id,
        )
        .await
        .ok()??;
    let group_id = conversation.group_id?;

    let limit = i64::try_from(AMBIENT_TRANSCRIPT_MAX_LINES).unwrap_or(25);
    let entries = match dispatch
        .resources
        .common
        .repos
        .groups
        .list_transcript_visible_to(&group_id, dispatch.auth_result.user_id, limit)
        .await
    {
        Ok(entries) => entries,
        Err(e) => {
            warn!(error = %e, "ambient transcript load failed; dispatching without it");
            return None;
        }
    };

    // Newest-first from the query; restore chronological order, cap the
    // per-line length.
    let mut lines: Vec<String> = Vec::new();
    let mut label_cache: HashMap<String, String> = HashMap::new();
    for entry in &entries {
        if entry.content.is_empty() {
            continue;
        }
        let label = match entry.speaker {
            TranscriptSpeaker::Coach => "Coach".to_owned(),
            TranscriptSpeaker::Member => {
                let author = entry.author_user_id.to_string();
                speaker_label(dispatch, &author, &mut label_cache).await
            }
        };
        let truncated: String = entry
            .content
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

/// Read the colour scheme the athlete pinned, for the charts this turn mints.
///
/// A messaging chart is fetched by the channel's servers, not the athlete's
/// device, so nothing on the wire can report the scheme the athlete is looking
/// at — the `users.theme` pin is the only signal there is. An athlete who
/// pinned nothing, or whose row cannot be read, gets
/// [`ColorScheme::Dark`]: messaging clients overwhelmingly draw media bubbles
/// on dark, and a chart in the wrong scheme still beats no chart.
async fn athlete_color_scheme(dispatch: &PendingDispatch) -> ColorScheme {
    match dispatch
        .resources
        .common
        .repos
        .users
        .get_global(dispatch.auth_result.user_id)
        .await
    {
        Ok(Some(user)) => ColorScheme::resolve(user.theme.as_deref()),
        Ok(None) => ColorScheme::default(),
        Err(e) => {
            warn!(error = %e, "theme lookup failed for chart minting; painting dark");
            ColorScheme::default()
        }
    }
}

/// Dispatch a message through the LLM pipeline and send the response back via the channel
///
/// Runs as a background task after the webhook has returned HTTP 200.
/// Acquires a per-conversation lock to ensure messages are processed in order.
///
/// The `#[instrument]` span pins `turn_id`, `channel`, and `conversation_id`
/// onto every downstream log line (chat pipeline stages, embacle HTTP call)
/// so an operator can grep a single `turn_id=...` across the whole flow.
///
/// LIMITATION(registre#109): `dispatch_and_respond` runs unbounded — `start` measures the turn
/// but no deadline closes it. The AG-UI status placeholder is only ever edited into a finished
/// reply, so a turn killed mid-generation leaves that placeholder open in the room forever,
/// indistinguishable to the athlete from a slow answer.
#[tracing::instrument(
    skip(dispatch),
    fields(
        turn_id = %dispatch.turn_id,
        channel = %dispatch.channel,
        conversation_id = %dispatch.session.conversation,
    )
)]
pub async fn dispatch_and_respond(dispatch: PendingDispatch) {
    let lock = acquire_dispatch_lock(&dispatch.session.conversation);
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

    let profile = build_messaging_profile(
        &dispatch.resources,
        dispatch.channel_type,
        dispatch.locale.clone(),
    );

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

    // Register an AG-UI run for this messaging turn so the in-process
    // channel-side status adapter can subscribe via
    // `resources.agui_registry.subscribe_self(run_id)` and render pipeline
    // progress to Telegram/Slack/Discord. The run id never leaves the
    // process. Scope drops at function exit, auto-unregistering on success,
    // error, or panic.
    //
    // `setup_messaging_agui` also opens an in-channel status adapter
    // (Telegram editMessageText / Slack chat.update / Discord PATCH
    // messages) and spawns a background consumer that mirrors each
    // AG-UI event as a `set_status` call so the user sees the pipeline
    // stage in real time.
    let messaging_agui = setup_messaging_agui(&dispatch, &channel_config).await;
    // Charts: this channel cannot draw a spec, so the pipeline asks the
    // publisher for a signed image URL per block once the assistant row is
    // durable, and the envelope carries them as `SceneImage` blocks.
    let scene_publisher = MessagingScenePublisher::new(
        Arc::clone(&dispatch.resources),
        profile.render,
        athlete_color_scheme(&dispatch).await,
    );
    let hooks = PipelineHooks {
        agui: messaging_agui.as_ref().map(|w| w.run()),
        scene_publisher: Some(&scene_publisher),
        ..PipelineHooks::none()
    };

    // Group turns get the room's recent cross-member transcript; DMs skip
    // the lookup entirely.
    let ambient_context = if dispatch.is_group_chat {
        build_group_ambient_context(&dispatch).await
    } else {
        None
    };
    let request = TurnRequest {
        conversation_id: dispatch.session.conversation.clone(),
        user_id: dispatch.auth_result.user_id,
        // The conversation lives under the session tenant (the athlete's own
        // for a DM, the bot's for a shared room so every member reads one
        // transcript).
        conversation_tenant_id: dispatch.session_tenant_id,
        // Tools, provider credentials AND usage counters all run under the
        // athlete's own tenant, never the bot's (registre#9).
        tool_tenant_id: dispatch.user_tenant_id,
        content: dispatch.text_content.clone(),
        // Reuse the turn id canot generated at the webhook boundary: a single
        // inbound message plus its full LLM/tool chain is one turn, and
        // canot's log spans already key off this id.
        turn_id: dispatch.turn_id,
        ambient_context,
        channel_type: &dispatch.channel,
        is_direct_message: !dispatch.is_group_chat,
        // The ingress answered any slash command before this turn was queued,
        // so these only say what the messaging surface's answer is: the
        // ambient group stands in for an unbound DM, and a room keeps only the
        // replies it saw.
        ambient_group_fallback: true,
        command_persistence: if dispatch.is_group_chat {
            CommandPersistence::RoomVisibleOnly
        } else {
            CommandPersistence::Always
        },
        sender_id: Some(&dispatch.sender_id),
        hooks,
    };

    let ctx = dispatch.resources.chat_pipeline_context();
    // Panic boundary: a bug in any pipeline stage must unwind into a structured
    // failure for *this* turn (graceful user reply + correlation-id log), never
    // escape the spawned task.
    //
    // `run_bounded` adds the two endings the turn has no other source for: a
    // wall-clock ceiling, and the shutdown drain. Both leave the athlete with
    // a closed placeholder instead of an open one.
    let drain = dispatch.resources.common.turns.drain_token();
    let dispatch_result = match run_bounded(
        run_guarded(pierre_chat_pipeline::execute(&ctx, request, &profile)),
        turn_watchdog(),
        &drain,
    )
    .await
    {
        TurnOutcome::Delivered(served) => served,
        // A quota or rate-limit refusal is the user's plan speaking, not a
        // fault: send the localized denial instead of the generic apology,
        // and log at WARN — paging on-call for a budget working as designed
        // is how real faults get tuned out.
        TurnOutcome::QuotaDenied(e) => {
            send_quota_denial_reply(&dispatch, &channel_config, &e).await;
            return;
        }
        // Includes a panic caught inside any pipeline stage: the athlete gets
        // an apology carrying a correlation id instead of silence.
        TurnOutcome::Failed(e) => {
            report_dispatch_failure(&dispatch, &channel_config, &e).await;
            return;
        }
        // Cut short with nothing to deliver. Unlike every arm above it,
        // this one has a placeholder still open on the channel, so the
        // wiring goes with it — the notice replaces the placeholder
        // rather than arriving underneath it.
        //
        // Safe cast: a turn's elapsed milliseconds cannot approach u64::MAX.
        #[allow(clippy::cast_possible_truncation)]
        TurnOutcome::Interrupted(cause) => {
            close_interrupted_turn(
                &dispatch,
                messaging_agui.as_ref(),
                &channel_config,
                cause,
                start.elapsed().as_millis() as u64,
            )
            .await;
            return;
        }
    };

    // A slash command reaches the turn service only when the ingress did not
    // already answer it — the catalog was unavailable when the message
    // arrived, say. Its reply is account state, not coaching: send the text
    // and stop, rather than handing the athlete an LLM answer to a command.
    let dispatch_result = match dispatch_result {
        ServedTurn::Pipeline(envelope) => *envelope,
        ServedTurn::Command { command, .. } => {
            send_plain_reply(&dispatch, &channel_config, &command.text).await;
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
        model = %dispatch_result.telemetry.model,
        "messaging response sent"
    );

    // Security signal: the reply identified as the underlying model/provider and
    // was withheld at the response boundary. The athlete received the canned
    // withheld string, not the leak, so nothing about this turn looks unusual
    // from outside — the alert is the only thing that makes it visible.
    emit_identity_leak(
        &dispatch_result,
        &LeakContext {
            conversation_tenant_id: dispatch.session_tenant_id,
            conversation_id: &dispatch.session.conversation,
            channel: &dispatch.channel,
        },
    );

    // Per-LLM-call `llm_usage` rows are written inline by the chat pipeline's
    // `LlmCallRecorder`, and the daily/weekly counters the next turn's quota
    // check reads were incremented by the turn service under the athlete's own
    // tenant. Nothing to record here.

    // Lay the envelope's blocks out for this channel. Every rendering decision
    // was already made against the surface's capabilities inside the pipeline
    // — what a text channel cannot draw is folded into the prose, what it can
    // arrives as its own block — so this is layout and splitting, nothing more.
    let rendered = render_reply(
        &profile.render,
        &dispatch_result.assistant,
        &dispatch.resources.mcp.messaging_strings_registry,
        &dispatch_result.locale,
    );

    // Guard: skip sending empty responses. The LLM occasionally returns empty
    // content (e.g., when the input is too technical or the context is exhausted)
    // and no list — Telegram rejects empty message text with HTTP 400.
    //
    // "And no list" is the whole condition, and the code used to test only the
    // first half. A reply that is one chart and no prose is a complete answer to
    // "fais-moi un graphique", and the athlete was told the coach could not
    // formulate a response while the chart it had drawn was discarded. Both
    // halves empty is the case Telegram actually rejects.
    if rendered.is_empty() {
        warn!(
            conversation_id = %dispatch.session.conversation,
            // The corpus reports these as "the coach returned nothing", which
            // is true of the delivered reply and says nothing about where it
            // was lost. These fields separate a model that produced no blocks
            // from a turn whose blocks this surface could not draw.
            blocks = dispatch_result.assistant.blocks.len(),
            finish_reason = dispatch_result.assistant.message.finish_reason.as_deref().unwrap_or("none"),
            content_len = dispatch_result.assistant.message.content.len(),
            "LLM returned empty response, sending fallback"
        );
        // The turn's own language, not the athlete's stored preference: the
        // fallback stands in for the reply that did not arrive, so it speaks
        // the language that reply would have been written in.
        let empty_reply = dispatch
            .resources
            .mcp
            .messaging_strings_registry
            .get(KEY_EMPTY_REPLY, &dispatch_result.locale);
        send_plain_reply(&dispatch, &channel_config, &empty_reply).await;
        return;
    }

    let RenderedReply { prose, attachments } = rendered;
    deliver_reply(
        &dispatch,
        messaging_agui.as_ref(),
        &channel_config,
        prose,
        dispatch_result.turn_id,
        attachments,
        &dispatch_result.assistant.message.id,
    )
    .await;

    // Dropping the wiring here aborts the consumer task (if still
    // live) and releases the RunScope so the registry entry is
    // cleaned up. Held until after `deliver_reply` so any
    // last events the pipeline emitted on the way out still render.
    drop(messaging_agui);

    // After the answer, never before it. The intake rides *behind* a served
    // turn — an athlete who asks "j'attaque la tourbière, c'est bon?" gets
    // answered and is then handed the form, not the reverse. Running it up at
    // the top put the question above a coaching reply that was delivered by
    // editing a placeholder opened later, so the answer was pinned below the
    // form in the thread and the opener's promise ("une minute maintenant, et
    // le coaching qui suit est plus précis") was already broken by the time it
    // was read (production Telegram, 2026-08-28).
    //
    // Inside the conversation lock, so the ledger write that records the probe
    // still lands before the athlete can answer it. Below the early returns for
    // quota denial, turn failure and an empty reply, so a turn that produced no
    // coaching no longer trails a form behind nothing — the intake keeps its
    // claim on the next served turn instead, because `try_build_first_question`
    // only asks while nothing has been probed.
    maybe_send_intake_question(&dispatch, &channel_config).await;

    // Held until here to serialize dispatches for the same conversation
    drop(dispatch_guard);
    evict_idle_dispatch_lock(&dispatch.session.conversation, &lock);
}

/// Send one plain-text body back to the athlete, outside the envelope path.
///
/// Everything the platform says when the turn produced no reply to lay out: a
/// failure apology with its correlation id, a quota denial, the empty-reply
/// fallback, a slash-command answer. Each of those is a finished sentence
/// already in the athlete's language, so there is nothing to render — only to
/// address and send.
async fn send_plain_reply(dispatch: &PendingDispatch, channel_config: &ChannelConfig, body: &str) {
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

    // Nothing here is a coaching answer — an apology, a quota denial, a slash
    // command's account state — so the row carries no assistant message id and
    // an emoji on it resolves to nothing to rate.
    send_outbound_response(dispatch, channel_config, &outgoing, None).await;
}

/// Load channel config, send outbound message, and persist the result
async fn send_outbound_response(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    outgoing: &OutgoingMessage,
    assistant_message_id: Option<&str>,
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
            persist_outbound_message(db, dispatch, channel_msg_id, outgoing, assistant_message_id)
                .await;
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
pub(super) async fn persist_outbound_message(
    db: &dyn MessagingRepository,
    dispatch: &PendingDispatch,
    channel_message_id: &str,
    outgoing: &OutgoingMessage,
    assistant_message_id: Option<&str>,
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
        // This send and the assistant row it delivers are the one moment both
        // ids are in hand; stamping it here is what lets an emoji reaction on
        // the channel message resolve back to a message to rate.
        chat_message_id: assistant_message_id,
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
