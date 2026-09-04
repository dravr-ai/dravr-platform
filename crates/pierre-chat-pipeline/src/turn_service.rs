// ABOUTME: The one turn ladder — quota, slash, locale, BYO model, pipeline, usage, envelope
// ABOUTME: Every chat surface enters a turn here, so a capability cannot land on one and miss another

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One turn, one ladder.
//!
//! A chat turn used to be assembled twice: once in the web route and once in
//! the messaging dispatcher. Each copy ran the same six steps — check the
//! quotas, answer a slash command, resolve the locale, resolve the tenant's
//! own model key, run the pipeline, record the usage — and each copy was free
//! to drift from the other. It did. Messaging bypassed every usage cap for
//! four months because enforcement was a step in the *other* ladder
//! (registre#9), and the per-conversation cap it did eventually check was
//! measured against a counter that path never incremented.
//!
//! [`execute`] is that ladder, once. A surface arrives with a
//! [`TurnRequest`] and a [`SurfaceProfile`] and leaves with a
//! [`ServedTurn`]; everything between is identical for every surface, because
//! it is literally the same code. What stays outside is transport: the
//! per-conversation ordering lock, the panic boundary, the empty-reply guard,
//! and the shape of the reply on the wire. Those are properties of a channel,
//! not of a turn.
//!
//! # Why not a bare envelope
//!
//! [`execute`] returns an enum rather than a [`TurnEnvelope`] because a slash
//! command is not a coaching turn: no model ran, no tokens were spent, and
//! its rows — when the surface's [`CommandPersistence`] writes them — are
//! stamped so no later prompt replays them. Folding that into the envelope a
//! pipeline turn produces would make every consumer tell the two apart by
//! sniffing fields; the enum says it outright.

use std::sync::Arc;

use pierre_commands::dispatch::{try_dispatch, DispatchOutcome, DispatchRequest};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::TranscriptSpeaker;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_llm::ChatProvider;
use pierre_services::tenant_chat_provider::resolve_tenant_chat_provider;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::envelope::{ActionKind, QuotaState, TurnAction, TurnEnvelope};
use crate::hooks::PipelineHooks;
use crate::quota_policy::{check_pre_chat_quotas_scoped, settle_quota_notice, PreChatScope};
use crate::stages::coach_mention::resolve_coach_mention;
use crate::stages::command_persistence::{
    is_room_visible, persist_command_turn, CommandPersistence, PersistedCommandReply,
};
use crate::stages::persistence::fan_out_to_group_transcript;
use crate::surface_profile::SurfaceProfile;
use crate::turn::{TurnInput, TurnOrigin};
use crate::usage_counters::{
    increment_usage_counters_scoped, tokens_from_envelope, UsageIncrementScope,
};
use crate::ChatPipelineContext;

/// Everything a surface knows about one inbound turn before the ladder runs.
pub struct TurnRequest<'a> {
    /// Conversation the turn appends to (`chat_conversations.id`).
    pub conversation_id: String,
    /// Authenticated athlete.
    pub user_id: Uuid,
    /// Tenant the conversation and its messages are read and written under.
    ///
    /// Differs from [`Self::tool_tenant_id`] on a shared messaging bot's group
    /// room, where one conversation is read by every member.
    pub conversation_tenant_id: TenantId,
    /// The athlete's own tenant: the one their tool credentials, provider
    /// connections **and usage counters** live under.
    ///
    /// Quotas are checked and recorded against this tenant on every surface.
    /// Counting a Telegram turn under the bot's tenant is what made messaging
    /// usage invisible to every quota read (registre#9).
    pub tool_tenant_id: TenantId,
    /// The athlete's message, already sanitized by the ingress that received
    /// it — or, when [`Self::origin`] says so, a prompt the platform composed.
    pub content: String,
    /// Who authored [`Self::content`].
    ///
    /// Every inbound surface sends [`TurnOrigin::Athlete`]. A background job
    /// re-entering the pipeline to answer on its own initiative sends
    /// [`TurnOrigin::Platform`], which keeps the prompt out of the athlete's
    /// transcript and out of their guided flow.
    pub origin: TurnOrigin,
    /// Correlation id minted at the inbound boundary.
    pub turn_id: ConversationTurnId,
    /// Pre-rendered room transcript for a group turn; `None` for a DM or an
    /// in-app conversation.
    pub ambient_context: Option<String>,
    /// Canonical channel identifier (`"web"`, `"mobile"`, `"telegram"`, …)
    /// used for slash-command analytics and the command context.
    pub channel_type: &'a str,
    /// `true` when the athlete is alone with the coach: a messaging DM, or an
    /// in-app conversation with no coaching group bound to it.
    pub is_direct_message: bool,
    /// Whether a `/group` command typed in a conversation bound to no group
    /// may act on the first group the athlete belongs to. `true` only on the
    /// messaging surfaces, where a DM with the bot is the athlete's one thread
    /// and the ambient group is the group they mean; `false` in the app, where
    /// a solo thread is exactly that and the group commands are refused rather
    /// than aimed at whichever group was touched last.
    pub ambient_group_fallback: bool,
    /// Which slash-command turns this surface writes to the transcript.
    pub command_persistence: CommandPersistence,
    /// Channel-native sender id, for commands that unlink a channel; `None`
    /// on the in-app surface, which has no channel link.
    pub sender_id: Option<&'a str>,
    /// Progress, streaming and chart-publishing wiring for this turn.
    pub hooks: PipelineHooks<'a>,
}

