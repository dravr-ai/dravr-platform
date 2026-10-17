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
use pierre_core::models::{AgentRuntimeContext, MemberFitnessSnapshot, OnboardingState};
use pierre_core::narration;
use pierre_database::database::ConversationRecord;
use pierre_llm::call_record::complete_recorded;
use pierre_llm::provider_stop::ProviderStop;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest, ChatResponse};
use pierre_services::prompt_leak;
use pierre_tool_runtime::coach_seat::TurnSeat;
use tracing::{info, warn};

use crate::envelope::ReconnectPrompt;
use crate::hooks::PipelineHooks;
use crate::recorders::{
    turn_call_recorder, CAPABILITY_REASK_CALL_TYPE, IDENTITY_LEAK_REASK_CALL_TYPE,
    PEER_CLAIM_VERIFIER_CALL_TYPE,
};
use crate::stages;
use crate::stages::peer_grounding::PeerGroundingBlock;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::{chat_provider_from_resources_arc, ChatPipelineContext};
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

/// One bounded re-ask when the model answered as the provider instead of the agent.
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
    input: &TurnInput,
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
    let recorder = turn_call_recorder(ctx, input, IDENTITY_LEAK_REASK_CALL_TYPE);
    apply_reask_outcome(
        complete_recorded(provider.as_ref(), &request, Some(&recorder)).await,
        result,
    );
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
/// A completed re-ask is judged by [`apply_identity_reask_reply`]; a failed
/// dispatch leaves the leaked reply for Stage 15.4 to withhold.
///
/// Split out purely to keep [`reask_after_identity_leak`] inside the
/// cognitive-complexity budget; it adds no public API surface.
fn apply_reask_outcome(outcome: Result<ChatResponse, AppError>, result: &mut ToolLoopResult) {
    match outcome {
        Ok(response) => apply_identity_reask_reply(response, result),
        Err(e) => warn!(
            error = %e,
            "re-ask after a model-identity leak failed to dispatch; withholding as before"
        ),
    }
}

/// The completed re-ask: taken only when it is an identity-clean answer.
///
/// Clean means an answer as well as identity-clean. The provider
/// intermittently ends a completion empty or as a dangling fragment, and an
/// empty completion carries no identity break by construction. Taking one
/// swaps the withheld reply for nothing: the athlete reads the empty-turn
/// apology (« je n'ai pas réussi à formuler une réponse ») instead of the
/// withhold, and the leak never reaches the boundary that records it
/// (carnet#108). So a degenerate reply is held to the rule every other
/// re-ask acceptance in the pipeline applies — a fragment is a lost turn,
/// not an answer. Rejecting one costs at most what the re-ask exists to save:
/// the leaked reply goes on to Stage 15.4 and is withheld, exactly as it is
/// with no re-ask at all.
///
/// Split from [`apply_reask_outcome`] for the same cognitive-complexity
/// budget, the way capability recovery splits its own re-ask.
fn apply_identity_reask_reply(response: ChatResponse, result: &mut ToolLoopResult) {
    if narration::is_degenerate_reply(&response.content) {
        warn!(
            reply_len = response.content.len(),
            "identity_leak_reask_degenerate: re-ask after a model-identity leak \
             carried no answer; withholding as before"
        );
    } else if narration::identity_leak_match(&response.content).is_none() {
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
    } else {
        warn!(
            reply_len = response.content.len(),
            "re-ask after a model-identity leak leaked again; withholding as before"
        );
    }
}

