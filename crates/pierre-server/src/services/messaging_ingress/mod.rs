// ABOUTME: Business logic for messaging ingress: OTP flow, channel linking, session resolution,
// ABOUTME: slash command dispatch, message persistence, LLM dispatch, and outbound response handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Channel reply-recipient addressing: the shared conversation-id-with-user-id-fallback rule.
pub mod addressing;
/// AG-UI run wiring + per-channel status-bridge setup for messaging dispatch.
mod agui;
/// In-chat provider-connect: in-process link-token mint + tappable connect Card.
mod coach_choice;
mod connect;
/// Platform-asked intake: profile type then the PAR-Q+, verbatim and strictly parsed.
mod intake;
/// Resolves a messaging channel's `SurfaceProfile` from the canot descriptor.
pub mod surface;
/// Per-channel fidelity negotiation: cards natively or as rich text, charts as media.
pub mod viz_delivery;

/// Re-exported so the integration suite can pin the strict numeric parse.
pub use coach_choice::parse_choice;
// Re-exported so the emitters keep importing from one place; the negotiator
// itself lives beside the media one because they are the same decision.
pub use viz_delivery::card_or_rich_text;
/// LLM dispatch + outbound delivery + retry queue for messaging turns.
mod dispatch;
/// Channel-linking commands (`/start <code>`, `LINK <code>`) + analytics-consent hydration.
mod linking;
/// Messaging-turn stored-locale resolution (channel link, then user profile).
pub mod locale;
/// In-chat OTP linking flow + logout + supporting helpers.
mod otp;
/// Shared failed-outbound enqueue helper — single source of truth for the retry
/// queue, reused by the synchronous reply path and the backfill-completion push.
pub mod outbound_retry;
mod outbound_send;
/// Inbound emoji reactions mapped onto the shared per-message feedback write.
pub mod reactions;
/// What a shared room is left seeing once a slash command is answered privately.
pub mod room_echo;
/// Session resolution for linked channel users + unlinked-user link-and-prompt.
mod session;
/// Channel framing around the turn service's slash dispatch: channel-link
/// auth and locale in, an addressed `OutgoingMessage` out.
#[cfg(feature = "client-messaging")]
mod slash;
use outbound_send::{send_channel_response, send_private_channel_response};
/// Ambient room-chatter capture into the shared group transcript read model.
mod transcript;

pub use locale::resolve_messaging_locale;

pub(crate) use dispatch::dispatch_and_respond;
/// Lays an assistant turn's blocks out as ordered channel messages, splitting
/// prose past the channel's per-message ceiling.
pub mod block_render;
mod channel_auth_outcome;
pub mod identity_leak_notify;
mod scene_publisher;
/// Per-conversation dispatch ordering + the pipeline panic boundary.
pub mod turn_guard;
use channel_auth_outcome::{
    handle_channel_auth_outcome, resolve_channel_user_email, ChannelAuthOutcomeInputs,
};
use linking::{detect_linking_code, handle_linking_command, LinkingAction};
// Re-exported (not just `use`) so the messaging-reset integration test can
// reach the helper — pierre-server keeps test modules external, not in src/.
pub use otp::is_reset_command;
/// Re-exported so integration tests can assert on an unlinked sender's reply.
///
/// The outbound adapters post to hardcoded hosts, so the built message is the
/// last readable point without a network stub.
pub use otp::start_otp_flow;
use otp::{apply_conversation_recipient, handle_logout, handle_otp_flow, is_logout_command};
/// Re-exported alongside [`start_otp_flow`], and for the same reason.
pub use session::create_link_and_prompt;
use session::{handle_reset, resolve_linked_session, ChannelChatRef};
#[cfg(feature = "client-messaging")]
pub use slash::{room_reply_thread_anchor, slash_reply_should_be_private};
#[cfg(feature = "client-messaging")]
use slash::{try_handle_slash_command, SlashCommandContext};

use pierre_auth::auth::AuthResult;
use pierre_core::models::groups::GroupRespondMode;
use pierre_core::models::messaging::{ChannelType, IncomingMessage, MessageContent};
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::safety::{scan as scan_for_injection, SanitizationOutcome};
use pierre_database::backends::{InsertMessageParams, MessagingRepository};
use pierre_messaging::channel::MessagingChannel;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use serde_json::Value;

use crate::mcp::resources::ServerContext;
use pierre_chat_pipeline::SurfaceProfile;
use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
use pierre_services::analytics::hash_id;

/// Outcome of persisting a single inbound message
pub(crate) enum PersistOutcome {
    /// Message was stored in DB and an LLM dispatch is pending
    StoredWithDispatch(Box<PendingDispatch>),
    /// Message was stored in DB but no LLM dispatch (non-text content)
    StoredNoDispatch,
    /// Handled without the ingress storing it: a link code, an unlinked prompt, or a slash command the turn service persisted
    HandledNotStored,
}

/// Resolved messaging session linking a channel user to a Pierre conversation
pub(crate) struct ResolvedSession {
    /// Messaging session identifier
    pub(crate) session_id: String,
    /// Pierre conversation identifier
    pub(crate) conversation: String,
    /// Pierre user identifier resolved from the channel link
    pub(crate) user_id: String,
}

