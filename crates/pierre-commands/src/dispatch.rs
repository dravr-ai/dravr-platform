// ABOUTME: Transport-agnostic slash-command dispatch — the single authority for every chat surface
// ABOUTME: Used by messaging ingress, web chat, mobile chat. Channel-specific framing belongs at callers.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_contremaitre::messaging_strings::KEY_UNKNOWN_COMMAND;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_messaging::commands::{CommandMatcher, CommandRegistry, CommandResponse};
use pierre_runtime_context::CommandCtx;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::info;
use uuid::Uuid;

use crate::{CommandHandlerRegistry, ConversationRotation, PlatformCommandContext};

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
    /// MCP tools (`/confirm`). Same concrete `ServerContext` as `ctx`,
    /// behind a separate trait to avoid a crate cycle through
    /// `pierre-runtime-context`.
    pub tool_runtime: &'a Arc<dyn ToolRuntime>,
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
        /// The command definition's `name:` id (e.g. `"coach"`,
        /// `"group-status"`) — the handler-registry key, NOT the spaced
        /// `/group status` trigger the user typed.
        command_name: String,
        /// Handler output. May be a plain text reply or a card with actions.
        response: CommandResponse,
        /// The conversation the athlete is now on, when the handler moved
        /// them — `/reset` and nothing else today. The surface follows it:
        /// a messaging channel already had its session repointed, the in-app
        /// clients open the id.
        rotated_to: Option<String>,
    },
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
/// Returns an error if the handler itself returns an error.
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
        // unknown-command reply so operators see the bug.
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

    let result = handler.execute(&ctx).await;
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
        rotated_to: ctx.rotation.taken().map(ToOwned::to_owned),
    })
}
