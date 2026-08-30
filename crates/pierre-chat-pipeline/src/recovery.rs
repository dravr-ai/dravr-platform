// ABOUTME: Identity-leak re-ask, capability recovery, and post-processing for one turn
// ABOUTME: Split out of lib.rs, which sits over its file-size ceiling

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The recovery half of a turn.
//!
//! Everything here runs AFTER the tool loop has produced a reply and before the
//! envelope is built: re-asking when the reply leaked the system prompt,
//! recovering when the model claimed a capability it did not use, and the
//! post-process pass that scans, folds and conforms the reply.
//!
//! Lives in its own module because `lib.rs` is over the file-size ceiling the
//! repo freezes oversized files at, so the turn orchestration there cannot grow
//! — including by the one line needed to hand the resolved model to the persona
//! repair, which is what prompted the split.

use std::mem;
use std::sync::Arc;

use pierre_core::errors::AppError;
use pierre_core::models::{CoachRuntimeContext, MemberFitnessSnapshot};
use pierre_core::narration;
use pierre_database::database::ConversationRecord;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest, ChatResponse};
use pierre_services::prompt_leak;
use tracing::{info, warn};

use crate::envelope::ReconnectPrompt;
use crate::hooks::PipelineHooks;
use crate::stages;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::{chat_provider_from_resources_arc, ChatPipelineContext};
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

/// One bounded re-ask when the model answered as the provider instead of the coach.
///
/// The identity break is a whole-persona failure, so the reply is unusable and
/// the response boundary withholds it — correctly, but the athlete then loses
/// the turn and is asked to resend. Measured over 30 days on `dravr-dev`, that
/// happened on 7 of 217 guided-flow-inactive turns (3.2%), and every one of them
/// was a real question answered with an apology.
///
/// The break is per-completion, not per-conversation: in the conversation that
/// produced the 2026-08-05 Telegram incident the turns immediately before and
/// after answered correctly. A single re-ask therefore converts a ~3.2% lost
/// turn into roughly 0.1%, without needing the underlying cause resolved.
///
/// # Why only the completion is re-run
///
/// `llm_messages` already carries every `<tool_result>` the loop gathered, so
/// the re-ask needs one more completion over the same messages — NOT another
/// pass through [`dispatch_stage`]. Re-entering dispatch would re-execute the
/// turn's tool calls, and those have side effects: `save_training_plan` would
/// write a second plan. A retry that double-saves is worse than a lost turn.
///
/// # Why the identity anchor is NOT re-asserted
///
/// The obvious move is to repeat the persona harder before trying again. The
/// evidence says otherwise: the provider's own refusal text shows it classifies
/// forceful persona instructions as an injection attempt ("looks like an
/// injected/conflicting instruction rather than a legitimate system
/// directive"), and the anchor was already present in every observed break.
/// Re-asserting it would push the retry toward the failure it is trying to
/// escape. A plain re-sample is the evidence-aligned choice.
///
/// Leaves `result` untouched unless the re-ask produced a clean reply, so the
/// existing withhold path stays exactly as it was on failure.
async fn reask_after_identity_leak(
    ctx: &ChatPipelineContext,
    llm_messages: &[ChatMessage],
    active_model: &str,
    result: &mut ToolLoopResult,
) {
    if narration::identity_leak_match(&result.content).is_none() {
        return;
    }
    let Some(provider) = resolve_reask_provider(ctx) else {
        return;
    };
    let request = ChatRequest::new(llm_messages.to_vec()).with_model(active_model);
    apply_reask_outcome(provider.complete(&request).await, result);
}