/// Extract forum topic thread ID from incoming message metadata
///
/// Telegram groups with Topics enabled include `message_thread_id` in each
/// message. This ID must be included in outbound replies so they route to
/// the correct topic thread instead of the main chat.
fn extract_thread_id(metadata: &Value) -> Option<String> {
    metadata
        .get("message_thread_id")
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

/// Build the [`SurfaceProfile`] for the originating messaging channel.
///
/// Nothing here is per-channel: the transport answers come from canot (see
/// [`surface::transport_caps`]) and the prose contract comes from
/// contremaitre, so a channel's real ceiling and its real media/card support
/// reach the pipeline without a table in this repository to keep in sync.
pub(crate) fn build_messaging_profile(
    resources: &Arc<ServerContext>,
    channel_type: ChannelType,
    locale: String,
) -> SurfaceProfile {
    SurfaceProfile::resolve(&surface::messaging_surface_request(
        channel_type,
        locale,
        Some(resources.messaging_context_prompt()),
    ))
}

/// Data needed to dispatch a message through the LLM pipeline after HTTP 200
pub(crate) struct PendingDispatch {
    /// Server resources for LLM access
    pub(crate) resources: Arc<ServerContext>,
    /// Channel adapter for outbound send
    pub(crate) adapter: Arc<dyn MessagingChannel>,
    /// Authenticated principal for the inbound turn.
    ///
    /// Produced by
    /// [`pierre_middleware::auth::McpAuthMiddleware::authenticate_channel`] at
    /// the top of `persist_single_message` — same shape and semantics as the
    /// `AuthResult` the JWT middleware produces for HTTP callers. Carries the
    /// user id, rate-limit state, and the user's own `active_tenant_id` so
    /// downstream stages don't re-derive any of this from the channel link.
    pub(super) auth_result: AuthResult,
    /// Resolved session info
    pub(super) session: ResolvedSession,
    /// Channel/bot-owner tenant — used ONLY for channel-scoped delivery
    /// machinery: the channel-link lookup, the channel-config load, and the
    /// outbound send + its retry queue. The bot credential lives here. Usage
    /// counters deliberately do NOT use it: quota enforcement reads under the
    /// user's tenant, so recording here hid messaging usage from every quota
    /// check (registre#9).
    pub(super) channel_tenant_id: TenantId,
    /// User's own tenant — used for tool execution (OAuth, activities, etc.).
    /// May differ from `channel_tenant_id` when the user belongs to a different
    /// tenant than the bot that owns the webhook.
    pub(super) user_tenant_id: TenantId,
    /// Tenant that OWNS the conversation + messages for this turn. For a DIRECT
    /// message this is `user_tenant_id` (the session lives under the user's own
    /// tenant, aligning chat history with the activity cache and letting the
    /// backfill push find it); for a GROUP session it is `channel_tenant_id` (a
    /// shared group must resolve to one tenant for members who may span tenants).
    /// `resolve_linked_session` created/resumed the session under exactly this
    /// tenant — the conversation read, every message write, and the reset forge
    /// MUST use it, or the pipeline ownership check (`get_conversation` filtering
    /// on `tenant_id`) misses and the turn fails with "Conversation not found".
    pub(super) session_tenant_id: TenantId,
    /// Channel type enum
    pub(super) channel_type: ChannelType,
    /// Channel name string (e.g., "slack")
    pub(super) channel: String,
    /// Original sender to reply to
    pub(super) sender_id: String,
    /// Channel-specific conversation/thread identifier (channel ID, chat ID, etc.)
    pub(super) conversation_id: Option<String>,
    /// Text content to dispatch
    pub(super) text_content: String,
    /// Channel-native message ID for reply/thread context (Slack ts, Telegram `message_id`)
    pub(super) channel_message_id: String,
    /// Forum topic thread ID (Telegram Topics `message_thread_id`)
    pub(super) thread_id: Option<String>,
    /// `true` when the turn originated in a shared group chat (not a DM).
    /// Group turns get the room's recent ambient transcript injected into
    /// the prompt, since each member's conversation history holds only
    /// their own exchanges with the coach.
    pub(super) is_group_chat: bool,
    /// The athlete's stored BCP-47 locale for this channel, resolved via
    /// [`resolve_messaging_locale`] when the dispatch is enqueued.
    ///
    /// The language of everything the platform says *around* a turn: the
    /// status placeholder, the connect card, an error apology, a quota
    /// denial. The turn's own language is refined from the athlete's message
    /// inside the turn service and comes back on
    /// [`pierre_chat_pipeline::TurnEnvelope::locale`].
    pub(super) locale: String,
    /// Conversation-turn correlation identifier set by canot's webhook
    /// adapter on [`IncomingMessage::turn_id`]. Threaded through the
    /// pipeline so every LLM call, tool invocation, and outbound reply
    /// shares one id end-to-end.
    pub(super) turn_id: ConversationTurnId,
}

/// Persist inbound messages, handling linking, OTP, logout, slash commands, and session resolution
///
/// Returns (`stored_count`, `pending_dispatches`) — the dispatches are processed
/// asynchronously after the webhook returns HTTP 200.
pub(crate) async fn persist_inbound(
    resources: &Arc<ServerContext>,
    channel: &str,
    tenant_id: TenantId,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    messages: &[IncomingMessage],
) -> (usize, Vec<PendingDispatch>) {
    let mut stored_count: usize = 0;
    let mut pending_dispatches = Vec::new();

    for message in messages {
        match persist_single_message(
            resources,
            channel,
            tenant_id,
            channel_type,
            adapter,
            message,
        )
        .await
        {
            Ok(PersistOutcome::StoredWithDispatch(dispatch)) => {
                stored_count += 1;
                pending_dispatches.push(*dispatch);
            }
            Ok(PersistOutcome::StoredNoDispatch) => {
                stored_count += 1;
            }
            Ok(PersistOutcome::HandledNotStored) | Err(()) => {}
        }
    }

    (stored_count, pending_dispatches)
}

/// Emit the `messaging.intent` product notify event for a recognised intent.
///
/// `user_id` is the raw (un-hashed) Pierre user id, or — for pre-link intents
/// that have no resolved user yet — the `{channel}:{sender_id}` distinct id the
/// provider will hash. `intent_type` is one of `link_code`, `otp_flow`,
/// `logout`, `normal_chat`.
fn emit_messaging_intent(user_id: &str, tenant_id: TenantId, channel: &str, intent_type: &str) {
    info!(
        target: "notify",
        event = "messaging.intent",
        user_id = %user_id,
        tenant_id = %tenant_id,
        channel = %channel,
        intent_type = intent_type,
        "messaging intent recognised"
    );
}

/// Emit the `messaging.message_received` product notify event for a stored
/// inbound message.
fn emit_message_received(user_id: &str, tenant_id: TenantId, channel: &str, content_type: &str) {
    info!(
        target: "notify",
        event = "messaging.message_received",
        user_id = %user_id,
        tenant_id = %tenant_id,
        channel = %channel,
        content_type = %content_type,
        "messaging message received"
    );
}

/// Emit the `messaging.unlinked_prompted` operational notify event.
///
/// Pre-link event — there is no Pierre user yet. Operational tier: the sink
/// keys on the hashed tenant and never forwards a user dimension, so we emit
/// `tenant_id` inline and omit `user_id`. `prompt_type` is `otp` or `link_url`.
fn emit_unlinked_prompted(tenant_id: TenantId, channel: &str, prompt_type: &str) {
    info!(
        target: "notify",
        event = "messaging.unlinked_prompted",
        tenant_id = %tenant_id,
        channel = %channel,
        prompt_type = %prompt_type,
        "unlinked user prompted to link"
    );
}

/// Inputs for [`dispatch_slash_command_if_any`], bundled into a struct so the
/// helper stays within clippy's argument-count budget — the channel / tenant /
/// auth / session context it needs is intrinsically wide.
struct SlashDispatchInputs<'a> {
    resources: &'a Arc<ServerContext>,
    db: &'a dyn MessagingRepository,
    channel: &'a str,
    channel_type: ChannelType,
    tenant_id: TenantId,
    adapter: &'a Arc<dyn MessagingChannel>,
    message: &'a IncomingMessage,
    auth_result: &'a AuthResult,
    session: &'a ResolvedSession,
    thread_id: Option<String>,
}

