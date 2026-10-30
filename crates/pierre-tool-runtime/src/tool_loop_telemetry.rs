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

/// Log target of the wire-shape line, which a log-based metric selects on.
const WIRE_SHAPE_LOG_TARGET: &str = "pierre_tool_runtime::tool_execution";

/// Name of the provider to attribute a completed call to.
///
/// The tier the chain observer reported, or — when nothing was reported,
/// because the provider is not a chain — the provider that was invoked.
pub(crate) fn served_provider_name(
    served: Option<ServedTier>,
    invoked: &'static str,
) -> &'static str {
    served.map_or(invoked, |tier| tier.provider)
}

/// Sum an optional per-call token count into a running optional total.
///
/// `None` means the provider reported nothing, which is not the same as a
/// measured zero, so the total stays `None` until some call reports a figure.
pub(crate) const fn accumulate_optional(total: Option<u32>, next: Option<u32>) -> Option<u32> {
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
pub(crate) fn emit_call_record(inputs: CallRecordInputs<'_>) {
    emit_call_record_with_text(inputs, None, None);
}

/// Convert an [`Instant`] elapsed time into milliseconds, saturating
/// at `i64::MAX` for pathologically long calls.
pub(crate) fn millis_elapsed(start: Instant) -> i64 {
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
pub(crate) fn log_wire_shape(dispatch_path: &'static str, llm_messages: &[ChatMessage]) {
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

    // The target is spelled out because the `dravr-wire-shape-multi-system`
    // log metric (infra/environments/dev/wire_shape_monitoring.tf) selects on
    // it: the default target is this module's path, which moves whenever the
    // function does and leaves the alert matching nothing.
    info!(
        target: WIRE_SHAPE_LOG_TARGET,
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
/// Records the iteration index, the call's sequence number within the turn
/// and the provider/model the call asks, so an operator can tie the call to
/// its eventual `llm_usage` row by `turn_id` + `call_sequence`. An iteration
/// re-asked on the turn's model is two calls, so the two numbers diverge.
pub(crate) fn log_iteration_start(
    iteration: usize,
    call_sequence: i64,
    params: &ToolLoopParams<'_>,
    model: &str,
    message_count: usize,
) {
    info!(
        iteration,
        call_sequence,
        provider = params.provider.name(),
        model,
        message_count,
        "tool loop iteration: dispatching to provider"
    );
}

/// Emit a structured log summarizing the provider's response for one
/// tool-loop iteration.
///
/// The line names the provider and model that answered this call: an
/// iteration re-asked on a cheaper model would otherwise inherit the turn's
/// `model` from the `chat.turn` span, and twelve Haiku calls read as Opus
/// (carnet#1194). The cache split rides along so the line and the
/// `llm call cost` line for the same `call_sequence` agree without a back-solve.
pub(crate) fn log_iteration_response(
    iteration: usize,
    call_sequence: Option<i64>,
    provider: &str,
    latency_ms: i64,
    response: &ChatResponseWithTools,
) {
    let usage = response.usage.as_ref();
    info!(
        iteration,
        call_sequence,
        provider,
        model = %response.model,
        latency_ms,
        content_len = response.content.as_deref().map_or(0, str::len),
        function_calls = response.function_calls.as_ref().map_or(0, Vec::len),
        prompt_tokens = usage.map_or(0, |u| u.prompt_tokens),
        completion_tokens = usage.map_or(0, |u| u.completion_tokens),
        cached_read_tokens = usage.and_then(|u| u.cached_read_tokens).unwrap_or(0),
        cached_write_tokens = usage.and_then(|u| u.cached_write_tokens).unwrap_or(0),
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
pub(crate) fn note_tool_loop_fallback(
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
pub(crate) fn note_tool_loop_cooldown_skip(
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt::Debug;
    use std::sync::{Arc, Mutex, PoisonError};

    use pierre_llm::TokenUsage;
    use tracing::field::{Field, Visit};
    use tracing::{callsite, subscriber, Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::Registry;

    use super::*;

    /// The fields of every iteration-response line logged while installed.
    #[derive(Clone, Default)]
    struct IterationLines(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

    struct FieldText<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for FieldText<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
    }

    impl<S: Subscriber> Layer<S> for IterationLines {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = BTreeMap::new();
            event.record(&mut FieldText(&mut fields));
            if fields.get("message").map(String::as_str)
                == Some("tool loop iteration: provider response received")
            {
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(fields);
            }
        }
    }

    /// An iteration re-asked on a cheaper model is logged under that model
    /// and the tier that answered, with its sequence and cache split, rather
    /// than inheriting the turn's model from the span (carnet#1194).
    #[test]
    fn the_iteration_line_names_the_model_that_answered() {
        let response = ChatResponseWithTools {
            content: Some("done".to_owned()),
            function_calls: None,
            model: "claude-haiku-5-5".to_owned(),
            usage: Some(
                TokenUsage::new(82_235, 6_398, 88_633).with_cache(Some(73_065), Some(9_170)),
            ),
            finish_reason: None,
            warnings: None,
        };
        let lines = IterationLines::default();
        subscriber::with_default(Registry::default().with(lines.clone()), || {
            callsite::rebuild_interest_cache();
            log_iteration_response(3, Some(6), "anthropic", 1_200, &response);
        });
        let lines = lines.0.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(lines.len(), 1, "one line per iteration: {lines:?}");
        let line = lines.first().expect("one iteration line"); // Safe: asserted above
        let field = |name: &str| line.get(name).map(String::as_str);
        assert_eq!(field("iteration"), Some("3"));
        assert_eq!(field("call_sequence"), Some("6"));
        assert_eq!(field("provider"), Some("anthropic"));
        assert_eq!(field("model"), Some("claude-haiku-5-5"));
        assert_eq!(field("latency_ms"), Some("1200"));
        assert_eq!(field("prompt_tokens"), Some("82235"));
        assert_eq!(field("completion_tokens"), Some("6398"));
        assert_eq!(field("cached_read_tokens"), Some("73065"));
        assert_eq!(field("cached_write_tokens"), Some("9170"));
    }
}
