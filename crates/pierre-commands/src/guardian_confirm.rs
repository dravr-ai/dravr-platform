// ABOUTME: Handlers for /confirm and /deny — resolve a Guardian-parked tool call or a parked agent draft
// ABOUTME: Single-use owner-checked claim; a confirmed tool call re-dispatches through the executor chokepoint

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
//! Whatever becomes of a parked tool call, the next turn's prompt can read it in
//! one platform-authored line ([`ParkedOutcome`]). The tool's own report is
//! discarded here and never echoed, on purpose.
//!
//! The re-dispatch runs under a fresh `confirm-<id>` turn token: the user's
//! explicit consent replaces the taint heuristic that parked the call (a
//! tainted `TurnState` would just park it again), and the token shape keeps
//! confirmed executions greppable in the Guardian's structured logs. The token is
//! fresh but the data is not: it runs inside a [`Provenance`] restored from what the
//! parked turn had served, so a write derived from first-party-only data is stamped
//! and gated as one, and what the row cannot say counts as first-party-only. Those log
//! lines are the durable audit of an athlete's answer: the sweep behind
//! `create_pending_action` deletes every elapsed row whatever its status, so a
//! resolved row outlives its five-minute TTL only until the next park. A
//! super admin's denial leaves an `audit_events` row as well.
//!
//! The same store parks the draft `/agent create` proposes, under
//! [`AGENT_PROPOSAL_ACTION`]. The claim path is shared — one token grammar,
//! one single-use rule, one expiry — and routes on the parked action's kind
//! after the claim: a confirmed draft becomes an agent, a confirmed tool call
//! is re-dispatched.

use pierre_core::permissions::scopes::OAuthScope;

use async_trait::async_trait;
use dravr_canot::commands::CommandResponse;
use pierre_core::errors::AppError;
use pierre_core::models::AddMessageParams;
use pierre_core::transport::TransportPolicy;
use serde_json::json;
use tracing::{info, warn};

use pierre_contremaitre::messaging_strings::{
    KEY_AGENT_CREATE_DISCARDED, KEY_GUARDIAN_CONFIRM_DENIED, KEY_GUARDIAN_CONFIRM_DONE,
    KEY_GUARDIAN_CONFIRM_EXPIRED, KEY_GUARDIAN_CONFIRM_FAILED, KEY_GUARDIAN_CONFIRM_NOT_FOUND,
};
use pierre_database::repositories::{ClaimOutcome, PendingGuardianAction, AGENT_PROPOSAL_ACTION};
use pierre_providers::ai_scope::{self, Provenance};
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::protocol::types::never_ran;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use pierre_tool_runtime::protocols::ProtocolError;

