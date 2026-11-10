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
//!   expiry checked in one guarded UPDATE) and creates the agent.
//! - `/deny <id>` claims the row as denied and discards the draft.

use async_trait::async_trait;
use dravr_canot::commands::CommandResponse;
use pierre_core::errors::AppError;
use tracing::{info, warn};

use pierre_contremaitre::messaging_strings::{
    KEY_AGENT_CREATE_DISCARDED, KEY_GUARDIAN_CONFIRM_EXPIRED, KEY_GUARDIAN_CONFIRM_NOT_FOUND,
};
use pierre_database::repositories::{ClaimOutcome, AGENT_PROPOSAL_ACTION};

use crate::agent_create::create_from_claimed_proposal;
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