/// Bundled inputs for [`run_recovery_and_post_process`].
pub struct RecoveryAndPostProcessInputs<'a> {
    pub ctx: &'a ChatPipelineContext,
    pub input: &'a TurnInput,
    pub profile: &'a SurfaceProfile,
    pub conv: &'a ConversationRecord,
    pub agent_ctx: Option<&'a AgentRuntimeContext>,
    pub prompt_guard: &'a prompt_leak::PromptGuard,
    /// The turn's assembled messages, replayed verbatim by the identity re-ask.
    pub llm_messages: &'a [ChatMessage],
    /// Model the turn ran on, so the re-ask does not drift to another one.
    pub active_model: &'a str,
    /// Group roster (empty outside a group conversation) for the claim verifier.
    pub peer_roster: &'a [MemberFitnessSnapshot],
    /// The peer blocks the grounding stage injected this turn.
    pub peer_blocks: &'a [PeerGroundingBlock],
    /// The guided flow that owns this turn, as loaded at its start, when one
    /// does — an interview turn is judged differently by capability recovery.
    pub guided_walk: Option<&'a OnboardingState>,
    /// Whose data the turn's tools ran against: in a coach's thread about one
    /// athlete, capability recovery never fetches the sender's own activities.
    pub seat: TurnSeat,
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
        agent_ctx,
        prompt_guard,
        llm_messages,
        active_model,
        peer_roster,
        peer_blocks,
        guided_walk,
        seat,
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
                provider_stop: ProviderStop::Complete,
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
    // agent claimed «problème de connexion de mon côté» on turns with zero
    // tool calls against a healthy provider).
    //
    // Cloned so the stamp net below can tell whether the stage replaced the
    // tool loop's reply — a few KB of text once per turn.
    let content_before_recovery = result.content.clone();
    let reask_recorder = turn_call_recorder(ctx, input, CAPABILITY_REASK_CALL_TYPE);
    let verifier_recorder = turn_call_recorder(ctx, input, PEER_CLAIM_VERIFIER_CALL_TYPE);
    stages::capability_recovery::apply_capability_recovery(
        stages::capability_recovery::CapabilityRecoveryDeps {
            ctx,
            llm_messages,
            active_model,
            peer_roster,
            peer_blocks,
            locale: &profile.locale,
            guided_walk,
            seat,
            reask_recorder: &reask_recorder,
            verifier_recorder: &verifier_recorder,
        },
        input,
        result,
    )
    .await;
    let recovery_replaced_reply = result.content != content_before_recovery;

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
                provider_stop: ProviderStop::Complete,
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
    // leaks again, carries no answer, errors, or finds no provider handle,
    // Stage 15.4 withholds exactly as it did before.
    //
    // Cloned for the same one comparison as `content_before_recovery` above.
    let content_before_identity_reask = result.content.clone();
    reask_after_identity_leak(ctx, input, llm_messages, active_model, result).await;
    stamp_reinstated_denial(
        recovery_replaced_reply,
        result.content != content_before_identity_reask,
        result,
    );

    // The provider's stop describes the reply the tool loop returned. Once a
    // recovery stage or the identity re-ask replaced that reply — with a
    // reconnect message, or with a re-ask whose own stop was never read — it
    // describes nothing the athlete will see.
    let provider_stop = if result.content == content_before_recovery {
        ProviderStop::from_finish_reason(result.finish_reason.as_deref())
    } else {
        ProviderStop::Complete
    };

    // Cloned rather than borrowed: `mem::take` below needs `&mut result`, and a
    // simultaneous immutable borrow of a sibling field does not survive being
    // packed into the struct literal. A handful of tool names per turn.
    let tools_called = result.tools_called.clone();

    let post_processed = stages::post_process::post_process_assistant_reply(
        stages::post_process::PostProcessInputs {
            ctx,
            input,
            conv,
            agent_ctx,
            prompt_guard,
            profile,
            tools_called: &tools_called,
            // A tool ran, or the prefetch put an activity block in the prompt.
            // Either way the reply has evidence behind it and may cite it.
            turn_was_grounded: !tools_called.is_empty()
                || stages::capability_recovery::turn_carries_activity_block(llm_messages),
            // A roster tool ran, or the room's member cards were in the prompt:
            // either way a data block in the reply may be another athlete's.
            roster_data_read: !peer_roster.is_empty()
                || stages::persona_conformance::turn_read_roster_data(&tools_called),
            active_model,
            provider_stop,
        },
        mem::take(&mut result.content),
        hooks,
    )
    .await;
    (post_processed, recovery.prompt)
}