/// A slash command's answer, in a shape no surface owns.
///
/// Deliberately the handler's own output rather than a rendered reply: web
/// lays it out as response blocks, a messaging channel as a card or text.
/// Neither re-runs the dispatch.
#[derive(Debug, Clone)]
pub struct CommandTurn {
    /// The command definition's `name:` id (`"coach"`, `"group-status"`),
    /// or `None` when the text was `/`-prefixed but matched nothing.
    pub command_name: Option<String>,
    /// The reply body, already localized by the handler, in inline markdown:
    /// the in-app surfaces parse it as is, and a messaging channel converts it
    /// into canot's rich-text dialect at its egress, so the persisted row and
    /// every delivery read from the same text.
    pub text: String,
    /// [`Self::text`] carries inline markdown a channel renderer must
    /// translate into native formatting. `false` when the body is shown as
    /// typed — the unknown-command reply, and handlers that answered plain.
    pub is_rich_text: bool,
    /// Title labelling [`Self::actions`], when the handler returned a card.
    pub card_title: Option<String>,
    /// Controls the athlete can press.
    pub actions: Vec<TurnAction>,
    /// The conversation the athlete is now on, when the command moved them
    /// — `/reset` and nothing else today.
    ///
    /// A messaging channel has already had its session repointed by the
    /// handler, so this is the in-app clients' copy of the same fact: the
    /// thread they posted to is not the thread the next turn belongs to.
    pub rotated_to: Option<String>,
    /// The transcript rows this turn wrote, when the surface's
    /// [`CommandPersistence`] covered the command and the write succeeded.
    /// `None` is a turn the transcript does not hold: a private reply in a
    /// shared room, or a write that failed after the answer was produced —
    /// the answer is still delivered, and the failure is logged.
    pub persisted: Option<PersistedCommandReply>,
}

/// How one call to [`execute`] ended.
pub enum ServedTurn {
    /// The pipeline ran and produced a reply.
    Pipeline(Box<TurnEnvelope>),
    /// A slash command answered. No model call and no token spend — a
    /// command's output is account state, not coaching — but the turn is
    /// history like any other: it is written to the transcript under the
    /// surface's [`CommandPersistence`] and kept out of every later prompt.
    Command {
        /// The handler's answer.
        command: Box<CommandTurn>,
        /// Where the athlete stood against their caps when the command ran.
        /// A command spends no budget, but it is as good a moment as any to
        /// tell them the budget is nearly spent.
        quota: QuotaState,
    },
}

/// Identifiers a slash command is dispatched against.
///
/// Separate from [`TurnRequest`] because the messaging ingress answers a
/// command before a coaching turn exists at all: the athlete's `/logout`
/// never becomes a conversation turn, so it arrives here without one.
pub struct SlashRequest<'a> {
    /// Authenticated athlete.
    pub user_id: Uuid,
    /// The athlete's own tenant — the scope their account data resolves in.
    pub tenant_id: TenantId,
    /// Conversation the command was typed in.
    pub conversation_id: &'a str,
    /// Tenant that owns [`Self::conversation_id`]'s row.
    pub conversation_tenant_id: TenantId,
    /// Canonical channel identifier for analytics and the command context.
    pub channel_type: &'a str,
    /// Locale every string the handler returns is written in.
    pub locale: &'a str,
    /// `true` when the athlete is alone with the coach.
    pub is_direct_message: bool,
    /// See [`TurnRequest::ambient_group_fallback`].
    pub ambient_group_fallback: bool,
    /// Which command turns this surface writes to the transcript.
    pub persistence: CommandPersistence,
    /// Channel-native sender id for commands that unlink a channel.
    pub sender_id: Option<&'a str>,
    /// The raw text the athlete typed.
    pub text: &'a str,
}

