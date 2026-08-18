// ABOUTME: Capability-failure recovery — catches a reply claiming broken data access, verifies, re-asks
// ABOUTME: A verified fetch disproves the claim with real data; an auth-shaped failure routes to the reconnect link
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Capability-failure recovery for the chat pipeline.
//!
//! Live incidents 2026-07-24 and 2026-08-11 (Telegram): the coach answered
//! «Je ne suis pas capable de récupérer tes activités … (problème de
//! connexion de mon côté)» on turns where **no tool was ever invoked** and
//! every sciotte scrape in the surrounding weeks had succeeded. The claim was
//! fabricated, then persisted, then replayed — teaching the model the access
//! was broken (the learned-helplessness loop the replay scrub exists to
//! break, defeated there by a phrasing mutation).
//!
//! The replay scrub can only stop *yesterday's* claim from poisoning
//! *tomorrow*. This stage handles *today's* claim while the turn is still
//! open:
//!
//! 1. Detect an outbound reply carrying capability-failure vocabulary
//!    ([`narration::contains_capability_failure`]).
//! 2. Run one platform-side `get_activities` fetch — read-only, so unlike
//!    re-entering dispatch it cannot double any side-effecting tool call.
//! 3. On success, the claim is disproven: re-ask the model once with the
//!    fetched data attached (same wire shape as an in-loop tool result) and
//!    deliver the corrected reply if it comes back clean.
//! 4. On an auth-shaped failure the claim was *right*: raise
//!    `pending_provider_auth_required` so [`super::auth_recovery`] replaces
//!    the apology with a localized reconnect link — the athlete gets a
//!    re-challenge they can act on instead of a dead end.
//! 5. On any other failure (backpressure, scraper fault) the reply stands —
//!    an honest "can't fetch right now" during a real outage must reach the
//!    athlete.

use std::sync::Arc;

use serde_json::json;
use tracing::{info, warn};

use super::prefetch::{needs_activity_grounding, REFRESH_GROUNDING_LEAD, STARTUP_GROUNDING_LEAD};
use pierre_tool_runtime::tool_execution as chat_tool_loop;

use crate::turn::TurnInput;
use crate::ChatPipelineContext;
use pierre_core::errors::AppError;
use pierre_core::narration;
use pierre_llm::{ChatMessage, ChatRequest, ChatResponse, FunctionResponse};
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_tool_runtime::protocol::{
    UniversalExecutor, UniversalRequest, META_AUTH_REQUIRED_PROVIDER,
};
use pierre_tool_runtime::tool_execution::{format_tool_results_as_text, ToolLoopResult};

/// The read-only verification tool. Every connected provider serves it, so
/// one fetch adjudicates the claim regardless of which backend the athlete
/// uses.
const VERIFICATION_TOOL: &str = "get_activities";

/// How many activities the verification fetch asks for — enough for the
/// model to answer "propose a session from my recent training" without
/// blowing up the re-ask prompt.
const VERIFICATION_LIMIT: u32 = 5;

/// Appended after the fetched data on the re-ask. English on purpose: the
/// platform's system corpus is English and the model answers in the
/// athlete's locale regardless.
const REASK_INSTRUCTION: &str = "Your data tools are connected and working — the activities \
     above were just fetched successfully on your behalf. Answer the athlete's last message \
     using this data, in their language. Do not claim any connection or data-access problem.";

/// Bundled borrows for [`apply_capability_recovery`], mirroring
/// [`super::auth_recovery::AuthRecoveryDeps`].
#[derive(Clone, Copy)]
pub struct CapabilityRecoveryDeps<'a> {
    /// Pipeline context: the tool runtime for the verification executor and
    /// the provider handles for the single re-ask completion.
    pub ctx: &'a ChatPipelineContext,
    /// The turn's assembled messages, extended (not mutated) by the re-ask.
    pub llm_messages: &'a [ChatMessage],
    /// Model the turn ran on, so the re-ask does not drift to another one.
    pub active_model: &'a str,
}

