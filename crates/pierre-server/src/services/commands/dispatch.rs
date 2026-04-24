// ABOUTME: Transport-agnostic slash-command dispatch — the single authority for every chat surface
// ABOUTME: Used by messaging ingress, web chat, mobile chat. Channel-specific framing belongs at callers.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_messaging::commands::{CommandMatcher, CommandResponse};
use uuid::Uuid;

use crate::contremaitre::messaging_strings::KEY_UNKNOWN_COMMAND;
use crate::mcp::resources::ServerResources;
use crate::services::analytics::{analytics, hash_id};

use super::PlatformCommandContext;

/// Minimal request payload for [`try_dispatch`].
///
/// Wraps the fields the dispatcher needs to build a
/// [`PlatformCommandContext`] if and only if the text parses as a command.
/// Callers assemble this lightweight struct from their own auth/resolution
/// layer (messaging ingress, web chat route, mobile chat route, Slack ops
/// button route) and the dispatcher handles parsing, handler lookup,
/// execution, and analytics uniformly.
pub struct DispatchRequest<'a> {
    /// Shared server resources (registries, repos, messaging strings).
    pub resources: &'a Arc<ServerResources>,
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
    /// chat are always `true` (each conversation is per-user); messaging
    /// channels set this from their native chat-kind signal.
    pub is_direct_message: bool,
    /// Raw user input. The dispatcher inspects it for the `/` prefix and
    /// routes accordingly.
    pub text: &'a str,
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
        /// The canonical command name (e.g. `"coach"`, `"group status"`).
        command_name: String,
        /// Handler output. May be a plain text reply or a card with actions.
        response: CommandResponse,
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
/// Returns an error if the command registries are not configured on the
/// server, or if the handler itself returns an error.
pub async fn try_dispatch(req: DispatchRequest<'_>) -> AppResult<DispatchOutcome> {
    // Fast path: not a slash command.
    if !req.text.trim().starts_with('/') {
        return Ok(DispatchOutcome::NotACommand);
    }

    let Some(cmd_registry) = req.resources.command_registry.as_ref() else {
        return Err(AppError::internal(
            "Command registry not configured on ServerResources",
        ));
    };
    let Some(handler_registry) = req.resources.command_handler_registry.as_ref() else {
        return Err(AppError::internal(
            "Command handler registry not configured on ServerResources",
        ));
    };

    let hashed_tenant = hash_id(&req.tenant_id.to_string());
    let hashed_user = hash_id(&req.user_id.to_string());

    let matcher = CommandMatcher::from_registry(cmd_registry);
    let Some(parsed) = matcher.try_match(req.text, cmd_registry) else {
        // Slash prefix but no matching command. Emit a localized "unknown
        // command" body so the caller can reply without touching the LLM
        // quota or the pipeline machinery.
        let body = req
            .resources
            .messaging_strings_registry
            .get(KEY_UNKNOWN_COMMAND, req.locale);
        analytics().track_command_executed(
            req.channel_type,
            &hashed_tenant,
            &hashed_user,
            "unknown",
            false,
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
        resources: Arc::clone(req.resources),
        locale: req.locale.to_owned(),
        is_direct_message: req.is_direct_message,
    };

    let result = handler.execute(&ctx).await;
    let ok = result.is_ok();
    analytics().track_command_executed(
        req.channel_type,
        &hashed_tenant,
        &hashed_user,
        &parsed.name,
        ok,
    );

    let response = result?;
    Ok(DispatchOutcome::Executed {
        command_name: parsed.name,
        response,
    })
}