/// Run one chat turn.
///
/// The ladder, in order: measure the athlete's standing against their usage
/// caps and refuse a hard breach; hand back a caller-served request; answer a
/// slash command; resolve the turn's locale from the athlete's own words;
/// resolve the tenant's own model key; run the pipeline; record the usage the
/// next turn's check will read.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError::quota_exceeded`] when a usage cap
/// refuses the turn, whatever the slash handler returned when a command
/// fails, and whatever the pipeline returned otherwise.
pub async fn execute(
    ctx: &ChatPipelineContext,
    request: TurnRequest<'_>,
    profile: &SurfaceProfile,
) -> AppResult<ServedTurn> {
    let user_id_str = request.user_id.to_string();

    // The conversation row supplies the coach the per-coach cap is keyed on.
    // A missing row is fine — the athlete may be opening a new conversation —
    // so a lookup failure narrows the scope rather than refusing the turn.
    let coach_id = ctx
        .repos
        .chat
        .get_conversation(
            &request.conversation_id,
            &user_id_str,
            request.conversation_tenant_id,
        )
        .await
        .ok()
        .flatten()
        .and_then(|conv| conv.coach_id);

    let quota = check_pre_chat_quotas_scoped(
        ctx,
        request.tool_tenant_id,
        request.user_id,
        &PreChatScope {
            conversation_id: Some(request.conversation_id.as_str()),
            coach_id: coach_id.as_deref(),
        },
    )
    .await?;

    if let Some(command) = dispatch_slash(
        ctx,
        &SlashRequest {
            user_id: request.user_id,
            tenant_id: request.tool_tenant_id,
            conversation_id: &request.conversation_id,
            conversation_tenant_id: request.conversation_tenant_id,
            channel_type: request.channel_type,
            locale: &profile.locale,
            is_direct_message: request.is_direct_message,
            ambient_group_fallback: request.ambient_group_fallback,
            persistence: request.command_persistence,
            sender_id: request.sender_id,
            text: &request.content,
        },
    )
    .await?
    {
        // A command answered, so the notice has somewhere to ride. It spends
        // no budget, but it is as good a moment as any to tell the athlete
        // theirs is nearly gone (registre#260).
        let quota = settle_quota_notice(
            ctx.repos.usage_counters.as_ref(),
            request.tool_tenant_id,
            &user_id_str,
            quota,
        )
        .await;
        return Ok(ServedTurn::Command {
            command: Box::new(command),
            quota,
        });
    }

    // The athlete's current message decides the turn's language, over their
    // stored preference: a model answers in the language it was addressed in,
    // and every platform string this turn renders has to match it.
    let profile = SurfaceProfile {
        locale: detect_turn_locale(&request.content, &profile.locale),
        ..profile.clone()
    };

    // `@handle` hands this one turn to an installed coach. Resolved here, on
    // the ladder every surface climbs, so a Telegram mention and a web mention
    // route identically and no client has to know the grammar. A slash command
    // never reaches this point, which is what keeps `/coach add @handle` a
    // command argument rather than a mention. Installs live in the athlete's
    // own tenant, so that is where the handle resolves.
    let mentioned_coach = resolve_coach_mention(
        ctx.repos.coaches.as_ref(),
        &request.content,
        request.user_id,
        request.tool_tenant_id,
    )
    .await;

    let turn_input = TurnInput {
        conversation_id: request.conversation_id.clone(),
        user_id: user_id_str,
        conversation_tenant_id: request.conversation_tenant_id,
        tool_tenant_id: request.tool_tenant_id,
        is_direct_message: request.is_direct_message,
        content: request.content.clone(),
        origin: request.origin,
        turn_id: request.turn_id,
        ambient_context: request.ambient_context,
        quota,
        mentioned_coach: mentioned_coach.map(Box::new),
    };

    let mut ctx_for_turn = ctx.clone();
    if let Some(provider) =
        resolve_byo_chat_provider(ctx, request.tool_tenant_id, request.user_id).await
    {
        ctx_for_turn.chat_provider = Some(provider);
    }

    let envelope = crate::run(&ctx_for_turn, turn_input, &profile, &request.hooks).await?;

    let (prompt_tokens, completion_tokens) = tokens_from_envelope(&envelope, &request.content);
    increment_usage_counters_scoped(
        ctx,
        request.tool_tenant_id,
        request.user_id,
        i64::from(prompt_tokens) + i64::from(completion_tokens),
        &UsageIncrementScope {
            conversation_id: Some(request.conversation_id.as_str()),
            coach_id: coach_id.as_deref(),
        },
    )
    .await;

    Ok(ServedTurn::Pipeline(Box::new(envelope)))
}

