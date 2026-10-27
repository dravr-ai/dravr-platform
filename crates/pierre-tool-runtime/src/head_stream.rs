// ABOUTME: One API tool-loop call on the turn's model, streamed: its prose forwarded to the turn sink as it arrives
// ABOUTME: Takes the prose back when the response calls a tool, is refused, or the stream breaks before it closes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A head call the athlete watches being written.
//!
//! On a head that streams a tool-calling turn (the Anthropic Messages API, see
//! [`ChatProvider::streams_tool_turn_text`]), the call on the turn's own model
//! opens embacle's tool stream instead of waiting for the whole response. Its
//! text deltas cross [`StreamedProse`] to the turn's sink — the same gate the
//! Copilot ACP turn streams through — and its closing `Done` is the very
//! response the non-streaming call returns, so the loop, the billing and the
//! logs read it exactly as before.
//!
//! What streamed is only the reply if the response turns out to be one. It is
//! taken back with a retraction when it is not:
//!
//! - **The response calls a tool.** Words before a tool call are a tool
//!   round's text, never the reply: the reply is the closing iteration's
//!   content alone, so they would otherwise sit in the bubble until the next
//!   iteration's deltas were glued onto them.
//! - **The closing content is not what streamed.** A refusal closes with its
//!   partial output discarded (`finish_reason: "refusal"`, empty content), and
//!   the loop then serves the turn as it does any refused completion.
//! - **The stream fails before it closes.** The turn re-runs on the chain's
//!   tail, whose reply would otherwise land after the head's fragment.
//!
//! [`ChatProvider::streams_tool_turn_text`]: pierre_llm::ChatProvider::streams_tool_turn_text

use pierre_core::errors::AppError;
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponseWithTools, Tool, ToolStreamEvent, ToolTurnStream,
};
use pierre_services::chat_stream::TurnEventSink;
use tokio_stream::StreamExt;

use crate::prose_stream::{request_canary, StreamedProse};
use crate::tool_iteration::requested_tools;

/// Ask the chain's head for one tool-calling response, streaming its prose to
/// `sink` (see the module docs).
///
/// # Errors
///
/// The head's error when the stream cannot be opened, the stream's own when
/// it fails, and an external-service error when it ends without closing.
pub(crate) async fn stream_head_call(
    provider: &ChatProvider,
    request: &ChatRequest,
    tools: Option<Vec<Tool>>,
    sink: &TurnEventSink,
) -> Result<ChatResponseWithTools, AppError> {
    let stream = provider
        .complete_stream_with_tools_on_head(request, tools)
        .await?;
    drain(
        stream,
        request_canary(request).unwrap_or_default(),
        sink,
        provider.name(),
    )
    .await
}

/// Forward `stream`'s prose to `sink` and return its closing response.
async fn drain(
    mut stream: ToolTurnStream,
    canary: &str,
    sink: &TurnEventSink,
    provider: &'static str,
) -> Result<ChatResponseWithTools, AppError> {
    let mut prose = StreamedProse::new(sink, canary);
    // What the deltas spelled, unfiltered, to compare with the closing content.
    let mut streamed = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(ToolStreamEvent::TextDelta(delta)) => {
                streamed.push_str(&delta);
                prose.push(&delta);
            }
            Ok(ToolStreamEvent::Done(response)) => {
                let response = ChatResponseWithTools::from_chat_response(response);
                let is_reply = requested_tools(&response).is_empty()
                    && response.content.as_deref().unwrap_or_default() == streamed;
                if is_reply {
                    prose.finish();
                } else {
                    prose.retract();
                }
                return Ok(response);
            }
            Err(error) => {
                prose.retract();
                return Err(error);
            }
            // A kind of event a later embacle streams (a tool call starting, a
            // thinking delta) is not reply text.
            Ok(_) => {}
        }
    }
    prose.retract();
    Err(AppError::external_service(
        provider,
        "the tool-calling stream ended without its closing response",
    ))
}
