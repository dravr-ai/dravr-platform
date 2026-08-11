// ABOUTME: Handlers for /confirm and /deny — resolve a Guardian-parked destructive tool call
// ABOUTME: Single-use owner-checked claim; /confirm re-dispatches the stored call through the executor chokepoint

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Human-in-the-loop resolution for `TaintedDestructive::Confirm`.
//!
//! The Guardian parks the destructive call and the chat pipeline prompts
//! the user with an opaque claim token; these handlers resolve it:
//!
//! - `/confirm <id>` atomically claims the row (single-use, owner + tenant +
//!   expiry checked in one guarded UPDATE) and re-dispatches the stored call
//!   through [`UniversalExecutor::execute_tool`] — the same chokepoint every
//!   transport funnels through, so budgets and tenant tool-disable still apply.
//! - `/deny <id>` claims the row as denied and discards it.
//!
//! The re-dispatch runs under a fresh `confirm-<id>` turn token: the user's
//! explicit consent replaces the taint heuristic that parked the call (a
//! tainted `TurnState` would just park it again), and the token shape keeps
//! confirmed executions greppable in the Guardian's structured logs. The
//! pending row itself is the audit record (status + resolved_at).

use std::fmt::Display;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;
use tracing::{info, warn};

use pierre_contremaitre::messaging_strings::{
    KEY_GUARDIAN_CONFIRM_DENIED, KEY_GUARDIAN_CONFIRM_DONE, KEY_GUARDIAN_CONFIRM_EXPIRED,
    KEY_GUARDIAN_CONFIRM_FAILED, KEY_GUARDIAN_CONFIRM_NOT_FOUND,
};
use pierre_database::repositories::{ClaimOutcome, PendingGuardianAction};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};

use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/confirm <id>` — approve and execute a parked action.
pub struct ConfirmHandler;

/// Handler for `/deny <id>` — cancel a parked action.
pub struct DenyHandler;

#[async_trait]
impl CommandHandler for ConfirmHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        resolve_pending(ctx, "confirmed").await
    }
}

#[async_trait]
impl CommandHandler for DenyHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        resolve_pending(ctx, "denied").await
    }
}

/// Claim the pending action for the calling user and act on the outcome.
///
/// A missing/garbled id deliberately gets the same reply as an unknown one:
/// distinguishing them would let claim tokens be probed for existence.
async fn resolve_pending(
    ctx: &PlatformCommandContext,
    resolution: &str,
) -> Result<CommandResponse, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();

    let id = ctx.args.first().map_or("", String::as_str).trim();
    if id.is_empty() {
        return Ok(CommandResponse::rich_text(reg.render(
            KEY_GUARDIAN_CONFIRM_NOT_FOUND,
            locale,
            &[],
        )));
    }

    let outcome = ctx
        .ctx
        .repos()
        .guardian_actions
        .claim_pending_action(
            id,
            &ctx.user_id.to_string(),
            &ctx.tenant_id.to_string(),
            resolution,
        )
        .await?;

    let action = match outcome {
        ClaimOutcome::NotFound => {
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_GUARDIAN_CONFIRM_NOT_FOUND,
                locale,
                &[],
            )));
        }
        ClaimOutcome::Expired => {
            info!(user_id = %ctx.user_id, pending_id = %id, "guardian pending action expired at resolution");
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_GUARDIAN_CONFIRM_EXPIRED,
                locale,
                &[],
            )));
        }
        ClaimOutcome::Claimed(action) => action,
    };

    if resolution == "denied" {
        info!(
            user_id = %ctx.user_id,
            pending_id = %action.id,
            tool_name = %action.tool_name,
            "guardian pending action denied by user"
        );
        return Ok(CommandResponse::rich_text(reg.render(
            KEY_GUARDIAN_CONFIRM_DENIED,
            locale,
            &[],
        )));
    }

    let executed = execute_confirmed(ctx, &action).await;
    let key = if executed {
        KEY_GUARDIAN_CONFIRM_DONE
    } else {
        KEY_GUARDIAN_CONFIRM_FAILED
    };
    Ok(CommandResponse::rich_text(reg.render(
        key,
        locale,
        &[&action.tool_name],
    )))
}

/// Re-dispatch the confirmed call through the executor chokepoint.
///
/// Returns whether the tool reported success. Failures are logged for the
/// operator; the user gets the localized failure string either way — never
/// the raw tool error, which could echo the stored (possibly injected)
/// arguments.
async fn execute_confirmed(ctx: &PlatformCommandContext, action: &PendingGuardianAction) -> bool {
    let mut executor = UniversalExecutor::new(ctx.tool_runtime.clone())
        .with_turn_token(format!("confirm-{}", action.id));
    if let Some(conversation_id) = action.conversation_id.clone() {
        executor = executor.with_conversation_id(conversation_id);
    }

    let request = UniversalRequest {
        tool_name: action.tool_name.clone(),
        parameters: action.arguments.clone(),
        user_id: action.user_id.clone(),
        protocol: "chat".to_owned(),
        tenant_id: Some(action.tenant_id.clone()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    let outcome = executor.execute_tool(request).await;
    log_confirmed_outcome(ctx, action, &outcome);
    matches!(outcome, Ok(ref response) if response.success)
}

/// Operator-facing log for a confirmed re-dispatch's outcome. Split from
/// [`execute_confirmed`] to keep it under the cognitive-complexity bar.
fn log_confirmed_outcome<E: Display>(
    ctx: &PlatformCommandContext,
    action: &PendingGuardianAction,
    outcome: &Result<UniversalResponse, E>,
) {
    match outcome {
        Ok(response) if response.success => {
            info!(
                user_id = %ctx.user_id,
                pending_id = %action.id,
                tool_name = %action.tool_name,
                "guardian pending action confirmed and executed"
            );
        }
        Ok(response) => {
            warn!(
                user_id = %ctx.user_id,
                pending_id = %action.id,
                tool_name = %action.tool_name,
                error = response.error.as_deref().unwrap_or("(none)"),
                "guardian pending action confirmed but the tool reported failure"
            );
        }
        Err(e) => {
            warn!(
                user_id = %ctx.user_id,
                pending_id = %action.id,
                tool_name = %action.tool_name,
                error = %e,
                "guardian pending action confirmed but dispatch errored"
            );
        }
    }
}
