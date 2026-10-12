// ABOUTME: Timing, attribution and logging helpers shared by the tool loops
// ABOUTME: Names who served a call, measures latency, and logs each iteration's wire shape and response
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Telemetry for the tool loops in [`crate::tool_execution`].
//!
//! Kept apart from the loops so they read as control flow: what a loop does on
//! each iteration is there, and how an iteration is timed, attributed and
//! logged is here.

use std::time::Instant;

use pierre_core::errors::AppError;
use pierre_llm::call_record::{emit_call_record_with_text, CallRecordInputs};
use pierre_llm::served_tier::ServedTier;
use pierre_llm::{ChatMessage, ChatProvider, ChatResponseWithTools, MessageRole};
use tracing::{info, warn};

use crate::tool_loop_io::ToolLoopParams;

/// Name of the provider to attribute a completed call to.
///
/// The tier the chain observer reported, or — when nothing was reported,
/// because the provider is not a chain — the provider that was invoked.
pub fn served_provider_name(served: Option<ServedTier>, invoked: &'static str) -> &'static str {
    served.map_or(invoked, |tier| tier.provider)
}

/// Sum an optional per-call token count into a running optional total.
///
/// `None` means the provider reported nothing, which is not the same as a
/// measured zero, so the total stays `None` until some call reports a figure.
pub const fn accumulate_optional(total: Option<u32>, next: Option<u32>) -> Option<u32> {
    match (total, next) {
        (None, None) => None,
        (Some(t), None) => Some(t),
        (None, Some(n)) => Some(n),
        (Some(t), Some(n)) => Some(t.saturating_add(n)),
    }
}

/// Hand one loop call's record to the optional sink, with no text to estimate
/// from: the provider reported its own usage, or nothing can be known.
///
/// Centralises token extraction so the three tool-loop variants share the
/// same recording contract.
pub fn emit_call_record(inputs: CallRecordInputs<'_>) {
    emit_call_record_with_text(inputs, None, None);
}

/// Convert an [`Instant`] elapsed time into milliseconds, saturating
/// at `i64::MAX` for pathologically long calls.
pub fn millis_elapsed(start: Instant) -> i64 {
    let ms = start.elapsed().as_millis();
    i64::try_from(ms).unwrap_or(i64::MAX)
}

/// Log the SHAPE of the message vector handed to the provider — roles, counts
/// and size, never content.
///
/// This exists because the platform spent months unable to answer "did the
/// block we injected actually reach the model?". `copilot_headless` keeps only
/// the FIRST system message and silently filters every other one out of
/// history, and the platform was emitting five: the compaction replay, the
/// same-turn splice, the turn-1 activity pre-load, the Stage 12b refresh and
/// the guardian planner. Four were discarded on every turn. Nothing logged it,
/// so the loss was invisible — the agent still looked grounded whenever it
/// chose to call `get_activities` itself, which is the same observable outcome.
///
/// `system_message_count` is the field that would have shown `5` on the first
/// turn after Stage 12b shipped, eleven days before anyone noticed. The
/// existing counters (`message_count` here and `msg_count` in prompt assembly)
/// count the vector without saying what is IN it, which is exactly the gap.
///
/// Deliberately NOT a `notify` event: this is diagnostic telemetry read from
/// Cloud Logging, and routing it through the notify pipeline would couple it to
/// the `dravr-contremaitre` event catalogue for no operator benefit.
pub fn log_wire_shape(dispatch_path: &'static str, llm_messages: &[ChatMessage]) {
    let mut system_message_count = 0_usize;
    let mut user_message_count = 0_usize;
    let mut assistant_message_count = 0_usize;
    let mut tool_message_count = 0_usize;
    let mut total_chars = 0_usize;

    for message in llm_messages {
        total_chars += message.content.len();
        match message.role {
            MessageRole::System => system_message_count += 1,
            MessageRole::User => user_message_count += 1,
            MessageRole::Assistant => assistant_message_count += 1,
            MessageRole::Tool => tool_message_count += 1,
        }
    }

    info!(
        dispatch_path,
        system_message_count,
        user_message_count,
        assistant_message_count,
        tool_message_count,
        message_count = llm_messages.len(),
        total_chars,
        "wire shape at the provider boundary"
    );
}

/// Emit a structured log marking the start of one tool-loop iteration.
///
/// Extracted from [`run_api_tool_loop`] to keep the loop body inside
/// the workspace cognitive complexity budget. Records the iteration
/// index and resolved provider/model so an operator can tie the call
/// to its eventual `llm_usage` row by `turn_id` + `call_sequence`.
pub fn log_iteration_start(iteration: usize, params: &ToolLoopParams<'_>, message_count: usize) {
    info!(
        iteration,
        provider = params.provider.name(),
        model = params.model,
        message_count,
        "tool loop iteration: dispatching to provider"
    );
}

/// Emit a structured log summarizing the provider's response for one
/// tool-loop iteration.
pub fn log_iteration_response(iteration: usize, latency_ms: i64, response: &ChatResponseWithTools) {
    info!(
        iteration,
        latency_ms,
        content_len = response.content.as_deref().map_or(0, str::len),
        function_calls = response.function_calls.as_ref().map_or(0, Vec::len),
        prompt_tokens = response.usage.as_ref().map_or(0, |u| u.prompt_tokens),
        completion_tokens = response.usage.as_ref().map_or(0, |u| u.completion_tokens),
        "tool loop iteration: provider response received"
    );
}

/// Record a tool loop leaving the chain's head for its tail.
///
/// The tool loops converse with the head alone (the headless loop through its
/// turn provider, the API loop through
/// [`ChatProvider::complete_with_tools_on_head`]), so this hop happens outside
/// embacle's chain walk and the lines embacle and the chain's observer write
/// for one are never written. Both are reproduced here:
///
/// - the `warn!` starts with `fallback: ` and carries the error's message, as
///   embacle's own chain line does, so a quota answer from the head is counted
///   by the tier-quota alert and labelled with the tier named in it
///   (`LLM provider rate limited or out of quota: anthropic#2: …`);
/// - the notify event is the one the chain's observer writes, so a head that
///   fails every tool turn is visible to the chain-fallthrough alert.
///
/// `tool_loop` names the loop that failed (`api`, `headless`).
pub fn note_tool_loop_fallback(
    from: &ChatProvider,
    to: &ChatProvider,
    error: &AppError,
    tool_loop: &'static str,
) {
    warn!(
        tool_loop,
        tier = from.name(),
        primary = from.name(),
        secondary = to.name(),
        code = ?error.code,
        error = %error.message,
        "fallback: tool loop head failed, re-running the turn on the chain's tail"
    );
    info!(
        target: "notify",
        event = "embacle.fallback_triggered",
        from_provider = from.name(),
        to_provider = to.name(),
        reason = ?error.code,
        tool_loop,
        "Runtime LLM fallback engaged on a tool loop head fault"
    );
}

/// Record a tool loop passing over a chain head inside its quota cooldown.
///
/// Mirrors what embacle's chain does for the same skip: no `fallback: ` line
/// (that line is a refusal, and the tier-quota alert counts it — this one was
/// counted when the window started), and no "Runtime LLM fallback engaged"
/// event (nothing failed: the head was not asked). The turn's provider
/// attribution already names the tier that served it.
pub fn note_tool_loop_cooldown_skip(
    from: &ChatProvider,
    to: &ChatProvider,
    tool_loop: &'static str,
) {
    let remaining_secs = from
        .quota_cooldowns()
        .and_then(|book| book.remaining(from.name()))
        .map_or(0, |left| left.as_secs());
    info!(
        tool_loop,
        tier = from.name(),
        next = to.name(),
        remaining_secs,
        "tool loop: chain head is in a quota cooldown; the turn goes to the chain's tail"
    );
}