/// Stamp the reply when the identity re-ask reinstated a denial the
/// capability-recovery stage had just disproven.
///
/// The recovery stage replaces a disproven data-access claim with a reply
/// built on a verification payload that lives only in its own re-ask request,
/// never in `llm_messages`. When that replacement itself carries a
/// model-identity leak, [`reask_after_identity_leak`] re-samples the turn over
/// the PRE-recovery messages — without the fetched evidence — and accepts
/// anything identity-clean, which can be the very denial the fetch disproved
/// (« je n'ai jamais eu accès à l'historique de Jean-Daniel »). Delivery stays
/// honest — the athlete reads what the model said — but the row is stamped
/// `capability_claim_unverified` so the disproven claim never replays into a
/// later prompt as established fact.
///
/// Phrase-free on the turn's history: the net keys on the two content
/// transitions (recovery replaced the reply, the re-ask changed it again) and
/// re-checks only the FINAL content against the capability-failure and
/// peer-denial registers, so it holds for any wording either re-ask produces.
fn stamp_reinstated_denial(
    recovery_replaced_reply: bool,
    reask_changed_reply: bool,
    result: &mut ToolLoopResult,
) {
    if !(recovery_replaced_reply && reask_changed_reply) {
        return;
    }
    if !(narration::contains_capability_failure(&result.content)
        || narration::contains_peer_access_denial(&result.content))
    {
        return;
    }
    warn!(
        reply_len = result.content.len(),
        "identity_reask_reinstated_denial: the identity re-ask re-sampled over the \
         pre-recovery messages and brought back a disproven capability denial; \
         stamping the row so it never replays"
    );
    result.capability_claim_unverified = true;
}

#[cfg(test)]
mod tests {
    //! What the identity re-ask takes in place of a withheld reply.
    //!
    //! The re-ask accepts only an identity-clean ANSWER. A completion that is
    //! identity-clean but empty or a fragment must leave the leaked reply in
    //! place for Stage 15.4 to withhold; replacing it blanks the turn and
    //! hands the athlete the empty-turn apology (carnet#108).

    use pierre_llm::ChatResponse;
    use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

    use super::apply_reask_outcome;

    /// Verbatim shape of the 2026-08-05 production identity break.
    const LEAKED: &str = "I'm GitHub Copilot CLI, a terminal-based coding assistant — \
                          I don't have access to fitness tools.";

    /// A turn whose reply broke persona, as the tool loop hands it over.
    fn leaked_turn() -> ToolLoopResult {
        ToolLoopResult {
            content: LEAKED.to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            activity_list: None,
            tool_calls_count: 1,
            tools_called: vec!["get_activities".to_owned()],
            pending_provider_auth_required: None,
            served_without_provider: None,
            guardian_denied: None,
            guardian_confirm: None,
            capability_claim_unverified: false,
            provider_warnings: Vec::new(),
        }
    }

    /// The re-ask's completion, as the provider returns it.
    fn reask(content: &str) -> ChatResponse {
        ChatResponse {
            content: content.to_owned(),
            model: "claude-sonnet-5".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        }
    }

    /// The live class: the provider ends the completion with nothing and
    /// `finish_reason` still "stop", which the chain hands back as `Ok` once
    /// no tier is left to ask.
    #[test]
    fn an_empty_reask_leaves_the_leak_for_the_withhold() {
        let mut result = leaked_turn();
        apply_reask_outcome(Ok(reask("")), &mut result);
        assert_eq!(
            result.content, LEAKED,
            "an empty re-ask must not replace the withheld reply — the athlete \
             would get the empty-turn apology instead of the withhold"
        );
    }

    /// «by Dravr.» is the fragment a live Telegram group received on
    /// 2026-08-22: identity-clean, and no answer.
    #[test]
    fn a_fragment_reask_leaves_the_leak_for_the_withhold() {
        let mut result = leaked_turn();
        apply_reask_outcome(Ok(reask("by Dravr.")), &mut result);
        assert_eq!(result.content, LEAKED);
    }

    /// The guard must not swallow the re-ask's purpose: a real answer still
    /// replaces the leak.
    #[test]
    fn an_answering_reask_replaces_the_leak() {
        let answer = "Pour tes descentes, vise 900 m de dénivelé négatif par semaine.";
        let mut result = leaked_turn();
        apply_reask_outcome(Ok(reask(answer)), &mut result);
        assert_eq!(result.content, answer);
    }
}
