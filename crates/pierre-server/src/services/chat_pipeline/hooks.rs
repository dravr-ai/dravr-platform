// ABOUTME: Hook traits for channel-specific side effects in the unified chat pipeline
// ABOUTME: QuotaGate, UsageRecorder, ResponsePostProcess plug channel adapters into the pipeline edges
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Hook traits for channel-specific side effects.
//!
//! Per-channel behavior that involves side effects plugs into the pipeline
//! through these traits rather than appearing as conditional branches inside
//! [`super::run`]. This keeps the pipeline body linear and channel-agnostic.
//!
//! Three extension points:
//!
//! - [`QuotaGate`] — runs before LLM dispatch. Web chat uses this to enforce
//!   message/token quotas and emit HTTP 402/429; messaging uses a more
//!   permissive variant that produces a polite reply and a Slack alert on
//!   quota exhaustion.
//! - [`UsageRecorder`] — runs after the assistant reply is persisted. Records
//!   LLM cost into the usage ledger and increments quota counters. Web
//!   records synchronously; messaging records from the `dispatch_and_respond`
//!   wrapper so the pipeline can stay agnostic of channel concurrency.
//! - [`ResponsePostProcess`] — transforms the final reply content before
//!   persistence. Web's insight-generation flow uses this to parse JSON from
//!   the LLM; conversational flows use a no-op.

use async_trait::async_trait;

use super::turn::{DispatchResult, TurnContext};
use crate::agui::AgUiSink;
use crate::errors::AppResult;

/// Warning payload produced by a soft-quota hit. Channel adapters decide
/// how to surface this (HTTP header on web; in-band reply text on messaging).
#[derive(Debug, Clone)]
pub struct QuotaWarning {
    /// Short machine-readable warning code, e.g. `"approaching_message_limit"`.
    pub code: String,
    /// Human-readable message suitable for surfacing to the user.
    pub message: String,
}

/// Pre-dispatch quota enforcement hook.
///
/// Returning `Ok(None)` permits the turn without warning.
/// Returning `Ok(Some(warning))` permits the turn with a soft warning.
/// Returning `Err(...)` hard-blocks the turn (channel adapter translates to
/// HTTP 402/429 on web or polite reply on messaging).
#[async_trait]
pub trait QuotaGate: Send + Sync {
    /// Check whether the turn may proceed.
    async fn pre_check(&self, ctx: &TurnContext) -> AppResult<Option<QuotaWarning>>;
}

/// Post-dispatch usage recording hook.
///
/// Called after the assistant reply is persisted. Implementations are
/// responsible for both cost tracking (LLM ledger) and quota counter
/// increments. Errors should be logged and swallowed — a usage-recording
/// failure must never cause a successful turn to fail.
#[async_trait]
pub trait UsageRecorder: Send + Sync {
    /// Record usage for a completed turn.
    async fn record(&self, ctx: &TurnContext, result: &DispatchResult, elapsed_ms: u64);
}

/// Synchronous, pure post-processing of the assistant reply content.
///
/// The pipeline calls [`ResponsePostProcess::transform`] after guardrails
/// and claim verification but before persistence. Web chat's insight
/// generation flow uses this to parse structured JSON out of the LLM's
/// free-form reply; conversational flows return the content unchanged.
pub trait ResponsePostProcess: Send + Sync {
    /// Transform the raw reply content into its persisted form.
    fn transform(&self, raw: &str) -> String;
}

/// AG-UI feedback wiring for a single turn.
///
/// When the caller wants progress feedback, it constructs an [`AgUiRun`]
/// bound to a fresh `run_id` plus an [`AgUiSink`] (typically a
/// [`crate::agui::BroadcastSink`] connected to the server-wide
/// [`crate::agui::RunRegistry`]) and passes it through
/// [`PipelineHooks`]. The pipeline emits lifecycle, step, and error
/// events against that sink. Clients open
/// `GET /api/agui/runs/{run_id}/stream` to subscribe.
pub struct AgUiRun<'a> {
    /// Stable identifier for this run. Shared with clients so they can
    /// subscribe to the matching SSE stream.
    pub run_id: String,
    /// Optional conversation/thread id, propagated verbatim into
    /// `RUN_STARTED`'s `thread_id` field.
    pub thread_id: Option<String>,
    /// Sink the pipeline emits events through. Implementations must
    /// filter and drop events they do not want forwarded.
    pub sink: &'a dyn AgUiSink,
}

/// Bundle of hooks a channel adapter passes to [`super::run`].
///
/// Each hook is optional; when absent, the corresponding stage runs a
/// no-op default.
pub struct PipelineHooks<'a> {
    /// Optional pre-dispatch quota gate.
    pub quota_gate: Option<&'a dyn QuotaGate>,
    /// Optional post-dispatch usage recorder.
    pub usage_recorder: Option<&'a dyn UsageRecorder>,
    /// Optional post-processor for the assistant reply content.
    pub response_post_process: Option<&'a dyn ResponsePostProcess>,
    /// Optional AG-UI progress feedback wiring. When present, the
    /// pipeline emits lifecycle, step, and error events against
    /// `agui.sink`.
    pub agui: Option<AgUiRun<'a>>,
}

impl PipelineHooks<'_> {
    /// Construct an empty hook set (all no-ops). Useful for tests.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            quota_gate: None,
            usage_recorder: None,
            response_post_process: None,
            agui: None,
        }
    }
}

impl Default for PipelineHooks<'_> {
    fn default() -> Self {
        Self::none()
    }
}

/// Identity post-processor — returns the reply content unchanged.
///
/// The default for conversational flows (messaging and non-insight web chat).
pub struct IdentityPostProcess;

impl ResponsePostProcess for IdentityPostProcess {
    fn transform(&self, raw: &str) -> String {
        raw.to_owned()
    }
}
