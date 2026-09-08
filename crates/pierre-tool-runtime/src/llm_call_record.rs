// ABOUTME: What one recorded LLM call is — the per-call metric and its sink trait
// ABOUTME: Split out of tool_execution.rs so the loop file stops carrying the recording vocabulary
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-call usage recording types.
//!
//! The tool loop measures one record per provider completion and hands it to
//! an [`LlmCallRecorder`]. Keeping the vocabulary here rather than in the loop
//! means the billing pipeline can depend on the shape of a call without
//! depending on how the loop runs.

use std::sync::Arc;

use pierre_core::llm::TokenUsage;
use pierre_core::tokens::{estimate_chat_tokens, estimate_prompt_tokens};

/// Decide the `(prompt_tokens, completion_tokens, estimated)` triple for one
/// recorded call.
///
/// A provider that reports its own usage is believed outright. When it reports
/// none — every CLI runner does, Copilot ACP included — the counts come from
/// the text that actually crossed the wire:
///
/// - **both sides present**: a completed call; estimate both.
/// - **prompt only**: the leg *failed*. The completion is genuinely absent, but
///   the prompt was assembled, sent, and paid for before the error. Recording a
///   zero here prices that abandoned work at nothing, which bends COGS downward
///   exactly when the system is misbehaving — a timed-out ACP turn has pushed
///   the whole 40-55K-token prefix upstream by the time it fails.
/// - **neither**: nothing is known. A fabricated count would be worse than an
///   honest zero, so the row stays zero and is not marked estimated.
#[must_use]
pub fn recorded_token_counts(
    usage: Option<&TokenUsage>,
    prompt_text: Option<&str>,
    completion_text: Option<&str>,
) -> (i64, i64, bool) {
    usage.map_or_else(
        || match (prompt_text, completion_text) {
            (Some(p), Some(c)) => {
                let (est_p, est_c) = estimate_chat_tokens(p, c);
                (i64::from(est_p), i64::from(est_c), true)
            }
            (Some(p), None) => (i64::from(estimate_prompt_tokens(p)), 0, true),
            _ => (0, 0, false),
        },
        |u| {
            (
                i64::from(u.prompt_tokens),
                i64::from(u.completion_tokens),
                false,
            )
        },
    )
}

/// Per-LLM-call metric captured by the tool loop and handed to a
/// [`LlmCallRecorder`]. One record corresponds to one invocation of the
/// provider's completion API inside the tool loop.
#[derive(Debug, Clone)]
pub struct LlmCallRecord {
    /// Provider name (e.g. `"gemini"`, `"groq"`, `"claude_code"`).
    pub provider: String,
    /// Model identifier used for this call.
    pub model: String,
    /// Prompt tokens reported by the provider, 0 if unavailable.
    pub prompt_tokens: i64,
    /// Completion tokens reported by the provider, 0 if unavailable.
    pub completion_tokens: i64,
    /// Prompt tokens served from the provider's context cache. Zero
    /// when the provider does not report cache hits.
    pub cached_tokens: i64,
    /// Prompt tokens written INTO the provider's context cache by this
    /// call. Billed at a premium by Anthropic (1.25x input), so folding
    /// these into the fresh-prompt count understates the bill. Zero when
    /// the provider does not break cache writes out.
    pub cached_write_tokens: i64,
    /// Reasoning / "thought" tokens reported apart from the completion
    /// count. Billed at the output rate. Zero when the provider does not
    /// report them separately.
    pub reasoning_tokens: i64,
    /// Wall-clock latency of the provider call (milliseconds).
    pub latency_ms: i64,
    /// Whether the provider returned a non-error response.
    pub success: bool,
    /// 1-based position of this call within the owning `turn_id`, assigned
    /// by the tool loop so the persister can preserve call order.
    pub call_sequence: Option<i64>,
    /// True when token counts were estimated from character length
    /// because the provider returned no usage (CLI runners — Claude
    /// Code, Copilot, Cursor — do this). Persisters append an
    /// `"_estimated"` suffix to `call_type` so billing can flag the row.
    pub token_counts_estimated: bool,
    /// Names of MCP tools dispatched by this LLM call's response. Empty
    /// when the LLM returned a plain-text answer with no tool calls.
    pub tools_called: Vec<String>,
}