/// Dispatch a slash command when the inbound text is one, routing the reply
/// privately in shared rooms (and deleting the command echo) or back into a 1:1
/// DM. Returns `true` when a command was recognized and handled (so the caller
/// stops before storing/LLM dispatch), `false` to fall through to normal chat.
///
/// Extracted from [`persist_single_message`] so that function stays within the
/// cognitive-complexity budget as command branches accrete.
async fn dispatch_slash_command_if_any(inputs: SlashDispatchInputs<'_>) -> bool {
    let SlashDispatchInputs {
        resources,
        db,
        channel,
        channel_type,
        tenant_id,
        adapter,
        message,
        auth_result,
        session,
        thread_id,
    } = inputs;
    let Some(text) = content_body_text(&message.content) else {
        return false;
    };
    let Some(mut reply) = try_handle_slash_command(
        resources,
        SlashCommandContext {
            channel,
            channel_type,
            auth_result,
            session,
            text: &text,
            sender_id: &message.sender_id,
            conversation_id: message.conversation_id.as_deref(),
            thread_id,
            is_direct_message: message.is_direct_message,
            webhook_tenant_id: tenant_id,
        },
    )
    .await
    else {
        return false;
    };

    if slash_reply_should_be_private(message.is_direct_message, reply.command_name.as_deref()) {
        // Shared room (any channel): deliver the answer privately to the caller
        // and remove the command echo so other members see neither. Both are
        // best-effort and channel-specific inside canot (DM / Slack ephemeral /
        // Discord DM; echo delete only where the platform allows).
        send_private_channel_response(
            db,
            tenant_id,
            channel,
            adapter,
            reply.message,
            &message.sender_id,
        )
        .await;
        if let Some(room_id) = message.conversation_id.as_deref() {
            if let Some(notice) = room_echo::settle_room_echo(room_echo::RoomEchoSettlement {
                resources,
                db,
                tenant_id,
                channel,
                channel_type,
                adapter,
                room_id,
                channel_message_id: &message.channel_message_id,
                user_id: &session.user_id,
                sender_id: &message.sender_id,
            })
            .await
            {
                send_channel_response(db, tenant_id, channel, adapter, notice).await;
            }
        }
    } else {
        // Either a 1:1 DM (the conversation IS the private chat) or a
        // room-visible command whose reply the whole room should see — a
        // group-wide setting change, a plan the athlete chose to share. The
        // command echo stays in place in that case, so the room reads as
        // "<member> ran it → here is the effect", and the reply threads onto
        // the echo — [`room_reply_thread_anchor`] decides which replies do.
        if let Some(anchor) = room_reply_thread_anchor(
            message.is_direct_message,
            reply.command_name.as_deref(),
            &message.channel_message_id,
        ) {
            reply.message.reply_to = Some(anchor);
        }
        send_channel_response(db, tenant_id, channel, adapter, reply.message).await;
    }
    true
}

