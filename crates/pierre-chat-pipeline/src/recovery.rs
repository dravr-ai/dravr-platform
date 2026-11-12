// ABOUTME: Capability recovery, auth recovery, and post-processing for one turn
// ABOUTME: Split out of lib.rs, which sits over its file-size ceiling

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The recovery half of a turn.
//!
//! Everything here runs AFTER the tool loop has produced a reply and before the
//! envelope is built: recovering when the model claimed a capability it did
//! not use, and the post-process pass that scans, withholds, folds and
//! conforms the reply. A reply that answers as the underlying model rather
//! than the agent is withheld there, at Stage 15.4, never re-asked.
//!
//! Lives in its own module because `lib.rs` is over the file-size ceiling the
//! repo freezes oversized files at, so the turn orchestration there cannot grow
//! — including by the one line needed to hand the resolved model to the persona
//! repair, which is what prompted the split.

use std::mem;

use pierre_core::models::{AgentRuntimeContext, MemberFitnessSnapshot, OnboardingState};
use pierre_database::database::ConversationRecord;
use pierre_llm::provider_stop::ProviderStop;
use pierre_llm::ChatMessage;
use pierre_services::prompt_leak;
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::tool_progress;

use crate::envelope::ReconnectPrompt;
use crate::hooks::PipelineHooks;
use crate::recorders::{
    turn_call_recorder, CAPABILITY_REASK_CALL_TYPE, PEER_CLAIM_VERIFIER_CALL_TYPE,
};
use crate::stages;
use crate::stages::peer_grounding::PeerGroundingBlock;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;
use crate::{TurnEvent, TurnEventSink};
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

/// Bundled inputs for [`run_recovery_and_post_process`].
pub(crate) struct RecoveryAndPostProcessInputs<'a> {
    pub ctx: &'a ChatPipelineContext,
    pub input: &'a TurnInput,
    pub profile: &'a SurfaceProfile,
    pub conv: &'a ConversationRecord,
    pub agent_ctx: Option<&'a AgentRuntimeContext>,
    pub prompt_guard: &'a prompt_leak::PromptGuard,
    /// The turn's assembled messages, extended by capability recovery's re-ask
    /// and read for the activity block that grounds a turn.
    pub llm_messages: &'a [ChatMessage],
    /// Model the turn ran on, so a re-ask or repair does not drift to another one.
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
    /// The sink the tool loop streamed the draft reply to, when it streamed
    /// one: a stage that replaces the draft takes the streamed text back there.
    pub streamed_to: Option<&'a TurnEventSink>,
}

/// The draft reply the tool loop streamed, while it may still be on screen.
///
/// Every stage below judges the whole reply after the athlete has watched it
/// arrive. One that replaces it — a Guardian's deterministic reply, a
/// disproven capability claim re-asked away, a reconnect message, an identity
/// break withheld — takes the streamed text back the moment it
/// knows, so the bubble stops showing words that will not be the reply while
/// the rest of the turn runs. The `done` frame still carries the reply itself.
struct StreamedDraft<'a> {
    sink: Option<&'a TurnEventSink>,
}

impl StreamedDraft<'_> {
    /// Take the streamed draft back, once: later replacements find it gone.
    fn take_back(&mut self) {
        if let Some(sink) = self.sink.take() {
            tool_progress::deliver(sink, TurnEvent::ProseRetract);
        }
    }
}

/// Wrap stages 14b–18: run auth recovery, then either post-process the model's
/// text or hand back the deterministic re-auth content the stage wrote.
///
/// The second element is the reconnect prompt whenever the re-auth stage minted
/// one — blanked turn and sibling-served turn alike. It rides out separately so
/// the envelope can draw a control on a surface that renders one.
pub(crate) async fn run_recovery_and_post_process(
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
        streamed_to,
    } = inputs;
    let mut draft = StreamedDraft { sink: streamed_to };
    // A Guardian block takes precedence over re-auth: a tool blocked by the
    // runtime Guardian renders the deterministic reply saying what
    // went through, bypassing LLM post-processing and the re-auth mint below.
    // Every tool loop checks the Guardian's signal before the auth trip, so a
    // batch that raised both returns the block, and the turn carries one block at
    // most: its first (`headless_finalize::settle_turn_signals`).
    if stages::guardian_denied::apply_guardian_denied(
        &ctx.messaging_strings_registry,
        ctx.tool_runtime.tool_registry(),
        &profile.locale,
        result,
    ) {
        draft.take_back();
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
    if recovery_replaced_reply {
        draft.take_back();
    }

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
    // answer to shape, so they owe post-processing, identity withhold included.
    if recovery.owns_reply {
        draft.take_back();
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

    // The provider's stop describes the reply the tool loop returned. Once a
    // recovery stage replaced that reply — with a reconnect message, or with a
    // re-ask whose own stop was never read — it describes nothing the athlete
    // will see.
    let provider_stop = if result.content == content_before_recovery {
        ProviderStop::from_finish_reason(result.finish_reason.as_deref())
    } else {
        ProviderStop::Complete
    };

    // Cloned rather than borrowed: `mem::take` below needs `&mut result`, and a
    // simultaneous immutable borrow of a sibling field does not survive being
    // packed into the struct literal. A handful of tool names per turn.
    let tools_called = result.tools_called.clone();
    // Taken, not cloned: post-processing is the notice's only reader.
    let envelope_days = mem::take(&mut result.envelope_days);

    let post_processed = stages::post_process::post_process_assistant_reply(
        stages::post_process::PostProcessInputs {
            ctx,
            input,
            conv,
            agent_ctx,
            prompt_guard,
            profile,
            tools_called: &tools_called,
            envelope_days: &envelope_days,
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
    // The response boundary withheld the reply (a canary hit, or an identity
    // break: the model answering as itself rather than the agent) and put the
    // canned string in its place. Every model-written reply reaches it: the
    // only two returns above carry platform text a model never wrote.
    if post_processed.leak_replaced {
        draft.take_back();
    }
    (post_processed, recovery.prompt)
}
