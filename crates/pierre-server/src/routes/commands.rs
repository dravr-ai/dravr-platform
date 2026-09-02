// ABOUTME: GET /api/commands — the slash commands THIS caller may actually run
// ABOUTME: Same registry, same arg signatures and same availability predicates /help resolves
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The per-caller slash-command catalogue.
//!
//! Every command works identically on every chat surface, but only
//! messaging ever advertised them — through `/help`, a rendered block of prose
//! a client cannot offer as an affordance. The in-app clients had no way to
//! learn a command existed short of being told.
//!
//! This endpoint is the machine-readable half of `/help`. It is deliberately
//! *not* a static list:
//! [`HelpHandler`](pierre_commands::help::HelpHandler) resolves per caller by
//! asking each handler whether it would refuse them, and so does this. A
//! palette that offers `/group invite` to an athlete who belongs to no group
//! is a worse affordance than no palette at all — the athlete learns the
//! command exists and that it does not work, in that order.
//!
//! Every field comes from the `commands/**/*.md` frontmatter by way of the
//! registry the server already serves: the command string, its argument
//! signature and its one-line description — the last one resolved through the
//! five-locale strings registry in the caller's language, exactly as `/help`
//! resolves it. Nothing here is a literal, so a command added to the catalogue
//! appears in the palette with no client change and no second list to keep in
//! step.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::warn;

use pierre_commands::{caller_group_standing, CallerGroupStanding, PlatformCommandContext};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::errors::AppError;
use pierre_core::models::default_locale;
use pierre_messaging::commands::CommandDefinition;
use pierre_middleware::extract_auth_from_headers;
use pierre_runtime_context::{resolve_tenant, tenant::require, CommandCtx, TenantMode};
use pierre_tool_runtime::runtime::ToolRuntime;

use crate::mcp::resources::ServerContext;

/// Channel label recorded on the context this route builds.
///
/// The listing does not vary with it: the only per-caller question a command
/// answers is [`CallerGroupStanding`], which reads the caller's memberships and
/// the conversation's group, never the channel. The field is named for what
/// this request is — a catalogue read over the REST API — rather than borrowed
/// from a chat turn that is not happening.
const CATALOGUE_CHANNEL: &str = "command_catalogue";

/// Query parameters for [`list_commands`].
#[derive(Debug, Deserialize)]
pub struct CommandCatalogueQuery {
    /// The conversation the palette is open in, when there is one.
    ///
    /// Group-scoped commands resolve the group *bound to the conversation*,
    /// and a conversation that binds none is a personal thread, so the same
    /// athlete gets a different answer in a group-bound conversation than in
    /// a solo one. Omitting it answers for the caller's memberships alone,
    /// which is the right answer for a palette opened outside any
    /// conversation.
    pub conversation_id: Option<String>,
}

/// One command the caller may run.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandEntry {
    /// Catalogue id and handler-registry key (`group-invite`).
    pub name: String,
    /// The string the athlete types (`/group invite`).
    pub command: String,
    /// Argument signature from the same frontmatter (`yes|no`, `[week|today]`).
    /// Absent for a command that takes no arguments.
    pub args: Option<String>,
    /// One-line description in the caller's locale, the same text `/help`
    /// prints.
    pub description: String,
    /// Domain grouping (`general`, `group`, `coach`, `data`, ...).
    pub domain: String,
}

/// Everything a client needs to draw a command palette.
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandCatalogueResponse {
    /// The caller's runnable commands, ordered by domain then command string.
    ///
    /// Empty is a real answer, not a failure: a server built without a command
    /// catalogue has no commands to offer, and a client that renders nothing
    /// for an empty list is correct.
    pub commands: Vec<CommandEntry>,
}

/// Build one response entry from a catalogue definition.
///
/// `arg_specs` is `None` on a build with no catalogue loaded, which is the same
/// answer as a command that declares no arguments: no signature to render.
fn entry(
    definition: &CommandDefinition,
    arg_specs: Option<&HashMap<String, String>>,
    strings: &MessagingStringsRegistry,
    locale: &str,
) -> CommandEntry {
    CommandEntry {
        name: definition.name.clone(),
        command: definition.command.clone(),
        args: arg_specs
            .and_then(|specs| specs.get(&definition.name))
            .cloned(),
        description: strings.command_description(&definition.name, &definition.description, locale),
        domain: definition.domain.clone(),
    }
}

