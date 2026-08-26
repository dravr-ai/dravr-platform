// ABOUTME: Streaming Copilot ACP turn — forwards deltas and tool-call observations to the turn sink
// ABOUTME: Accumulates the same final HeadlessToolResponse the non-streaming converse() produces
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppError;
use pierre_llm::ChatRequest;
use pierre_services::chat_stream::{TurnEvent, TurnEventSink, TurnProgress};

/// Run a streaming Copilot ACP turn, forwarding text deltas and tool-call
/// observations to `sink` while accumulating the same final
/// [`HeadlessToolResponse`] that the non-streaming `converse()` produces.
///
/// Returns the aggregated response so the caller can record per-call usage
/// and fold it into `ToolLoopResult` exactly like the non-streaming branch.
pub async fn run_headless_streaming(
    headless_runner: &pierre_llm::CopilotHeadlessRunner,
    request: &ChatRequest,
    sink: &TurnEventSink,
) -> Result<pierre_llm::HeadlessToolResponse, AppError> {
    use pierre_llm::HeadlessStreamEvent;
    use tokio_stream::StreamExt;

    let mut stream = headless_runner
        .converse_stream(request)
        .await
        .map_err(AppError::from)?;
    let mut final_response: Option<pierre_llm::HeadlessToolResponse> = None;

    while let Some(item) = stream.next().await {
        let event = item.map_err(AppError::from)?;
        match event {
            HeadlessStreamEvent::TextDelta(delta) => {
                // Send may fail if the receiver was dropped (client disconnected
                // or the pipeline aborted) — treat as a benign no-op so the ACP
                // session keeps draining and the run still completes cleanly.
                let _ = sink.send(TurnEvent::ProseDelta(delta));
            }
            HeadlessStreamEvent::ToolCall(tc) => {
                let _ = sink.send(TurnEvent::Progress(TurnProgress::tool(
                    tc.id, tc.title, tc.status,
                )));
            }
            HeadlessStreamEvent::Done(response) => {
                final_response = Some(response);
            }
        }
    }

    final_response.ok_or_else(|| {
        AppError::external_service(
            "copilot-headless",
            "converse_stream completed without a Done event",
        )
    })
}