impl LlmCallRecord {
    /// True when this record accounts for nothing at all.
    ///
    /// A record reaches the sink for a real LLM invocation, so every one of
    /// them cost something. All counts zero with [`Self::token_counts_estimated`]
    /// unset means the provider reported no usage **and** no text reached the
    /// estimator — the row lands priced at zero and, because it carries no
    /// `_estimated` suffix, reads downstream as a measured zero rather than a
    /// missing measurement.
    ///
    /// That is not hypothetical. 163 of 454 real `chat`/`messaging` rows
    /// between March and June 2026 were written this way, so a COGS query over
    /// that window reads about a third of production LLM calls as free. The
    /// two fixes that closed it — character-based estimation on the success
    /// path, then the prompt-only arm on the error paths — each landed for its
    /// own reason, and nothing asserted the invariant they jointly restored.
    /// A third provider path that reports no usage and passes no text would
    /// reintroduce it in silence.
    #[must_use]
    pub const fn is_unaccounted(&self) -> bool {
        !self.token_counts_estimated
            && self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.reasoning_tokens == 0
    }
}

/// Sink that receives one [`LlmCallRecord`] per LLM call.
///
/// Implementations persist the record (typically to `llm_usage`) so
/// the per-turn endpoint can surface one entry per call in its
/// `llm_calls` array.
///
/// Invocations happen on the async runtime but the sink method itself
/// is synchronous; implementers should spawn a task or push to a
/// channel if the work is blocking.
pub trait LlmCallRecorder: Send + Sync {
    /// Record a completed LLM call.
    fn record(&self, record: LlmCallRecord);
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

/// Hand one [`LlmCallRecord`] to the optional sink. Centralises token
/// extraction so the three tool-loop variants can share the same
/// recording contract. `cached_tokens` is zero unless the provider
/// wrapped its usage in
/// the provider's own [`pierre_core::llm::TokenUsage`] and forwarded it through
/// the caller. `call_sequence` is the 1-based turn-local position of
/// the call (1, 2, 3, ...).
/// Shared parameters for the [`emit_call_record`] / [`emit_call_record_with_text`]
/// pair. Bundles every field a recorder needs to capture a single LLM call so
/// the call sites don't carry a nine/eleven-arg positional signature.
pub(crate) struct CallRecordInputs<'a> {
    /// Optional recorder; `None` short-circuits the call (no row written).
    pub(crate) recorder: Option<&'a Arc<dyn LlmCallRecorder>>,
    /// Provider name (e.g. `"groq"`, `"gemini"`).
    pub(crate) provider: &'a str,
    /// Model identifier as reported by the provider.
    pub(crate) model: &'a str,
    /// Token-usage payload reported by the provider; `None` when the provider
    /// emits no usage and the caller will fall back to text-based estimation.
    pub(crate) usage: Option<&'a TokenUsage>,
    /// End-to-end call latency in milliseconds.
    pub(crate) latency_ms: i64,
    /// `true` when the call completed without a provider-side error.
    pub(crate) success: bool,
    /// 1-based turn-local position of the call.
    pub(crate) call_sequence: Option<i64>,
    /// Tool function names invoked during the call.
    pub(crate) tools_called: Vec<String>,
}

pub(crate) fn emit_call_record(inputs: CallRecordInputs<'_>) {
    emit_call_record_with_text(inputs, None, None);
}

/// Variant of [`emit_call_record`] that estimates token counts from
/// character-based prompt/completion text when the provider returns
/// no usage, so CLI runners (Claude Code, Copilot, Cursor, etc.) produce
/// non-zero usage rows instead of silently dropping.
pub(crate) fn emit_call_record_with_text(
    inputs: CallRecordInputs<'_>,
    prompt_text: Option<&str>,
    completion_text: Option<&str>,
) {
    let CallRecordInputs {
        recorder,
        provider,
        model,
        usage,
        latency_ms,
        success,
        call_sequence,
        tools_called,
    } = inputs;
    let Some(recorder) = recorder else {
        return;
    };
    let (prompt_tokens, completion_tokens, estimated) =
        recorded_token_counts(usage, prompt_text, completion_text);
    // Every cache and reasoning count is read off the same `usage` the prompt
    // and completion counts come from, so one call can never report two
    // figures that disagree about the same turn. A provider that reports
    // nothing leaves these `None`, which prices identically to a measured
    // zero -- the distinction survives on the wire, not in the billed cost.
    let cached_tokens = usage
        .and_then(|u| u.cached_read_tokens)
        .map_or(0, i64::from);
    let cached_write_tokens = usage
        .and_then(|u| u.cached_write_tokens)
        .map_or(0, i64::from);
    let reasoning_tokens = usage.and_then(|u| u.reasoning_tokens).map_or(0, i64::from);
    recorder.record(LlmCallRecord {
        provider: provider.to_owned(),
        model: model.to_owned(),
        prompt_tokens,
        completion_tokens,
        cached_tokens,
        cached_write_tokens,
        reasoning_tokens,
        latency_ms,
        success,
        call_sequence,
        token_counts_estimated: estimated,
        tools_called,
    });
}