/// The two explicit bot interactions that precede any session: a linking code
/// and an in-flight OTP exchange.
///
/// Both are conversations the sender deliberately started with the bot, which
/// is why they sit above the group respond-mode gate and the logout keyword —
/// neither should be swallowed by ambient chatter rules.
///
/// `Some` means the turn was answered here and must not continue to the model.
async fn handle_pre_session_commands(
    resources: &Arc<ServerContext>,
    channel: &str,
    tenant_id: TenantId,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
    pre_link_identity: &str,
) -> Option<PersistOutcome> {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let thread_id = extract_thread_id(&message.metadata);

    let mut reply = if let LinkingAction::LinkCode(code) =
        detect_linking_code(channel_type, &message.content)
    {
        info!(channel = %channel, sender_id = %message.sender_id, "Processing channel linking command");
        emit_messaging_intent(pre_link_identity, tenant_id, channel, "link_code");
        handle_linking_command(resources, tenant_id, channel, &message.sender_id, &code).await
    } else if let Some(otp_response) = handle_otp_flow(
        resources,
        tenant_id,
        channel_type,
        channel,
        &message.sender_id,
        &message.content,
    )
    .await
    {
        emit_messaging_intent(pre_link_identity, tenant_id, channel, "otp_flow");
        otp_response
    } else {
        return None;
    };

    reply.thread_id = thread_id;
    apply_conversation_recipient(&mut reply, message.conversation_id.as_deref());
    send_channel_response(db, tenant_id, channel, adapter, reply).await;
    Some(PersistOutcome::HandledNotStored)
}