/// Whether `definition` should be listed for a caller with this standing.
///
/// Asks the handler the same question `/help` asks it, for the same reason: the
/// handler is the only thing that knows which group its command acts on, and it
/// enforces the predicate it answers with, so the listing cannot drift from the
/// behaviour. A command with no registered handler is listed — the absence of a
/// handler is not evidence the caller may not run it.
///
/// `standing` is `None` when the lookup failed. Every command is then listed:
/// a failed lookup must never hide a command the caller can actually run.
fn is_listed(
    resources: &Arc<ServerContext>,
    definition: &CommandDefinition,
    standing: Option<&CallerGroupStanding>,
) -> bool {
    let Some(standing) = standing else {
        return true;
    };
    resources
        .common
        .command_handler_registry
        .as_ref()
        .and_then(|registry| registry.get(&definition.name))
        .is_none_or(|handler| handler.is_available(standing))
}

/// Axum handler for `GET /api/commands`.
///
/// Returns the slash commands the authenticated caller may run, in the order a
/// palette should show them. A server with no command catalogue configured
/// answers with an empty list rather than an error, which is the same thing the
/// chat pipeline does with a `/`-prefixed message it has no catalogue for.
///
/// # Errors
///
/// - Authentication failures from the middleware extractor.
/// - Tenant resolution failure when the caller belongs to no tenant.
pub async fn list_commands(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Query(params): Query<CommandCatalogueQuery>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require(resolve_tenant(&resources, &auth, TenantMode::Required).await?)?;

    let Some(registry) = resources.common.command_registry.as_ref() else {
        return Ok((
            StatusCode::OK,
            Json(CommandCatalogueResponse {
                commands: Vec::new(),
            }),
        )
            .into_response());
    };
    let arg_specs = resources.common.command_arg_specs.as_deref();

    // The athlete's stored preference, resolved the same way a chat turn
    // resolves it. Group lookups render their not-found text through it, and
    // every entry's description is read in it.
    let locale = resources
        .common
        .repos
        .users
        .get_global(auth.user_id)
        .await
        .ok()
        .flatten()
        .map_or_else(default_locale, |user| user.locale);

    // The conversation decides what kind of thread the palette is open in: a
    // thread bound to a group is a group conversation, anything else is
    // personal. Read under the caller's identity, so a conversation they
    // cannot open is not found rather than someone else's standing.
    let conversation = match params.conversation_id.as_deref() {
        Some(id) => Some(
            resources
                .common
                .repos
                .chat
                .get_conversation(id, &auth.user_id.to_string(), tenant_id)
                .await?
                .ok_or_else(|| AppError::not_found("Conversation"))?,
        ),
        None => None,
    };
    let is_direct_message = conversation
        .as_ref()
        .is_none_or(|conversation| conversation.group_id.is_none());

    let command_ctx: Arc<dyn CommandCtx> = Arc::<ServerContext>::clone(&resources);
    let tool_runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);
    let ctx = PlatformCommandContext {
        user_id: auth.user_id,
        tenant_id,
        channel_type: CATALOGUE_CHANNEL.to_owned(),
        args: Vec::new(),
        raw_text: String::new(),
        ctx: command_ctx,
        locale,
        is_direct_message,
        // A solo in-app thread is a personal conversation: the athlete's
        // memberships elsewhere do not make `/group invite` work in it.
        ambient_group_fallback: false,
        conversation_id: params.conversation_id,
        // A conversation the caller can open on web or mobile is filed under
        // their own tenant, so the conversation tenant and the caller tenant
        // are the same value here.
        conversation_tenant_id: tenant_id,
        // No channel link behind a REST call, so there is no channel sender.
        sender_id: None,
        tool_runtime,
    };

    let standing = match caller_group_standing(&ctx).await {
        Ok(standing) => Some(standing),
        Err(e) => {
            warn!(
                user_id = %auth.user_id,
                error = %e,
                "command catalogue could not resolve group standing; listing every command"
            );
            None
        }
    };

    let mut commands: Vec<CommandEntry> = registry
        .all_commands()
        .into_iter()
        .filter(|definition| is_listed(&resources, definition, standing.as_ref()))
        .map(|definition| {
            entry(
                definition,
                arg_specs,
                &resources.mcp.messaging_strings_registry,
                &ctx.locale,
            )
        })
        .collect();
    // The registry hands back HashMap values, so without this the palette lists
    // the same commands in a different order on every process start.
    commands.sort_by(|a, b| {
        a.domain
            .cmp(&b.domain)
            .then_with(|| a.command.cmp(&b.command))
    });

    Ok((StatusCode::OK, Json(CommandCatalogueResponse { commands })).into_response())
}

/// Slash-command catalogue routes.
pub struct CommandRoutes;

impl CommandRoutes {
    /// Mount `GET /api/commands` onto a fresh router.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/commands", get(list_commands))
            .with_state(resources)
    }
}
