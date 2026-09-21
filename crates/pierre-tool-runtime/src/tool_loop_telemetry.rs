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

use pierre_llm::served_tier::ServedTier;
use pierre_llm::{ChatMessage, ChatResponseWithTools, MessageRole};
use tracing::info;

use crate::tool_loop_io::ToolLoopParams;

/// Name of the provider to attribute a completed call to.
///
/// The tier the chain observer reported, or — when nothing was reported,
/// because the provider is not a chain — the provider that was invoked.
pub fn served_provider_name(served: Option<ServedTier>, invoked: &'static str) -> &'static str {
    served.map_or(invoked, |tier| tier.provider)
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