/// Apply capability-failure recovery in place.
///
/// Runs after the Guardian-denied short-circuit and before
/// [`super::auth_recovery::apply_auth_recovery`], so a verification fetch
/// that fails auth-shaped lands on the same reconnect path a failed in-loop
/// tool call does. Leaves `result` untouched unless the verification fetch
/// settled the claim one way or the other.
pub async fn apply_capability_recovery(
    deps: CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &mut ToolLoopResult,
) {
    // A turn the tool loop already routed to auth recovery or the Guardian
    // is getting a deterministic replacement reply — nothing to verify.
    if result.pending_provider_auth_required.is_some() || result.guardian_denied.is_some() {
        return;
    }

    let Some(trigger) = recovery_trigger(&deps, input, result) else {
        return;
    };

    info!(
        trigger = trigger.as_str(),
        tool_calls = result.tool_calls_count,
        reply_len = result.content.len(),
        "capability_recovery_triggered: running a verification fetch"
    );

    match run_verification_fetch(deps.ctx, input).await {
        VerificationOutcome::AuthRequired(provider_slug) => {
            info!(
                provider = %provider_slug,
                "capability_failure_confirmed: verification fetch needs re-auth; \
                 routing to the reconnect re-challenge"
            );
            result.pending_provider_auth_required = Some(provider_slug);
            // The athlete gets an actionable reconnect link, but it describes
            // this moment only — stamp it out of every later prompt.
            result.capability_claim_unverified = true;
        }
        VerificationOutcome::Unverifiable => {
            // The claim may be an honest outage report, so it still reaches the
            // athlete — but the platform could not stand behind it, so it does
            // not get to teach the model anything on a later turn.
            result.capability_claim_unverified = matches!(trigger, RecoveryTrigger::ClaimedFailure);
        }
        VerificationOutcome::Verified(payload) => {
            // The fetch succeeded, so the claim is disproven. Book the
            // verification call like any other tool call this turn —
            // per-turn observability reads `tools_called` to answer "which
            // tools ran".
            result.tool_calls_count += 1;
            result.tools_called.push(VERIFICATION_TOOL.to_owned());
            let before = result.content.clone();
            reask_with_verified_data(&deps, payload, result).await;
            // The re-ask replaces the reply on success. When it does not, the
            // original text survives — and if that text was a data-access
            // claim, the fetch just proved it false, so it must never replay.
            let claim_survived =
                result.content == before && matches!(trigger, RecoveryTrigger::ClaimedFailure);
            result.capability_claim_unverified = claim_survived;
        }
    }
}

/// Why this turn is being verified.
#[derive(Clone, Copy)]
enum RecoveryTrigger {
    /// The reply carries capability-failure vocabulary. Lexical, so it only
    /// ever catches phrasings someone has already seen and catalogued — kept
    /// as the cheap fast path, never relied on alone.
    ClaimedFailure,
    /// The athlete asked a question that needs real activities, the turn made
    /// no tool call, and no activity block was injected for it. Structural:
    /// it holds for any phrasing in any language, including the mutations the
    /// vocabulary has always trailed (2026-07-24 → 08-11 → 08-11 14:15, three
    /// escapes in three weeks). An answer built with no data behind it is the
    /// failure, whatever words it wears.
    UngroundedDataAsk,
}

impl RecoveryTrigger {
    /// Stable telemetry label.
    const fn as_str(self) -> &'static str {
        match self {
            Self::ClaimedFailure => "claimed_failure",
            Self::UngroundedDataAsk => "ungrounded_data_ask",
        }
    }
}

/// Decide whether this turn needs verification, and say why.
///
/// The structural arm is the load-bearing one. The 2026-08-11 root-cause
/// review found the model answers data asks with no tool call and no injected
/// data, then rationalises the gap in whatever words it likes; the words are a
/// symptom the platform has chased three times. `needs_activity_grounding` is
/// the same intent predicate the prefetch stage already trusts to decide a
/// turn needs real activities, so a turn it flags that produced neither a tool
/// call nor an injected block is ungrounded by construction.
fn recovery_trigger(
    deps: &CapabilityRecoveryDeps<'_>,
    input: &TurnInput,
    result: &ToolLoopResult,
) -> Option<RecoveryTrigger> {
    if narration::contains_capability_failure(&result.content) {
        return Some(RecoveryTrigger::ClaimedFailure);
    }
    let ungrounded = needs_activity_grounding(&input.content)
        && result.tool_calls_count == 0
        && !turn_carries_activity_block(deps.llm_messages);
    ungrounded.then_some(RecoveryTrigger::UngroundedDataAsk)
}

/// Whether the prefetch stage put an activity block in this turn's messages.
///
/// Keys on the two lead sentences that stage prepends, so no extra plumbing is
/// needed to learn what it decided. When a block is present the model HAS the
/// data and is told to answer from it without re-fetching — a zero-tool turn
/// is then correct, not ungrounded.
fn turn_carries_activity_block(llm_messages: &[ChatMessage]) -> bool {
    llm_messages.iter().any(|m| {
        m.content.contains(STARTUP_GROUNDING_LEAD) || m.content.contains(REFRESH_GROUNDING_LEAD)
    })
}

/// What one verification fetch concluded about the model's claim.
enum VerificationOutcome {
    /// The fetch needs re-auth: the claim was right but useless — the athlete
    /// needs the reconnect link, not an apology. Carries the provider slug.
    AuthRequired(String),
    /// The fetch failed non-auth (backpressure, scraper fault): the claim may
    /// be an honest outage report, so the reply stands. Already logged.
    Unverifiable,
    /// The fetch succeeded: the claim is disproven. Carries the tool payload.
    Verified(serde_json::Value),
}

