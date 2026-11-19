// ABOUTME: Transport-agnostic slash-command dispatch — the single authority for every chat surface
// ABOUTME: Used by messaging ingress, web chat, mobile chat. Channel-specific framing belongs at callers.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::sync::Arc;

use dravr_canot::commands::{CommandMatcher, CommandRegistry, CommandResponse};
use pierre_contremaitre::messaging_strings::KEY_UNKNOWN_COMMAND;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TurnOrigin};
use pierre_runtime_context::CommandCtx;
use pierre_services::conversation_forge::reactivate_for_turn;
use pierre_tool_runtime::context::refuse_in_support_session;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::info;
use uuid::Uuid;

use crate::guided_walk::refused_in_athlete_thread;
use crate::{
    CommandHandler, CommandHandlerRegistry, ConversationRotation, PlatformCommandContext, RotatedTo,
};

/// Minimal request payload for [`try_dispatch`].
///
/// Wraps the fields the dispatcher needs to build a
/// [`PlatformCommandContext`] if and only if the text parses as a command.
/// Callers assemble this lightweight struct from their own auth/resolution
/// layer (messaging ingress, web chat route, mobile chat route, Slack ops
/// button route) and the dispatcher handles parsing, handler lookup,
/// execution, and analytics uniformly.
pub struct DispatchRequest<'a> {
    /// Narrow runtime context — repos, group service, messaging strings.
    /// Concrete `ServerContext` lives in `pierre-server`; this crate
    /// stays a leaf by holding the trait object instead.
    pub ctx: &'a Arc<dyn CommandCtx>,
    /// Slash-command catalog — built from `commands/*.md` at startup.
    /// Each chat surface pulls it from its server context and hands it
    /// in here so this crate doesn't need to know how the host stores
    /// it.
    pub command_registry: &'a Arc<CommandRegistry>,
    /// Handler-name → handler mapping, populated alongside
    /// `command_registry`. Same rationale: passed in explicitly.
    pub command_handler_registry: &'a Arc<CommandHandlerRegistry>,
    /// Authenticated user.
    pub user_id: Uuid,
    /// Active tenant for this turn.
    pub tenant_id: TenantId,
    /// Canonical channel identifier (`"telegram"`, `"slack"`, `"web"`,
    /// `"mobile"`, ...). Used for analytics hashing and the
    /// [`PlatformCommandContext::channel_type`] field.
    pub channel_type: &'a str,
    /// BCP-47 short locale resolved up-front by the caller.
    pub locale: &'a str,
    /// `true` when the sender is in a 1:1 DM with the bot. Web and mobile
    /// chat sets it from the conversation — `true` unless a coaching group
    /// is bound to it; messaging channels set this from their native
    /// chat-kind signal.
    pub is_direct_message: bool,
    /// See [`PlatformCommandContext::ambient_group_fallback`]. Messaging
    /// surfaces pass `true`; the in-app chat and the catalogue pass `false`.
    pub ambient_group_fallback: bool,
    /// Pierre `chat_conversations.id` carrying this turn, when known.
    ///
    /// Set by every chat surface that has a resolved conversation: web/
    /// mobile chat (path param), messaging ingress (`session.conversation`).
    /// Group-scoped commands (notably `/group consent yes`) read
    /// `chat_conversations.group_id` from this id so consent lands on
    /// the chat-bound coaching group instead of the user's most-recent
    /// group from `list_groups_for_user` (which is non-deterministic
    /// when a user belongs to several groups).
    pub conversation_id: Option<&'a str>,
    /// Tenant that owns [`Self::conversation_id`]'s `chat_conversations` row.
    ///
    /// Separate from [`Self::tenant_id`] because the two differ on every
    /// shared-bot group chat: the caller's own tenant scopes their user data,
    /// while the session, conversation and messages of a non-DM room live under
    /// the channel/bot tenant so all members read one conversation. Callers set
    /// this to the same value their own conversation reads and writes use — for
    /// a 1:1 DM (and for web/mobile, which are per-user by construction) that is
    /// the caller's tenant.
    pub conversation_tenant_id: TenantId,
    /// Channel sender identifier (e.g. Telegram chat id, Slack user id) for
    /// messaging surfaces; `None` on web/mobile and synthetic dispatch where
    /// there is no channel link. Used by `/logout` to unlink the exact
    /// channel sender.
    pub sender_id: Option<&'a str>,
    /// Raw user input. The dispatcher inspects it for the `/` prefix and
    /// routes accordingly.
    pub text: &'a str,
    /// Tool-dispatch runtime handed through to
    /// [`PlatformCommandContext::tool_runtime`] for handlers that execute
    /// MCP tools. Same concrete `ServerContext` as `ctx`,
    /// behind a separate trait to avoid a crate cycle through
    /// `pierre-runtime-context`.
    pub tool_runtime: &'a Arc<dyn ToolRuntime>,
    /// Who [`Self::text`] came from. Only an athlete's command takes an
    /// archived thread's quota slot back (see
    /// [`crate::CommandHandler::resumes_thread`]); one the platform composed —
    /// a re-entry turn replaying an earlier question — runs, but never
    /// reactivates the thread.
    pub origin: TurnOrigin,
}