/// Persist a single inbound message and optionally prepare an LLM dispatch
///
/// Handles three cases:
/// 1. Linking command -> consume code, create link, send confirmation (not stored)
/// 2. Linked user -> resolve session, store message, dispatch to LLM pipeline
/// 3. Unlinked user -> send prompt to authenticate (not stored)
///
/// Returns `Ok(StoredWithDispatch)` for linked-user text messages,
/// `Ok(StoredNoDispatch)` for stored non-text messages,
/// `Ok(HandledNotStored)` for linking commands or unlinked users,
/// or `Err(())` if persistence failed.
async fn persist_single_message(
    resources: &Arc<ServerContext>,
    channel: &str,
    tenant_id: TenantId,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
) -> Result<PersistOutcome, ()> {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let thread_id = extract_thread_id(&message.metadata);

    // Pre-link intent events have no resolved Pierre user yet, so the
    // `{channel}:{sender_id}` channel identity is the raw distinct_id the
    // provider will hash (the same value the post-link `alias` glues to the
    // user id once linking completes).
    let pre_link_identity = format!("{channel}:{}", message.sender_id);

    if let Some(outcome) = handle_pre_session_commands(
        resources,
        channel,
        tenant_id,
        channel_type,
        adapter,
        message,
        &pre_link_identity,
    )
    .await
    {
        return Ok(outcome);
    }

    // Group respond-mode gate. Placed BEFORE the logout keyword check so
    // ambient chatter ("logout", "reset"…) can never trigger account
    // actions, and before `resolve_or_prompt` so an unlinked member's
    // chatter draws no link prompt into a busy human room. Linking codes
    // and active OTP flows stay above the gate — both are explicit
    // interactions the sender started with the bot.
    if is_ambient_group_message(resources, tenant_id, channel, message).await {
        return handle_ambient_group_message(resources, channel, tenant_id, message).await;
    }

    // Check for logout command: unlink channel and destroy session
    if is_logout_command(&message.content) {
        emit_messaging_intent(&pre_link_identity, tenant_id, channel, "logout");
        let mut logout_response = handle_logout(
            resources,
            tenant_id,
            channel_type,
            channel,
            &message.sender_id,
        )
        .await;
        logout_response.thread_id = thread_id;
        apply_conversation_recipient(&mut logout_response, message.conversation_id.as_deref());
        send_channel_response(db, tenant_id, channel, adapter, logout_response).await;
        return Ok(PersistOutcome::HandledNotStored);
    }

    // Resolve session via channel link (returns None for unlinked / denied users)
    let resolved = resolve_or_prompt(
        resources,
        db,
        tenant_id,
        channel,
        channel_type,
        adapter,
        message,
    )
    .await?;

    let Some((auth_result, session)) = resolved else {
        return Ok(PersistOutcome::HandledNotStored);
    };

    // The user's own tenant for tool execution comes straight from AuthResult —
    // authenticate_channel already resolved it (first tenant membership,
    // falling back to the webhook tenant), the same way the JWT path's
    // active_tenant_id flows. No second lookup.
    let user_tenant_id = auth_result
        .active_tenant_id
        .map_or(tenant_id, TenantId::from_uuid);
    // The tenant that owns the session + conversation + messages for this turn.
    // resolve_linked_session stored the session here (the user's own tenant for a
    // DM, the channel tenant for a group), so every conversation read, message
    // write, and reset forge below must use it too — otherwise the pipeline's
    // ownership check misses the row and the turn fails. Computed before the
    // inbound store and reset so those land under the right tenant.
    let session_tenant_id = if message.is_direct_message {
        user_tenant_id
    } else {
        tenant_id
    };

    // Reset command: rotate onto a fresh conversation so a user can abandon a
    // long or degraded thread. Handled here rather than via the generic slash
    // dispatcher because it mutates the messaging session binding the dispatcher
    // cannot reach; must run before the slash dispatch below (which would
    // otherwise report "/reset" as an unknown command).
    if is_reset_command(&message.content) {
        emit_messaging_intent(&session.user_id, tenant_id, channel, "reset");
        let mut reset_response = handle_reset(
            resources,
            db,
            session_tenant_id,
            channel_type,
            channel,
            &message.sender_id,
            &session,
        )
        .await;
        reset_response.thread_id = thread_id;
        apply_conversation_recipient(&mut reset_response, message.conversation_id.as_deref());
        send_channel_response(db, tenant_id, channel, adapter, reset_response).await;
        return Ok(PersistOutcome::HandledNotStored);
    }

    // Locale for this reply. Resolved here rather than reusing the one computed
    // further down, because that happens after the LLM-dispatch branch this
    // block short-circuits.
    let choice_locale = match Uuid::parse_str(&session.user_id) {
        Ok(uuid) => {
            resolve_messaging_locale(resources, user_tenant_id, uuid, channel, &message.sender_id)
                .await
        }
        Err(_) => DEFAULT_LOCALE.to_owned(),
    };

    // An answer to a question the platform asked: profile type, or one of the
    // seven PAR-Q+ questions. Sits ahead of the coach-proposal reply because an
    // intake is outstanding before a proposal ever goes out, so a bare "1" here
    // is answering the intake, not choosing a coach.
    //
    // Only a message that PARSES as an answer is handled here — see [`intake::IntakeOutcome`].
    let intake_outcome = intake::try_handle_intake(intake::IntakeParams {
        resources,
        // The conversation, and the facts the intake writes, both live under the
        // session tenant — for a DM that is the athlete's own tenant. Reading it
        // with the channel tenant misses the row and the intake silently never
        // sees an answer.
        tenant_id: session_tenant_id,
        conversation_id: &session.conversation,
        channel_type,
        sender_id: &message.sender_id,
        user_id: auth_result.user_id,
        locale: &choice_locale,
        text: content_body_text(&message.content)
            .unwrap_or_default()
            .as_str(),
        is_direct_message: message.is_direct_message,
    })
    .await;
    let intake_awaiting = intake_outcome.awaiting();

    if let Some(mut intake_reply) = intake_outcome.into_reply() {
        intake_reply.thread_id = thread_id;
        apply_conversation_recipient(&mut intake_reply, message.conversation_id.as_deref());
        send_channel_response(db, tenant_id, channel, adapter, intake_reply).await;
        return Ok(PersistOutcome::HandledNotStored);
    }

    // A bare number answering the coach proposal. Sits here — after auth, before
    // the model — because it is a selection, not conversation: the proposal told
    // the user to reply with a number, so that reply must bind a coach rather
    // than becoming the first thing they ever say to their coach.
    //
    // Returns None for anything that is not a bare in-range number against an
    // outstanding proposal, so ordinary messages fall through untouched.
    if let Some(mut choice_reply) =
        coach_choice::try_handle_coach_choice(coach_choice::CoachChoiceParams {
            resources,
            tenant_id,
            channel,
            channel_type,
            sender_id: &message.sender_id,
            user_id: auth_result.user_id,
            locale: &choice_locale,
            text: content_body_text(&message.content)
                .unwrap_or_default()
                .as_str(),
            intake_awaiting,
        })
        .await
    {
        choice_reply.thread_id = thread_id;
        apply_conversation_recipient(&mut choice_reply, message.conversation_id.as_deref());
        send_channel_response(db, tenant_id, channel, adapter, choice_reply).await;
        return Ok(PersistOutcome::HandledNotStored);
    }

    // Check for slash commands before storing or dispatching to LLM.
    // Commands are handled immediately and not stored in conversation history.
    if dispatch_slash_command_if_any(SlashDispatchInputs {
        resources,
        db,
        channel,
        channel_type,
        tenant_id,
        adapter,
        message,
        auth_result: &auth_result,
        session: &session,
        thread_id: thread_id.clone(),
    })
    .await
    {
        return Ok(PersistOutcome::HandledNotStored);
    }

    let stored = store_inbound_message(db, session_tenant_id, &session, channel, message).await?;
    if !stored {
        return Err(());
    }

    emit_message_received(
        &session.user_id,
        tenant_id,
        channel,
        content_type_label(&message.content),
    );
    emit_messaging_intent(&session.user_id, tenant_id, channel, "normal_chat");

    // Resolve the user's preferred locale once per dispatch so every
    // downstream stage (guardrails, verification, empty-reply) speaks the
    // same language. Uses the channel-link override first, then the user
    // profile, then the registry default.
    let locale = match Uuid::parse_str(&session.user_id) {
        Ok(uuid) => {
            resolve_messaging_locale(resources, user_tenant_id, uuid, channel, &message.sender_id)
                .await
        }
        Err(_) => DEFAULT_LOCALE.to_owned(),
    };

    // Extract text content for LLM dispatch, then run the Phase C input
    // sanitization scanner. Verbatim user text is preserved in the stored
    // message above for audit/compliance; only the LLM-bound copy gets the
    // redaction so injection patterns never reach prompt assembly.
    content_body_text(&message.content).map_or_else(
        || {
            info!("Skipping non-text message for LLM dispatch");
            Ok(PersistOutcome::StoredNoDispatch)
        },
        |text_content| {
            let sanitized = sanitize_for_dispatch(channel, &session.user_id, text_content);
            Ok(PersistOutcome::StoredWithDispatch(Box::new(
                PendingDispatch {
                    resources: Arc::clone(resources),
                    adapter: Arc::clone(adapter),
                    auth_result,
                    session,
                    channel_tenant_id: tenant_id,
                    user_tenant_id,
                    session_tenant_id,
                    channel_type,
                    channel: channel.to_owned(),
                    sender_id: message.sender_id.clone(),
                    conversation_id: message.conversation_id.clone(),
                    text_content: sanitized,
                    channel_message_id: message.channel_message_id.clone(),
                    thread_id,
                    is_group_chat: !message.is_direct_message,
                    locale,
                    // Canot generates the turn id at the webhook boundary
                    // (see canot's IncomingMessage::turn_id); adopt it so
                    // canot-emitted log spans and the platform's
                    // /internal/conversation-turn row share one key.
                    turn_id: message.turn_id.into(),
                },
            )))
        },
    )
}

