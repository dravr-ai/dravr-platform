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