/// Resolve the provider for a re-ask, or `None` after logging why not.
///
/// Goes through the same factory Stage 11 dispatch uses at
/// `tool_dispatch.rs`. Reading `ctx.llm_provider` directly — as the first
/// implementation did — makes the re-ask dead code in production: the server
/// binary sets `llm_provider: None` and wires `chat_provider` instead, so the
/// early return fired on every live turn while both e2e tests stayed green,
/// because they inject through the very seam production leaves empty.
/// Anything here that needs a provider must ask this factory for one.
///
/// A failure is a wiring bug rather than a transient condition, so it warns
/// instead of returning silently.
fn resolve_reask_provider(ctx: &ChatPipelineContext) -> Option<Arc<ChatProvider>> {
    match chat_provider_from_resources_arc(ctx.chat_provider.as_ref(), ctx.llm_provider.as_ref()) {
        Ok(provider) => Some(provider),
        Err(e) => {
            warn!(
                error = %e,
                "re-ask after a model-identity leak found no provider; withholding as before"
            );
            None
        }
    }
}

/// Take the re-ask's reply if it is clean; otherwise leave the withhold alone.
///
/// Split out purely to keep [`reask_after_identity_leak`] inside the
/// cognitive-complexity budget; it adds no public API surface.
fn apply_reask_outcome(outcome: Result<ChatResponse, AppError>, result: &mut ToolLoopResult) {
    match outcome {
        Ok(response) if narration::identity_leak_match(&response.content).is_none() => {
            // Deliberately NOT `target: "notify"`. A notify event has to be
            // declared in dravr-contremaitre's catalogue, and the test that
            // polices that runs full-suite-only — so an undeclared event greens
            // the branch and reds main after the squash. This is an operational
            // signal, queryable in Cloud Logging like every other measurement
            // behind this change, and it does not need a notification tier.
            info!(
                reply_len = response.content.len(),
                "identity_leak_reask_recovered: re-ask after a model-identity leak \
                 produced a usable reply; the athlete keeps their turn"
            );
            result.content = response.content;
        }
        Ok(response) => {
            warn!(
                reply_len = response.content.len(),
                "re-ask after a model-identity leak leaked again; withholding as before"
            );
        }
        Err(e) => {
            warn!(
                error = %e,
                "re-ask after a model-identity leak failed to dispatch; withholding as before"
            );
        }
    }
}

/// Bundled inputs for [`run_recovery_and_post_process`].
pub struct RecoveryAndPostProcessInputs<'a> {
    pub ctx: &'a ChatPipelineContext,
    pub input: &'a TurnInput,
    pub profile: &'a SurfaceProfile,
    pub conv: &'a ConversationRecord,
    pub coach_ctx: Option<&'a CoachRuntimeContext>,
    pub prompt_guard: &'a prompt_leak::PromptGuard,
    /// The turn's assembled messages, replayed verbatim by the identity re-ask.
    pub llm_messages: &'a [ChatMessage],
    /// Model the turn ran on, so the re-ask does not drift to another one.
    pub active_model: &'a str,
    /// Group roster (empty outside a group conversation) for the claim verifier.
    pub peer_roster: &'a [MemberFitnessSnapshot],
}