/// Resolve the respond mode of the coaching group bound to a channel chat.
///
/// No chat id, no group binding, or a lookup failure all resolve to
/// [`GroupRespondMode::All`] — the pre-feature behavior — so a transient DB
/// error can only ever make the coach chattier, never mute it.
async fn channel_group_respond_mode(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    conversation_id: Option<&str>,
) -> GroupRespondMode {
    let Some(chat_id) = conversation_id.filter(|c| !c.is_empty()) else {
        return GroupRespondMode::All;
    };
    match resources
        .common
        .repos
        .groups
        .get_group_by_channel(tenant_id, channel, chat_id)
        .await
    {
        Ok(Some(group)) => group.respond_mode,
        Ok(None) => GroupRespondMode::All,
        Err(e) => {
            warn!(
                error = %e,
                channel = %channel,
                "respond-mode lookup failed; answering (fail-open to pre-feature behavior)"
            );
            GroupRespondMode::All
        }
    }
}

/// `true` when this inbound is ambient room conversation the coach must not
/// answer: a group-chat message in a mentions-mode group that neither
/// addresses the bot nor invokes a slash command.
async fn is_ambient_group_message(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    message: &IncomingMessage,
) -> bool {
    if message.is_direct_message || message.addressed_to_bot {
        return false;
    }
    // Slash commands stay live unaddressed — `/group respond all` must work
    // without a mention, or the owner could never leave mentions mode from
    // inside the chat. Their replies are already delivered privately in
    // shared rooms, so honoring them adds no room noise.
    if content_body_text(&message.content).is_some_and(|t| t.trim_start().starts_with('/')) {
        return false;
    }
    channel_group_respond_mode(
        resources,
        tenant_id,
        channel,
        message.conversation_id.as_deref(),
    )
    .await
        == GroupRespondMode::Mentions
}