impl fmt::Debug for DispatchRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The text is the athlete's message or a command line carrying invite
        // codes and confirm tokens; the sender is a phone number on some
        // channels.
        f.debug_struct("DispatchRequest")
            .field("command_registry", &self.command_registry)
            .field("command_handler_registry", &self.command_handler_registry)
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .field("channel_type", &self.channel_type)
            .field("locale", &self.locale)
            .field("is_direct_message", &self.is_direct_message)
            .field("ambient_group_fallback", &self.ambient_group_fallback)
            .field("conversation_id", &self.conversation_id)
            .field("conversation_tenant_id", &self.conversation_tenant_id)
            .field("sender_id", &self.sender_id.map(|_| "[REDACTED]"))
            .field("text_len", &self.text.len())
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

/// Outcome of a single dispatch attempt.
///
/// Callers fan out on the variant:
/// - `NotACommand` → fall through to the LLM pipeline (web/mobile) or
///   normal message persistence (messaging channels).
/// - `UnknownCommand` → surface the localized body to the user and
///   short-circuit quota/LLM work.
/// - `Executed` → render/persist `response` per the surface's conventions.
pub enum DispatchOutcome {
    /// Text did not start with `/`. The caller owns the non-command path.
    NotACommand,
    /// `/`-prefixed but matcher did not find a registered command (typo,
    /// renamed command, uppercase, etc.). Body is already localized.
    UnknownCommand {
        /// Ready-to-send body text in the requested locale.
        body: String,
    },
    /// A registered command handler ran and produced a response.
    Executed {
        /// The command definition's `name:` id (e.g. `"agent-list"`,
        /// `"group-status"`) — the handler-registry key, NOT the spaced
        /// `/group status` trigger the user typed.
        command_name: String,
        /// Handler output. May be a plain text reply or a card with actions.
        response: CommandResponse,
        /// The conversation the athlete is now on, when the handler moved
        /// them — `/reset`, and `/group create` in the app, which moves the
        /// command's own turn with them (carnet#1186). The surface follows
        /// it: a messaging channel already had its session repointed, the
        /// in-app clients open the id.
        rotated_to: Option<RotatedTo>,
        /// The divider of the topic the handler opened in the athlete's
        /// personal thread instead of moving them — `/reset` there
        /// (carnet#840). The in-app clients re-read the thread to draw it.
        opened_topic: Option<String>,
    },
}

// A command's reply can carry a fresh group invite code: only sizes print.
impl fmt::Debug for DispatchOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotACommand => f.write_str("NotACommand"),
            Self::UnknownCommand { body } => f
                .debug_struct("UnknownCommand")
                .field("body_len", &body.len())
                .finish(),
            Self::Executed {
                command_name,
                response,
                rotated_to,
                opened_topic,
            } => f
                .debug_struct("Executed")
                .field("command_name", command_name)
                .field("response_text_len", &response.text.len())
                .field("response_actions", &response.actions.len())
                .field("rotated_to", rotated_to)
                .field("opened_topic", opened_topic)
                .finish(),
        }
    }
}

/// The command `text` would run, when what it leaves behind outlives the
/// session that typed it.
///
/// Returns its catalogue name when its handler says so for these arguments
/// ([`crate::CommandHandler::outlives_support_session`]); `None` for anything
/// else, including text that is not a command. Parses exactly as
/// [`try_dispatch`] does, so the command it names is the one that would run.
/// A surface that serves an admin's support session asks this before the turn
/// starts and refuses the command there.
#[must_use]
pub fn command_outliving_support_session(
    text: &str,
    command_registry: &Arc<CommandRegistry>,
    command_handler_registry: &CommandHandlerRegistry,
) -> Option<String> {
    if !text.trim().starts_with('/') {
        return None;
    }
    let parsed =
        CommandMatcher::from_registry(command_registry).try_match(text, command_registry)?;
    command_handler_registry
        .get(&parsed.name)
        .filter(|handler| handler.outlives_support_session(&parsed.args))
        .map(|_| parsed.name)
}