/// Wrap stages 14b–18: run auth recovery, then either post-process the model's
/// text or hand back the deterministic re-auth content the stage wrote.
///
/// The second element is the reconnect prompt whenever the re-auth stage minted
/// one — blanked turn and sibling-served turn alike. It rides out separately so
/// the envelope can draw a control on a surface that renders one.
pub async fn run_recovery_and_post_process(
    inputs: RecoveryAndPostProcessInputs<'_>,
    result: &mut ToolLoopResult,
    hooks: &PipelineHooks<'_>,
) -> (
    stages::post_process::PostProcessedReply,
    Option<ReconnectPrompt>,
) {
    let RecoveryAndPostProcessInputs {
        ctx,
        input,
        profile,
        conv,
        coach_ctx,
        prompt_guard,
        llm_messages,
        active_model,
        peer_roster,
    } = inputs;
    // Guardian short-circuits take precedence over re-auth: a tool blocked by
    // the runtime Guardian (enforce mode) renders the deterministic "blocked
    // for safety" reply, and a parked confirm-required call renders the
    // deterministic confirmation ask — both bypassing LLM post-processing and
    // the re-auth mint below. Mutually exclusive by construction (the tool
    // loop short-circuits on whichever fires first).
    let guardian_denied = stages::guardian_denied::apply_guardian_denied(
        &ctx.messaging_strings_registry,
        &profile.locale,
        result,
    );
    let guardian_confirm = !guardian_denied
        && stages::guardian_confirm::apply_guardian_confirm(
            &ctx.messaging_strings_registry,
            &profile.locale,
            result,
        );
    if guardian_denied || guardian_confirm {
        return (
            stages::post_process::PostProcessedReply {
                content: mem::take(&mut result.content),
                #[cfg(feature = "tools-verification")]
                pending_verdicts: Vec::new(),
                content_blocks: None,
                leak_replaced: false,
                identity_leak: None,
                verdict_chips: Vec::new(),
            },
            None,
        );
    }

    // Capability-failure verification: a reply claiming broken data access is
    // adjudicated against one real read-only fetch BEFORE auth recovery runs,
    // so a fetch that fails auth-shaped raises `pending_provider_auth_required`
    // and lands on the same reconnect re-challenge a failed in-loop tool call
    // does, while a fabricated claim is disproven and re-asked away with the
    // fetched data attached (live incidents 2026-07-24/2026-08-11, where the
    // coach claimed «problème de connexion de mon côté» on turns with zero
    // tool calls against a healthy provider).
    stages::capability_recovery::apply_capability_recovery(
        stages::capability_recovery::CapabilityRecoveryDeps {
            ctx,
            llm_messages,
            active_model,
            peer_roster,
        },
        input,
        result,
    )
    .await;

    let recovery = stages::auth_recovery::apply_auth_recovery(
        stages::auth_recovery::AuthRecoveryDeps {
            admin_jwt_secret: &ctx.admin_jwt_secret,
            base_url: &ctx.config.base_url,
            messaging_strings_registry: &ctx.messaging_strings_registry,
            tool_runtime: &ctx.tool_runtime,
            short_links: &ctx.repos.short_links,
        },
        input,
        profile,
        result,
    )
    .await;

    // Only a reply the stage OWNS bypasses what follows: a blanked turn whose
    // mint produced a link, deterministic platform text no model wrote. A turn
    // a sibling served, and a blanked turn whose mint failed, both keep an
    // answer to shape, so they owe the identity re-ask and post-processing.
    if recovery.owns_reply {
        return (
            stages::post_process::PostProcessedReply {
                content: mem::take(&mut result.content),
                #[cfg(feature = "tools-verification")]
                pending_verdicts: Vec::new(),
                content_blocks: None,
                leak_replaced: false,
                identity_leak: None,
                verdict_chips: Vec::new(),
            },
            recovery.prompt,
        );
    }

    // Stage 14a: one bounded re-ask if the model answered as the provider.
    //
    // Deliberately below both short-circuits above: guardian-denied and re-auth
    // replies are deterministic platform text, not model output, so they cannot
    // carry an identity break and must never be re-asked. Deliberately above
    // post-processing so that chain runs exactly once, on whichever reply
    // survived — and post-processing still owns the withhold, so if the re-ask
    // leaks again, errors, or finds no provider handle, Stage 15.4 withholds
    // exactly as it did before.
    reask_after_identity_leak(ctx, llm_messages, active_model, result).await;

    // Cloned rather than borrowed: `mem::take` below needs `&mut result`, and a
    // simultaneous immutable borrow of a sibling field does not survive being
    // packed into the struct literal. A handful of tool names per turn.
    let tools_called = result.tools_called.clone();

    let post_processed = stages::post_process::post_process_assistant_reply(
        stages::post_process::PostProcessInputs {
            ctx,
            input,
            conv,
            coach_ctx,
            prompt_guard,
            profile,
            tools_called: &tools_called,
            active_model,
        },
        mem::take(&mut result.content),
        hooks,
    )
    .await;
    (post_processed, recovery.prompt)
}