/// Silently capture an ambient group message for the room transcript.
///
/// Mirrors the normal path's session resolution (auto-enrolling the sender
/// in the bound group) and inbound store, but sends NOTHING outbound and
/// never dispatches to the LLM. Senders who cannot resolve — unlinked,
/// pending, suspended, rate-limited, no session — are dropped silently: in
/// mentions mode the bot must not inject prompts into a busy human room.
/// The stored row later surfaces in the ambient transcript injected into
/// addressed group turns.
async fn handle_ambient_group_message(
    resources: &Arc<ServerContext>,
    channel: &str,
    tenant_id: TenantId,
    message: &IncomingMessage,
) -> Result<PersistOutcome, ()> {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    let Ok(auth_result) = resources
        .auth
        .auth_middleware
        .authenticate_channel(tenant_id, channel, &message.sender_id)
        .await
    else {
        debug!(
            sender_id = %message.sender_id,
            channel = %channel,
            "ambient group message from unresolvable sender; dropped silently"
        );
        return Ok(PersistOutcome::HandledNotStored);
    };

    let user_tenant_id = auth_result
        .active_tenant_id
        .map_or(tenant_id, TenantId::from_uuid);
    let chat_ref = ChannelChatRef {
        chat_id: message.conversation_id.as_deref(),
        chat_title: message.chat_title.as_deref(),
    };
    let session = match resolve_linked_session(
        resources,
        tenant_id,
        user_tenant_id,
        channel,
        &message.sender_id,
        chat_ref,
        message.is_direct_message,
    )
    .await
    {
        Ok(Some(session)) => session,
        Ok(None) => return Ok(PersistOutcome::HandledNotStored),
        Err(e) => {
            // Warn, not error: no user-visible reply was owed on this turn,
            // so this is a transcript gap rather than a dropped answer.
            warn!(
                error = %e,
                sender_id = %message.sender_id,
                channel = %channel,
                "ambient group message: session resolution failed; transcript row lost"
            );
            return Ok(PersistOutcome::HandledNotStored);
        }
    };

    // Group sessions live under the channel/bot tenant (`tenant_id` here) —
    // same rule as the dispatching path's `session_tenant_id`.
    if store_inbound_message(db, tenant_id, &session, channel, message)
        .await
        .is_err()
    {
        return Ok(PersistOutcome::HandledNotStored);
    }

    transcript::append_ambient_transcript_entry(resources, tenant_id, &session, message).await;

    emit_message_received(
        &session.user_id,
        tenant_id,
        channel,
        content_type_label(&message.content),
    );
    emit_messaging_intent(&session.user_id, tenant_id, channel, "ambient_group");
    Ok(PersistOutcome::StoredNoDispatch)
}

/// Phase C input sanitization wrapper.
///
/// Runs [`pierre_core::safety::scan`] on the inbound text and returns the
/// version that should reach the LLM. When sanitization fires the function
/// emits a structured warn-level log entry tagged with the matched
/// signature names so SOC tooling can react. The verbatim text remains in
/// `chat_messages` for audit purposes.
fn sanitize_for_dispatch(channel: &str, user_id: &str, text_content: String) -> String {
    match scan_for_injection(&text_content) {
        SanitizationOutcome::Clean => text_content,
        SanitizationOutcome::Sanitized { redacted, matches } => {
            let signatures: Vec<&'static str> =
                matches.iter().map(|m| m.signature.as_str()).collect();
            let signatures_str = signatures.join(",");
            warn!(
                channel = %channel,
                user_id = %hash_id(user_id),
                signatures = %signatures_str,
                match_count = matches.len(),
                "input sanitization fired — redacting injection patterns from LLM-bound text"
            );
            redacted
        }
    }
}

/// Send an authentication prompt to an unlinked user
///
/// Chooses between in-chat OTP flow (when email service is available) and
/// link-URL flow (fallback), then sends the response via the channel adapter.
async fn send_unlinked_user_prompt(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
) {
    let prompt_type = if resources.common.email_service.is_some() {
        "otp"
    } else {
        "link_url"
    };
    emit_unlinked_prompted(tenant_id, channel, prompt_type);

    let mut prompt = if resources.common.email_service.is_some() {
        info!(channel = %channel, sender_id = %message.sender_id, "Unlinked user, starting OTP flow");
        start_otp_flow(
            resources,
            db,
            tenant_id,
            channel_type,
            &message.sender_id,
            message.sender_name.as_deref(),
        )
        .await
    } else {
        info!(channel = %channel, sender_id = %message.sender_id, "Unlinked user, sending link URL (no email service)");
        create_link_and_prompt(
            resources,
            db,
            tenant_id,
            channel_type,
            &message.sender_id,
            message.sender_name.as_deref(),
        )
        .await
    };
    prompt.thread_id = extract_thread_id(&message.metadata);
    apply_conversation_recipient(&mut prompt, message.conversation_id.as_deref());
    send_channel_response(db, tenant_id, channel, adapter, prompt).await;
}

