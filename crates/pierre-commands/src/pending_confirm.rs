// ABOUTME: Handlers for /confirm and /deny — resolve the agent draft /agent create parked
// ABOUTME: Single-use owner-checked claim; a confirmed draft becomes an agent, a denied one is discarded

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Resolution of a parked agent draft.
//!
//! `/agent create` parks its draft under an opaque claim token, and these handlers
//! resolve it:
//!
//! - `/confirm <id>` atomically claims the row (single-use, owner + tenant +
//!   expiry checked in one guarded UPDATE) and creates the agent, from the
//!   conversation the draft was proposed in only.
//! - `/deny <id>` claims the row as denied and discards the draft.

use async_trait::async_trait;
use dravr_canot::commands::CommandResponse;
use pierre_core::errors::AppError;
use tracing::{info, warn};

use pierre_contremaitre::messaging_strings::{
    KEY_AGENT_CREATE_DISCARDED, KEY_GUARDIAN_CONFIRM_ELSEWHERE, KEY_GUARDIAN_CONFIRM_EXPIRED,
    KEY_GUARDIAN_CONFIRM_NOT_FOUND, KEY_GUARDIAN_CONFIRM_TYPE_IN_THREAD,
};
use pierre_database::repositories::{ClaimOutcome, AGENT_PROPOSAL_ACTION};

use crate::agent_create::{confirmed_where_drafted, create_from_claimed_proposal};
use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/confirm <id>` — approve a parked draft.
#[derive(Debug)]
pub struct ConfirmHandler;

/// Handler for `/deny <id>` — discard a parked draft.
#[derive(Debug)]
pub struct DenyHandler;

#[async_trait]
impl CommandHandler for ConfirmHandler {
    /// Can bind the agent a confirmed draft creates to this thread.
    fn resumes_thread(&self, _args: &[String]) -> bool {
        true
    }

    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        resolve_pending(
            ctx,
            ctx.args.first().map_or("", String::as_str),
            "confirmed",
        )
        .await
    }
}

#[async_trait]
impl CommandHandler for DenyHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        resolve_pending(ctx, ctx.args.first().map_or("", String::as_str), "denied").await
    }
}

/// Claim the pending draft `id` for the calling user and act on the outcome.
///
/// The one claim behind `/confirm`, `/deny` and `/agent create confirm`.
/// A missing/garbled id deliberately gets the same reply as an unknown one:
/// distinguishing them would let claim tokens be probed for existence.
pub(crate) async fn resolve_pending(
    ctx: &PlatformCommandContext,
    id: &str,
    resolution: &str,
) -> Result<CommandResponse, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();
    let not_found =
        || CommandResponse::rich_text(reg.render(KEY_GUARDIAN_CONFIRM_NOT_FOUND, locale, &[]));

    let id = id.trim();
    if id.is_empty() {
        return Ok(not_found());
    }

    // Only the draft's owner reaches this answer (the read is owner-scoped),
    // so it says plainly where the draft can be confirmed; anyone else is
    // answered as for an unknown code. A surface that names no conversation
    // (a card button in a chat that holds no session) is told what to type
    // in the draft's own conversation.
    if resolution == "confirmed" && confirmed_elsewhere(ctx, id).await? {
        let refusal = if ctx.conversation_id.is_some() {
            reg.render(KEY_GUARDIAN_CONFIRM_ELSEWHERE, locale, &[])
        } else {
            reg.render(
                KEY_GUARDIAN_CONFIRM_TYPE_IN_THREAD,
                locale,
                &[&format!("/agent create confirm {id}")],
            )
        };
        return Ok(CommandResponse::rich_text(refusal));
    }

    let outcome = ctx
        .ctx
        .repos()
        .agent_drafts
        .claim_pending_action(
            id,
            &ctx.user_id.to_string(),
            &ctx.tenant_id.to_string(),
            resolution,
        )
        .await?;

    let action = match outcome {
        ClaimOutcome::NotFound => return Ok(not_found()),
        ClaimOutcome::Expired(action) => {
            info!(
                user_id = %ctx.user_id,
                pending_id = %action.id,
                "parked draft expired at resolution"
            );
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_GUARDIAN_CONFIRM_EXPIRED,
                locale,
                &[],
            )));
        }
        ClaimOutcome::Claimed(action) => action,
    };

    // Only a draft resolves into anything. Any other row an older binary left in
    // the table is never run.
    if action.tool_name != AGENT_PROPOSAL_ACTION {
        warn!(
            user_id = %ctx.user_id,
            pending_id = %action.id,
            tool_name = %action.tool_name,
            "claimed a parked row that is not an agent draft; not running it"
        );
        return Ok(not_found());
    }

    if resolution == "denied" {
        info!(
            user_id = %ctx.user_id,
            pending_id = %action.id,
            "parked draft denied by user"
        );
        return Ok(CommandResponse::rich_text(reg.render(
            KEY_AGENT_CREATE_DISCARDED,
            locale,
            &[],
        )));
    }

    create_from_claimed_proposal(ctx, &action).await
}

/// Whether `/confirm <id>` runs outside the conversation the draft `id` was
/// proposed in ([`confirmed_where_drafted`]).
///
/// Read before the claim, so a confirm typed elsewhere spends nothing: the
/// draft still waits in its own thread. A row that no longer waits is the
/// claim's to answer.
async fn confirmed_elsewhere(ctx: &PlatformCommandContext, id: &str) -> Result<bool, AppError> {
    let waiting = ctx
        .ctx
        .repos()
        .agent_drafts
        .pending_action(id, &ctx.user_id.to_string(), &ctx.tenant_id.to_string())
        .await?;
    let elsewhere = waiting.is_some_and(|action| !confirmed_where_drafted(ctx, &action));
    if elsewhere {
        info!(
            user_id = %ctx.user_id,
            "agent draft confirm refused outside the conversation it was drafted in"
        );
    }
    Ok(elsewhere)
}