/// Try to execute `text` as a slash command.
///
/// This is the single dispatch authority for every chat surface — messaging
/// channels, web chat, mobile chat, Slack ops buttons. Channel-specific
/// framing (persistence, card-to-payload conversion, AG-UI lifecycle)
/// belongs at the caller; this function's job is only to parse the text,
/// look up the handler, execute it, and record analytics.
///
/// # Errors
///
/// Returns an error if the handler itself returns an error, and the
/// `max_active_conversations` refusal when a command that carries its thread
/// on (see [`crate::CommandHandler::resumes_thread`]) is typed into an
/// archived thread at the cap — the handler never runs.
pub async fn try_dispatch(req: DispatchRequest<'_>) -> AppResult<DispatchOutcome> {
    // Fast path: not a slash command.
    if !req.text.trim().starts_with('/') {
        return Ok(DispatchOutcome::NotACommand);
    }

    let cmd_registry = req.command_registry;
    let handler_registry = req.command_handler_registry;

    let matcher = CommandMatcher::from_registry(cmd_registry);
    let Some(parsed) = matcher.try_match(req.text, cmd_registry) else {
        // Slash prefix but no matching command. Emit a localized "unknown
        // command" body so the caller can reply without touching the LLM
        // quota or the pipeline machinery.
        let body = req
            .ctx
            .messaging_strings_registry()
            .get(KEY_UNKNOWN_COMMAND, req.locale);
        // Operational tier: the sink keys on the hashed tenant and drops the
        // user dimension, so emit `tenant_id` inline and omit user.
        info!(
            target: "notify",
            event = "messaging.command_executed",
            tenant_id = %req.tenant_id,
            channel = %req.channel_type,
            command_name = "unknown",
            success = false,
            "slash command executed"
        );
        return Ok(DispatchOutcome::UnknownCommand { body });
    };

    let Some(handler) = handler_registry.get(&parsed.name) else {
        // Command name is registered in the definitions registry but has
        // no handler bound. This is a startup wiring mismatch, not a user
        // typo — return a loud internal error rather than the localized
        // unknown-command reply so admins see the bug.
        return Err(AppError::internal(format!(
            "Command '{}' has no handler registered",
            parsed.name
        )));
    };

    let ctx = PlatformCommandContext {
        user_id: req.user_id,
        tenant_id: req.tenant_id,
        channel_type: req.channel_type.to_owned(),
        args: parsed.args,
        raw_text: parsed.raw_text,
        ctx: Arc::clone(req.ctx),
        locale: req.locale.to_owned(),
        is_direct_message: req.is_direct_message,
        ambient_group_fallback: req.ambient_group_fallback,
        conversation_id: req.conversation_id.map(ToOwned::to_owned),
        conversation_tenant_id: req.conversation_tenant_id,
        sender_id: req.sender_id.map(ToOwned::to_owned),
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::clone(req.tool_runtime),
    };

    let result = match refuse_outliving_command(&ctx, handler.as_ref(), &parsed.name).await {
        Ok(()) => run_handler(&req, &ctx, handler.as_ref()).await,
        Err(refused) => Err(refused),
    };
    let ok = result.is_ok();
    info!(
        target: "notify",
        event = "messaging.command_executed",
        tenant_id = %req.tenant_id,
        channel = %req.channel_type,
        command_name = %parsed.name,
        success = ok,
        "slash command executed"
    );

    let response = result?;
    Ok(DispatchOutcome::Executed {
        command_name: parsed.name,
        response,
        rotated_to: ctx.rotation.taken().cloned(),
        opened_topic: ctx.rotation.opened_topic().map(ToOwned::to_owned),
    })
}

/// Refuse a command whose effect outlives the session when the turn runs in
/// an admin's support session
/// ([`pierre_tool_runtime::context::SUPPORT_SESSION`]), audited (carnet#1634).
///
/// Asked on the text the dispatcher parsed, so a tapped suggestion whose
/// words are such a command is refused too: the chat route's own check reads
/// the postback, which is no command, before the pipeline resolves it.
async fn refuse_outliving_command(
    ctx: &PlatformCommandContext,
    handler: &dyn CommandHandler,
    command: &str,
) -> AppResult<()> {
    if !handler.outlives_support_session(&ctx.args) {
        return Ok(());
    }
    refuse_in_support_session(
        ctx.ctx.repos().admin_audit.as_ref(),
        ctx.user_id,
        &format!("run the {command} command"),
    )
    .await
}

/// Run `handler` for the turn, behind the two checks a command passes before
/// it may write anything.
async fn run_handler(
    req: &DispatchRequest<'_>,
    ctx: &PlatformCommandContext,
    handler: &dyn CommandHandler,
) -> AppResult<CommandResponse> {
    // A guided walk typed in a coach's thread about one athlete is answered
    // with the refusal and nothing else: no walk state, no superseded
    // answers, no profile stamp, and no thread slot taken back for it.
    if handler.opens_guided_walk(&ctx.args) {
        if let Some(refusal) = refused_in_athlete_thread(ctx).await? {
            return Ok(refusal);
        }
    }
    // A command that carries its thread on, typed by the athlete into a
    // thread `/reset` archived, takes a quota slot back before it runs — or
    // is refused at the cap before it writes anything. The refusal is the
    // command's outcome, recorded like any other failure.
    if req.origin == TurnOrigin::Athlete && handler.resumes_thread(&ctx.args) {
        if let Some(conversation_id) = req.conversation_id {
            reactivate_for_turn(
                req.ctx.repos(),
                req.ctx.admin_config().as_deref(),
                &req.user_id.to_string(),
                req.conversation_tenant_id,
                conversation_id,
            )
            .await?;
        }
    }
    handler.execute(ctx).await
}