/// Run one read-only [`VERIFICATION_TOOL`] fetch and classify what it proves.
///
/// Auth-required rides two shapes: the executor's typed error
/// (`ProtocolError::ProviderAuthRequired`) and a `success: false` response
/// stamped with `META_AUTH_REQUIRED_PROVIDER` (the provider resolver and
/// token layer use this one — including the zero-connections case). The tool
/// loop scans for the same pair at `tool_execution.rs`.
async fn run_verification_fetch(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
) -> VerificationOutcome {
    // Same construction as tool dispatch (stage 9): conversation id for
    // detached-work routing, per-utterance turn token so Guardian budget
    // accumulates on THIS message and resets next one.
    let executor = UniversalExecutor::new(Arc::clone(&ctx.tool_runtime))
        .with_conversation_id(input.conversation_id.clone())
        .with_turn_token(input.turn_id.0.to_string());

    let request = UniversalRequest {
        tool_name: VERIFICATION_TOOL.to_owned(),
        parameters: json!({ "limit": VERIFICATION_LIMIT }),
        user_id: input.user_id.clone(),
        protocol: "chat".to_owned(),
        tenant_id: Some(input.tool_tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    match executor.execute_tool(request).await {
        Ok(response) if response.success => VerificationOutcome::Verified(
            response
                .result
                .unwrap_or_else(|| json!({ "status": "success" })),
        ),
        Ok(response) => {
            let auth_provider = response.metadata.as_ref().and_then(|m| {
                m.get(META_AUTH_REQUIRED_PROVIDER)
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            });
            if let Some(provider_slug) = auth_provider {
                return VerificationOutcome::AuthRequired(provider_slug);
            }
            warn!(
                error = response.error.as_deref().unwrap_or("unknown"),
                "capability_failure_unverifiable: verification fetch reported failure; \
                 leaving the reply as delivered"
            );
            VerificationOutcome::Unverifiable
        }
        Err(e) => {
            if let Some(provider_slug) = e.provider_auth_required_provider() {
                return VerificationOutcome::AuthRequired(provider_slug.to_owned());
            }
            warn!(
                error = %e,
                "capability_failure_unverifiable: verification fetch failed non-auth; \
                 leaving the reply as delivered"
            );
            VerificationOutcome::Unverifiable
        }
    }
}

/// One completion over the turn's messages plus the verified tool result,
/// taken only if it comes back free of capability-failure claims.
async fn reask_with_verified_data(
    deps: &CapabilityRecoveryDeps<'_>,
    payload: serde_json::Value,
    result: &mut ToolLoopResult,
) {
    let Ok(provider) = chat_provider_from_resources_arc(
        deps.ctx.chat_provider.as_ref(),
        deps.ctx.llm_provider.as_ref(),
    ) else {
        // A missing provider handle is a wiring bug, not a transient condition
        // (see `resolve_reask_provider`); the fetched data still proves access
        // works, but without a completion there is nothing to re-ask with.
        warn!(
            "capability recovery found no provider for the re-ask; leaving the reply as delivered"
        );
        return;
    };

    let function_response = FunctionResponse {
        name: VERIFICATION_TOOL.to_owned(),
        response: payload,
    };
    let tool_text = format_tool_results_as_text(&[function_response]);

    let mut messages = deps.llm_messages.to_vec();
    messages.push(ChatMessage::user(format!(
        "{tool_text}\n\n{REASK_INSTRUCTION}"
    )));
    let request = ChatRequest::new(messages).with_model(deps.active_model);

    apply_reask_outcome(provider.complete(&request).await, result);
}

/// Take the re-ask's reply if it is free of capability-failure claims;
/// otherwise leave the original reply alone. Mirrors the identity re-ask's
/// accept-only-if-clean contract.
fn apply_reask_outcome(outcome: Result<ChatResponse, AppError>, result: &mut ToolLoopResult) {
    match outcome {
        Ok(reply) if !narration::contains_capability_failure(&reply.content) => {
            info!(
                reply_len = reply.content.len(),
                "capability_failure_reask_recovered: re-ask with verified data \
                 produced a usable reply; the athlete keeps their turn"
            );
            // Stripped, because this assignment lands downstream of every other
            // strip in the turn. The tool loop cleans its own output and stage
            // 19 cleans the durable copy; a re-ask reply assigned raw here is
            // washed by neither, so on 2026-08-18 a Telegram athlete received
            // the model's `<tool_call>` scaffolding while the persisted row for
            // the same turn was empty.
            //
            // `contains_capability_failure` above is an accept-only-if-clean
            // check for one property. Scaffolding is a second one, and a reply
            // free of failure claims can still be raw tool-call XML — which is
            // exactly the shape a re-challenged provider tends to produce.
            result.content = chat_tool_loop::strip_simulation_artifacts(&reply.content);
        }
        Ok(_) => {
            warn!(
                "capability_failure_reask_persisted: re-ask still claimed broken \
                 access with the data in hand; keeping the original reply"
            );
        }
        Err(e) => {
            warn!(
                error = %e,
                "capability_failure_reask_failed: re-ask did not complete; \
                 keeping the original reply"
            );
        }
    }
}