/// Try to answer `request.text` as a slash command.
///
/// `Ok(None)` means the text was not a command and the caller should run a
/// coaching turn (or, at a messaging ingress, persist the message and
/// dispatch one). Every chat surface reaches
/// [`pierre_commands::dispatch::try_dispatch`] through here, so a command
/// behaves the same wherever it is typed and the channel-specific work is
/// confined to rendering what comes back.
///
/// A host with no command catalog configured — a test context that skips
/// `commands/` — resolves every text to `Ok(None)`, so the turn falls through
/// to the coach rather than failing.
///
/// An answered command is then written to the transcript under
/// `request.persistence` — the `/…` line and the reply, both stamped so they
/// never replay into a prompt. The write is best-effort: a reply that was
/// produced is delivered even when the rows could not land, and the failure
/// is logged rather than turned into an error the athlete reads as "the
/// command failed".
///
/// # Errors
///
/// Returns whatever the command handler returned when it fails.
pub async fn dispatch_slash(
    ctx: &ChatPipelineContext,
    request: &SlashRequest<'_>,
) -> AppResult<Option<CommandTurn>> {
    if !request.text.trim_start().starts_with('/') {
        return Ok(None);
    }
    let (Some(command_registry), Some(handler_registry)) = (
        ctx.command_registry.as_ref(),
        ctx.command_handler_registry.as_ref(),
    ) else {
        debug!("command catalog not configured; treating slash text as a coaching turn");
        return Ok(None);
    };

    let outcome = try_dispatch(DispatchRequest {
        ctx: &ctx.command_ctx,
        command_registry,
        command_handler_registry: handler_registry,
        user_id: request.user_id,
        tenant_id: request.tenant_id,
        channel_type: request.channel_type,
        locale: request.locale,
        is_direct_message: request.is_direct_message,
        ambient_group_fallback: request.ambient_group_fallback,
        conversation_id: Some(request.conversation_id),
        conversation_tenant_id: request.conversation_tenant_id,
        sender_id: request.sender_id,
        text: request.text,
        tool_runtime: &ctx.tool_runtime,
    })
    .await?;

    let Some(mut command) = command_turn(outcome, request.channel_type) else {
        return Ok(None);
    };
    persist_if_covered(ctx, request, &mut command).await;
    Ok(Some(command))
}

/// Shape a dispatch outcome as a command turn; `None` when the text was not a
/// command at all.
fn command_turn(outcome: DispatchOutcome, channel_type: &str) -> Option<CommandTurn> {
    match outcome {
        DispatchOutcome::NotACommand => None,
        DispatchOutcome::UnknownCommand { body } => Some(CommandTurn {
            command_name: None,
            text: body,
            is_rich_text: false,
            card_title: None,
            actions: Vec::new(),
            rotated_to: None,
            persisted: None,
        }),
        DispatchOutcome::Executed {
            command_name,
            response,
            rotated_to,
        } => {
            info!(
                command = %command_name,
                channel = %channel_type,
                "Slash command executed"
            );
            let card_title = if response.is_card() {
                response.card_title.clone()
            } else {
                None
            };
            let actions = response
                .actions
                .iter()
                .map(|action| TurnAction {
                    label: action.label.clone(),
                    kind: ActionKind::from_wire(&action.action_type),
                    value: action.value.clone(),
                })
                .collect();
            Some(CommandTurn {
                command_name: Some(command_name),
                text: response.text,
                is_rich_text: response.is_rich_text,
                card_title,
                actions,
                rotated_to,
                persisted: None,
            })
        }
    }
}