/// Resolve a linked session and authenticated principal, or send a prompt /
/// denial reply when the channel sender cannot proceed.
///
/// Calls [`pierre_middleware::auth::McpAuthMiddleware::authenticate_channel`]
/// for the unified auth/status/rate-limit gate, then resolves the session
/// row on success. Returns `Ok(Some((auth_result, session)))` for authorized
/// linked users, `Ok(None)` after surfacing the right reply (no link → prompt;
/// pending / suspended / rate-limited → translated denial), or `Err(())` on
/// operator-category failure.
async fn resolve_or_prompt(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    adapter: &Arc<dyn MessagingChannel>,
    message: &IncomingMessage,
) -> Result<Option<(AuthResult, ResolvedSession)>, ()> {
    // No provider gate here any more. A providerless athlete used to be refused
    // the turn outright because the model could not tell "no recent activity"
    // from "no connected provider" and invented the difference. That is now
    // handled where it belongs: the system prompt states the absence, the
    // athlete-data verifier contradicts any specific figure asserted without a
    // source, and the dispatch chokepoint refuses every provider-requiring tool.
    // The tappable connect card that used to ride the refusal now rides the
    // served reply instead — see `maybe_send_connect_card` in `dispatch`.
    let auth_outcome = resources
        .auth
        .auth_middleware
        .authenticate_channel(tenant_id, channel, &message.sender_id)
        .await;
    let Some(auth_result) = handle_channel_auth_outcome(ChannelAuthOutcomeInputs {
        resources,
        db,
        tenant_id,
        channel,
        channel_type,
        adapter,
        message,
        outcome: auth_outcome,
    })
    .await?
    else {
        return Ok(None);
    };

    // The user's OWN tenant (their personal workspace). For DIRECT messages it
    // becomes the session tenant — so a DM's session aligns with the user's
    // activity cache and a backfill-completion push can find it — instead of the
    // bot/channel tenant the webhook carries. authenticate_channel already
    // resolved it (active_tenant_id), the same value tool execution uses. Group
    // sessions, the channel-link lookup, and the outbound send stay on the bot
    // tenant (the bot credential belongs to the bot owner, who may differ from
    // the user; a group's coaching_group must resolve to one row for everyone).
    let user_tenant_id = auth_result
        .active_tenant_id
        .map_or(tenant_id, TenantId::from_uuid);

    let chat_ref = ChannelChatRef {
        chat_id: message.conversation_id.as_deref(),
        chat_title: message.chat_title.as_deref(),
    };
    match resolve_linked_session(
        resources,
        tenant_id,
        user_tenant_id,
        channel,
        &message.sender_id,
        chat_ref,
        message.is_direct_message,
    )
    .await
    {
        Ok(Some(session)) => Ok(Some((auth_result, session))),
        Ok(None) => {
            // authenticate_channel succeeded but channel link disappeared
            // between calls — race or operator action. Treat as unlinked.
            send_unlinked_user_prompt(
                resources,
                db,
                tenant_id,
                channel,
                channel_type,
                adapter,
                message,
            )
            .await;
            Ok(None)
        }
        Err(e) => {
            // Error-level: the user sent a message and got no reply — this is a
            // production incident, not a warning. Triggers the dravr-tronc
            // Slack notifier so operators see the outage within seconds.
            error!(
                error = %e,
                sender_id = %message.sender_id,
                channel = %channel_type,
                "Failed to resolve messaging session, dropping message"
            );
            Err(())
        }
    }
}

/// Store a single inbound message in the database
///
/// Returns `Ok(true)` if stored, `Err(())` on duplicate or DB error (already logged).
async fn store_inbound_message(
    db: &dyn MessagingRepository,
    // The session's tenant (user's own for DMs, channel tenant for groups) —
    // the inbound row must share the tenant of its session + conversation.
    session_tenant_id: TenantId,
    session: &ResolvedSession,
    channel: &str,
    message: &IncomingMessage,
) -> Result<bool, ()> {
    let msg_id = Uuid::new_v4().to_string();
    let content_type = content_type_label(&message.content);
    let content_body = content_body_text(&message.content);
    let raw_payload = serde_json::to_string(&message.raw_payload).ok();
    // The `messaging_messages` table column is still called
    // `correlation_id` — it existed before the turn-id threading — so
    // the string from `turn_id` is what gets persisted there.
    let correlation_str = message.turn_id.to_string();

    let params = InsertMessageParams {
        id: &msg_id,
        tenant_id: session_tenant_id,
        session_id: &session.session_id,
        direction: "inbound",
        channel_type: channel,
        channel_message_id: &message.channel_message_id,
        sender_id: &message.sender_id,
        content_type,
        content_body: content_body.as_deref(),
        correlation_id: &correlation_str,
        raw_payload: raw_payload.as_deref(),
        // Inbound rows are what the athlete said; there is no assistant reply
        // to rate on this side of the turn.
        chat_message_id: None,
    };

    match db.insert_message(&params).await {
        Ok(true) => Ok(true),
        Ok(false) => {
            info!(
                channel_message_id = %message.channel_message_id,
                "Duplicate message skipped (idempotent)"
            );
            Err(())
        }
        Err(e) => {
            warn!(
                error = %e,
                channel_message_id = %message.channel_message_id,
                "Failed to persist inbound message"
            );
            Err(())
        }
    }
}

/// Extract a content type label from the message content variant
fn content_type_label(content: &MessageContent) -> &'static str {
    match content {
        MessageContent::Text { .. } => "text",
        MessageContent::RichText { .. } => "rich_text",
        MessageContent::Media { .. } => "media",
        MessageContent::Location { .. } => "location",
        MessageContent::Card { .. } => "card",
    }
}

/// Extract the text body from the message content (if applicable)
pub(super) fn content_body_text(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::Text { body }
        | MessageContent::RichText { body }
        | MessageContent::Card { body, .. } => Some(body.clone()),
        MessageContent::Media { caption, .. } => caption.clone(),
        MessageContent::Location { .. } => None,
    }
}
