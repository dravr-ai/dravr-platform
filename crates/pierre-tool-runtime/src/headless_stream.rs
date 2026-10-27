// ABOUTME: Streaming Copilot ACP turn — forwards deltas and tool-call observations to the turn sink
// ABOUTME: Accumulates the same final HeadlessToolResponse the non-streaming converse() produces
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppError;
use pierre_llm::ChatRequest;
use pierre_services::chat_stream::{TurnEvent, TurnEventSink, TurnProgress};

use crate::prose_stream::{deliver, request_canary, StreamedProse};

/// One streamed ACP turn, drained: the response the non-streaming `converse()`
/// would have produced, and whether any of its prose reached the sink.
#[derive(Debug)]
pub struct ForwardedTurn {
    /// The aggregated response, folded into `ToolLoopResult` like the
    /// non-streaming branch's.
    pub response: pierre_llm::HeadlessToolResponse,
    /// Whether prose crossed to the sink. A turn that turns out not to be the
    /// reply takes back only what is on screen: a model that wrote nothing but
    /// `<tool_call>` blocks, which never cross, left nothing to take back.
    pub prose_delivered: bool,
}

/// Run a streaming Copilot ACP turn, forwarding text deltas and tool-call
/// observations to `sink` while accumulating the same final
/// [`HeadlessToolResponse`] that the non-streaming `converse()` produces.
///
/// Prose is filtered against the turn's canary on the way out, so the one leak
/// signal that is conclusive on its own — a verbatim canary — is caught while
/// the reply is still being produced instead of only at the response boundary,
/// which sees the assembled text after the athlete has read it.
///
/// Returns the aggregated response so the caller can record per-call usage
/// and fold it into `ToolLoopResult` exactly like the non-streaming branch,
/// together with whether any prose reached `sink`.
///
/// # Errors
///
/// Returns [`AppError`] when the ACP session cannot be opened, when the
/// underlying stream yields a transport error, or when the run ends without a
/// terminal response frame.
pub async fn run_headless_streaming(
    headless_runner: &dyn pierre_llm::HeadlessTurnProvider,
    request: &ChatRequest,
    sink: &TurnEventSink,
) -> Result<ForwardedTurn, AppError> {
    let stream = headless_runner
        .converse_stream(request)
        .await
        .map_err(AppError::from)?;
    forward_headless_stream(stream, request_canary(request).unwrap_or_default(), sink).await
}

/// Drain one ACP turn's events into `sink`, filtering prose against `canary`.
///
/// Split from [`run_headless_streaming`] so the boundary that decides what
/// reaches the athlete's screen is exercised against a stream of events rather
/// than only against a live subprocess. An empty `canary` filters nothing.
///
/// # Errors
/// Propagates a runner error off the stream, and returns an external-service
/// error when the stream ends without a `Done` event.
pub async fn forward_headless_stream(
    mut stream: pierre_llm::HeadlessEventStream,
    canary: &str,
    sink: &TurnEventSink,
) -> Result<ForwardedTurn, AppError> {
    use pierre_llm::HeadlessStreamEvent;
    use tokio_stream::StreamExt;

    let mut final_response: Option<pierre_llm::HeadlessToolResponse> = None;
    let mut prose = StreamedProse::new(sink, canary);

    while let Some(item) = stream.next().await {
        let event = match item {
            Ok(event) => event,
            Err(error) => {
                // The turn re-runs on the chain's tail, whose reply streams
                // into the same bubble: what this one wrote is taken back.
                prose.retract();
                return Err(AppError::from(error));
            }
        };
        match event {
            HeadlessStreamEvent::TextDelta(delta) => prose.push(&delta),
            HeadlessStreamEvent::ToolCall(tc) => {
                deliver(
                    sink,
                    TurnEvent::Progress(TurnProgress::tool(tc.id, tc.title, tc.status)),
                );
            }
            HeadlessStreamEvent::Done(response) => {
                final_response = Some(response);
            }
        }
    }

    let Some(response) = final_response else {
        prose.retract();
        return Err(AppError::external_service(
            "copilot-headless",
            "converse_stream completed without a Done event",
        ));
    };
    // The tail the filters were holding against a split canary or tag: the
    // model has stopped, so nothing can complete a match any more and the
    // text is owed to the reader.
    prose.finish();
    Ok(ForwardedTurn {
        response,
        prose_delivered: prose.sent(),
    })
}