use crate::agent_create::create_from_claimed_proposal;
use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/confirm <id>` — approve and execute a parked action.
#[derive(Debug)]
pub struct ConfirmHandler;

/// Handler for `/deny <id>` — cancel a parked action.
#[derive(Debug)]
pub struct DenyHandler;

#[async_trait]
impl CommandHandler for ConfirmHandler {
    /// Completes the action a turn in this thread parked, and can bind the agent a confirmed draft creates to it.
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

/// Claim the pending action `id` for the calling user and act on the outcome.
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

    let id = id.trim();
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
        ClaimOutcome::Expired(action) => {
            info!(
                user_id = %ctx.user_id,
                pending_id = %action.id,
                tool_name = %action.tool_name,
                "guardian pending action expired at resolution"
            );
            leave_outcome_note(ctx, &action, ParkedOutcome::Expired).await;
            return Ok(CommandResponse::rich_text(reg.render(
                KEY_GUARDIAN_CONFIRM_EXPIRED,
                locale,
                &[],
            )));
        }
        ClaimOutcome::Claimed(action) => action,
    };

    let is_agent_draft = action.tool_name == AGENT_PROPOSAL_ACTION;
    if resolution == "denied" {
        info!(
            user_id = %ctx.user_id,
            pending_id = %action.id,
            tool_name = %action.tool_name,
            "guardian pending action denied by user"
        );
        leave_outcome_note(ctx, &action, ParkedOutcome::Denied).await;
        let key = if is_agent_draft {
            KEY_AGENT_CREATE_DISCARDED
        } else {
            KEY_GUARDIAN_CONFIRM_DENIED
        };
        return Ok(CommandResponse::rich_text(reg.render(key, locale, &[])));
    }

    if is_agent_draft {
        return create_from_claimed_proposal(ctx, &action).await;
    }

    let outcome = execute_confirmed(ctx, &action).await;
    leave_outcome_note(ctx, &action, outcome).await;
    let key = if outcome == ParkedOutcome::Succeeded {
        KEY_GUARDIAN_CONFIRM_DONE
    } else {
        KEY_GUARDIAN_CONFIRM_FAILED
    };
    // The title, not the registry id: the athlete approved an action named in
    // words. The id stays in the log lines.
    let display_name = ctx
        .tool_runtime
        .tool_registry()
        .display_name(&action.tool_name);
    Ok(CommandResponse::rich_text(reg.render(
        key,
        locale,
        &[&display_name],
    )))
}

/// What became of a parked tool call, as the next turn's model reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParkedOutcome {
    /// Confirmed, ran, and the tool reported success.
    Succeeded,
    /// Confirmed and ran, and the tool reported failure.
    Failed,
    /// Confirmed, and the chokepoint refused it before the body ran.
    NotRun,
    /// The athlete said no.
    Denied,
    /// The confirmation window closed before the athlete answered.
    Expired,
}

impl ParkedOutcome {
    /// The fields of the note. Fixed by the platform: no argument and no tool
    /// text reach it, since either can carry the content the park fired on.
    fn fields(self) -> serde_json::Value {
        match self {
            Self::Succeeded => json!({"athlete": "confirmed", "ran": true, "success": true}),
            Self::Failed => json!({"athlete": "confirmed", "ran": true, "success": false}),
            Self::NotRun => json!({"athlete": "confirmed", "ran": false}),
            Self::Denied => json!({"athlete": "denied", "ran": false}),
            Self::Expired => json!({"athlete": "did_not_answer_in_time", "ran": false}),
        }
    }
}

/// Leave the next turn's prompt a one-line account of what became of `action`.
///
/// `/confirm` and its reply are stamped as a command turn and dropped from every
/// prompt, and the tool's report is discarded by design, so without this the
/// model never learns whether the action it asked for happened. A `tool_result`
/// row, the channel a tool's output replays through, written to the database so
/// it survives a pod change. Best effort: the athlete's answer has already been
/// acted on, and a lost note must not turn it into an error.
async fn leave_outcome_note(
    ctx: &PlatformCommandContext,
    action: &PendingGuardianAction,
    outcome: ParkedOutcome,
) {
    // A parked agent draft is not a tool call, and its turn has its own rows.
    if action.tool_name == AGENT_PROPOSAL_ACTION {
        return;
    }
    let Some(conversation_id) = action
        .conversation_id
        .as_deref()
        .or(ctx.conversation_id.as_deref())
    else {
        return;
    };
    let content = format!(
        "[Tool Result for {}]: {}",
        action.tool_name,
        outcome.fields()
    );
    let user_id = ctx.user_id.to_string();
    let params = AddMessageParams {
        tenant_id: ctx.conversation_tenant_id,
        conversation_id,
        user_id: &user_id,
        role: "tool_result",
        content: &content,
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
        content_blocks: None,
        transport_policy: TransportPolicy::AnyTransport,
        channel_type: None,
        about_user_id: None,
    };
    if let Err(e) = ctx.ctx.repos().chat.add_message(&params).await {
        warn!(
            user_id = %ctx.user_id,
            pending_id = %action.id,
            tool_name = %action.tool_name,
            error = %e,
            "guardian pending action resolved but its note could not be written"
        );
    }
}

/// Re-dispatch the confirmed call through the executor chokepoint, under the
/// agent and seat of the turn that parked it.
///
/// Every gate a turn's call answers to answers again here, against the state
/// of the world now: a tool disabled since the park, a provider gone, a seat
/// that withholds it. Failures are logged for the team; the user gets the
/// localized failure string either way — never the raw tool error, which could
/// echo the stored (possibly injected) arguments.
async fn execute_confirmed(
    ctx: &PlatformCommandContext,
    action: &PendingGuardianAction,
) -> ParkedOutcome {
    // A row whose seat cannot be read does not run as the athlete's own: that
    // reads a coach's parked call as if the coach were its subject.
    let Some(seat) = action.seat.as_deref().and_then(TurnSeat::from_stored) else {
        warn!(
            user_id = %ctx.user_id,
            pending_id = %action.id,
            tool_name = %action.tool_name,
            seat = action.seat.as_deref().unwrap_or("(none)"),
            "guardian pending action confirmed but its seat is unreadable; not running it"
        );
        return ParkedOutcome::NotRun;
    };
    let request = UniversalRequest {
        tool_name: action.tool_name.clone(),
        parameters: action.arguments.clone(),
        user_id: action.user_id.clone(),
        protocol: "chat".to_owned(),
        tenant_id: Some(action.tenant_id.clone()),
    };
    // Inside the scope, because an executor adopts the provenance it is built under.
    let provenance = Provenance::from_stored(action.turn_provenance.as_deref());
    let outcome = ai_scope::tracking(provenance, async {
        let mut executor = UniversalExecutor::new(ctx.tool_runtime.clone())
            .with_scopes(OAuthScope::self_grant())
            .with_turn_token(format!("confirm-{}", action.id))
            .with_turn_agent(action.turn_agent_id.clone())
            .with_seat(seat);
        if let Some(conversation_id) = action.conversation_id.clone() {
            // The conversation tenant for this re-dispatch is the one the /confirm
            // arrived under — same resolution the original turn's executor carried.
            executor = executor
                .with_conversation_id(conversation_id)
                .with_conversation_tenant(ctx.conversation_tenant_id.as_uuid())
                .with_direct_message(ctx.is_direct_message);
        }
        executor.execute_tool(request).await
    })
    .await;
    log_confirmed_outcome(ctx, action, &outcome);
    match outcome {
        Ok(response) if response.success => ParkedOutcome::Succeeded,
        Ok(response) if never_ran(&response) => ParkedOutcome::NotRun,
        Err(ProtocolError::ToolNotFound { .. } | ProtocolError::PermissionDenied { .. }) => {
            ParkedOutcome::NotRun
        }
        Ok(_) | Err(_) => ParkedOutcome::Failed,
    }
}

/// Operator-facing log for a confirmed re-dispatch's outcome. Split from
/// [`execute_confirmed`] to keep it under the cognitive-complexity bar.
fn log_confirmed_outcome(
    ctx: &PlatformCommandContext,
    action: &PendingGuardianAction,
    outcome: &Result<UniversalResponse, ProtocolError>,
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
                ran = !never_ran(response),
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
