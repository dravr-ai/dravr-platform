// ABOUTME: Hook traits for channel-specific side effects in the unified chat pipeline
// ABOUTME: ResponsePostProcess and ScenePublisher plug channel adapters into the pipeline edges
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Hook traits for channel-specific side effects.
//!
//! Per-surface behavior that involves side effects plugs into the pipeline
//! through these traits rather than appearing as conditional branches inside
//! [`super::run`]. This keeps the pipeline body linear and channel-agnostic.
//!
//! Two extension points:
//!
//! - [`ResponsePostProcess`] — transforms the final reply content before
//!   persistence. Every conversational surface installs the identity
//!   transform; the seam exists so a surface with a structured reply shape
//!   can parse it here rather than as a branch inside the pipeline body.
//! - [`ScenePublisher`] — turns a reply's chart specs into fetchable images.
//!   Wired by surfaces whose [`crate::BlockSupport::scene_raster`] is set;
//!   absent everywhere else, because a surface that draws a spec inline has
//!   nothing to publish.

use super::envelope::SceneImage;
use pierre_agui::AgUiSink;
use pierre_core::models::TenantId;
pub use pierre_services::chat_stream::{
    ProgressKind, TurnEvent, TurnEventSink, TurnProgress, STAGE_STATUS_FINISHED,
    STAGE_STATUS_STARTED,
};

/// Synchronous, pure post-processing of the assistant reply content.
///
/// The pipeline calls [`ResponsePostProcess::transform`] after guardrails
/// and claim verification but before persistence. Conversational flows
/// return the content unchanged.
pub trait ResponsePostProcess: Send + Sync {
    /// Transform the raw reply content into its persisted form.
    fn transform(&self, raw: &str) -> String;
}

/// One reply's chart specs, addressed well enough to mint a signed URL per
/// block.
pub struct ScenePublishRequest<'a> {
    /// JSON array of block specs as stored on the assistant message row.
    pub specs: &'a str,
    /// Conversation the message belongs to.
    pub conversation_id: &'a str,
    /// Author of the turn.
    pub user_id: &'a str,
    /// Tenant the conversation was written under — the same one the render
    /// route re-reads the message with, not the surface's owning tenant.
    pub tenant_id: TenantId,
    /// Assistant message row the specs are stored on.
    pub message_id: &'a str,
    /// Locale the axis labels must resolve in.
    pub locale: &'a str,
}

/// Publishes a reply's chart specs as fetchable images.
///
/// Implemented by the surfaces that cannot draw a spec inline and instead hand
/// their transport a URL to fetch. The pipeline calls it once per turn, after
/// the assistant message is durable — the specs are addressed by message id,
/// so there is nothing to publish before the row exists.
pub trait ScenePublisher: Send + Sync {
    /// Publish every spec in the request, in order. An empty result means the
    /// reply keeps the sentences the coach wrote around the chart.
    fn publish(&self, request: &ScenePublishRequest<'_>) -> Vec<SceneImage>;
}

/// AG-UI feedback wiring for a single turn.
///
/// The messaging surfaces' progress rail. A channel adapter constructs an
/// [`AgUiRun`] bound to a fresh `run_id` plus an [`AgUiSink`] (a
/// [`pierre_agui::BroadcastSink`] connected to the server-wide
/// [`pierre_agui::RunRegistry`]) and passes it through [`PipelineHooks`];
/// the pipeline emits lifecycle, step, and error events against that sink,
/// and `pierre_services::messaging_status_bridge` subscribes to the same
/// registry **in process** to drive Telegram/Slack/Discord placeholder edits.
/// Nothing subscribes over HTTP.
///
/// In-app surfaces do not use this: their progress rides the turn's own
/// stream as [`TurnEvent::Progress`], on the one body the reply arrives on.
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
    /// Optional post-processor for the assistant reply content.
    pub response_post_process: Option<&'a dyn ResponsePostProcess>,
    /// Optional AG-UI progress feedback wiring. When present, the
    /// pipeline emits lifecycle, step, and error events against
    /// `agui.sink`.
    pub agui: Option<AgUiRun<'a>>,
    /// Optional sink for the turn's live event stream. When present the
    /// pipeline reports each stage it enters and leaves, the dispatch stage
    /// calls the LLM provider's streaming variant where one exists, and every
    /// observation is forwarded as a [`TurnEvent`] so the route layer can
    /// wrap the stream in an SSE response.
    pub stream_sink: Option<TurnEventSink>,
    /// Optional chart publisher. Wired by surfaces whose
    /// [`crate::BlockSupport::scene_raster`] is set; the pipeline emits
    /// [`crate::ReplyBlock::SceneImage`] from what it returns.
    pub scene_publisher: Option<&'a dyn ScenePublisher>,
}

impl PipelineHooks<'_> {
    /// Construct an empty hook set (all no-ops). Useful for tests.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            response_post_process: None,
            agui: None,
            stream_sink: None,
            scene_publisher: None,
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
/// The default for conversational flows (messaging and in-app chat).
pub struct IdentityPostProcess;

impl ResponsePostProcess for IdentityPostProcess {
    fn transform(&self, raw: &str) -> String {
        raw.to_owned()
    }
}