/// Write the turn to the transcript when the surface's policy covers it.
///
/// Best-effort: the reply is already produced and owed to the athlete, so a
/// write that fails is logged and the turn goes out unpersisted.
///
/// A room-visible command turn in a shared messaging room is also fanned out
/// to the group's shared transcript: the reply was posted to the room, so the
/// room's history — the ambient block a later turn reads — carries it, and a
/// coach can discuss the plan an athlete just shared. The in-app surfaces
/// persist every command turn into the caller's own conversation and fan
/// nothing out, exactly as before.
async fn persist_if_covered(
    ctx: &ChatPipelineContext,
    request: &SlashRequest<'_>,
    command: &mut CommandTurn,
) {
    if !request
        .persistence
        .persists(command.command_name.as_deref())
    {
        return;
    }
    let persisted = match persist_command_turn(ctx.repos.chat.as_ref(), request, command).await {
        Ok(persisted) => persisted,
        Err(e) => {
            warn!(
                error = %e,
                conversation_id = %request.conversation_id,
                command = ?command.command_name,
                "command reply could not be written to the transcript; delivering it unpersisted"
            );
            return;
        }
    };
    if request.persistence == CommandPersistence::RoomVisibleOnly
        && is_room_visible(command.command_name.as_deref())
    {
        fan_out_room_visible_turn(ctx, request, &persisted).await;
    }
    command.persisted = Some(persisted);
}

/// Append both rows of a room-visible command turn to the group's shared
/// transcript — the `/…` line as the member, the reply as the coach — when
/// the conversation is group-bound. Best-effort like the rows themselves: a
/// failed append is logged, never turned into a failed command.
async fn fan_out_room_visible_turn(
    ctx: &ChatPipelineContext,
    request: &SlashRequest<'_>,
    persisted: &PersistedCommandReply,
) {
    let user_id = request.user_id.to_string();
    let rows = [
        (TranscriptSpeaker::Member, &persisted.user_message),
        (TranscriptSpeaker::Coach, &persisted.assistant_message),
    ];
    for (speaker, row) in rows {
        if let Err(e) = fan_out_to_group_transcript(
            ctx.repos.groups.as_ref(),
            &persisted.conversation,
            request.conversation_tenant_id,
            &user_id,
            speaker,
            &row.content,
            &row.id,
        )
        .await
        {
            warn!(
                error = %e,
                conversation_id = %request.conversation_id,
                speaker = ?speaker,
                "room-visible command turn could not reach the group transcript"
            );
        }
    }
}

/// Resolve the language this turn is conducted in.
///
/// Prefers the language of the message the athlete just typed over their
/// stored preference, because a model mirrors the language it was addressed
/// in — and every platform string around the reply (a verification banner, a
/// scope refusal, a status placeholder, a chart axis) has to speak the same
/// language as the coaching text beside it.
///
/// `fallback` is whatever the surface resolved before the turn: the channel
/// link's override then `users.locale` on a messaging channel, `users.locale`
/// in the app. It is returned unchanged when the message is too short for a
/// reliable signal (whatlang guesses badly on "ok", "oui") or the detected
/// language is not one the platform speaks.
#[must_use]
pub fn detect_turn_locale(text: &str, fallback: &str) -> String {
    /// Below this many characters whatlang's verdict is noise, and the stored
    /// preference is almost always what the athlete wants.
    const MIN_LEN: usize = 12;
    if text.trim().chars().count() < MIN_LEN {
        return fallback.to_owned();
    }
    let Some(info) = whatlang::detect(text) else {
        return fallback.to_owned();
    };
    if !info.is_reliable() {
        return fallback.to_owned();
    }
    match info.lang() {
        whatlang::Lang::Fra => "fr".to_owned(),
        whatlang::Lang::Eng => "en".to_owned(),
        whatlang::Lang::Spa => "es".to_owned(),
        whatlang::Lang::Deu => "de".to_owned(),
        whatlang::Lang::Por => "pt".to_owned(),
        _ => fallback.to_owned(),
    }
}

/// The chat provider a `(tenant, athlete)` pair should run this turn on.
///
/// `Some` is the tenant's own stored key ("bring your own"); `None` leaves
/// the turn on the process-wide provider singleton. A short TTL cache keeps
/// the common no-key case off the database on every turn.
async fn resolve_byo_chat_provider(
    ctx: &ChatPipelineContext,
    tenant_id: TenantId,
    user_id: Uuid,
) -> Option<Arc<ChatProvider>> {
    resolve_tenant_chat_provider(
        &ctx.tenant_chat_providers,
        ctx.repos.llm_credentials.as_ref(),
        ctx.repos.security.as_ref(),
        tenant_id,
        user_id,
    )
    .await
}
