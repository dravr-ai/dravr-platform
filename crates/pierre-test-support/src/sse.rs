// ABOUTME: Reads a server-sent-events response body as the client does: one decoded frame per `event:` block
// ABOUTME: Stops at the turn's terminal `done` or `failed` frame, or when the caller's deadline elapses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The frames a client reads off a streaming chat turn.
//!
//! A turn's body is a sequence of `event: <name>` / `data: <payload>` blocks,
//! each closed by a blank line, and ends on a `done` or `failed` frame. The
//! integration suites that drive the real `send_message` handler over HTTP
//! assert on that sequence, so they decode it the same way here rather than
//! each holding its own reader.

use std::str;
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;
use tokio::time;

/// The frame names that end a turn's stream.
const TERMINAL_EVENTS: [&str; 2] = ["done", "failed"];

/// One decoded SSE frame: its `event:` name and its joined `data:` payload.
#[derive(Debug, Clone)]
pub struct Frame {
    /// The `event:` line's name (`progress`, `delta`, `retract`, `block`,
    /// `done`, `failed`).
    pub event: String,
    /// Every `data:` line of the block, joined by newlines.
    pub data: String,
}

impl Frame {
    /// The frame's payload parsed as JSON.
    ///
    /// # Errors
    ///
    /// Returns the parse error when the payload is not JSON.
    pub fn json(&self) -> serde_json::Result<Value> {
        serde_json::from_str(&self.data)
    }
}

/// Read `response`'s whole SSE body, decoding every frame up to and including
/// the terminal one, or until `deadline` elapses.
///
/// A frame with no `event:` name or no `data:` line is skipped, as a client
/// skips a keep-alive comment. The one space the SSE format allows after a
/// field's colon is not part of the value.
pub async fn read_frames(response: reqwest::Response, deadline: Duration) -> Vec<Frame> {
    let mut buffer = String::new();
    let mut frames: Vec<Frame> = Vec::new();
    let mut body = response.bytes_stream();

    // Elapsing is the caller's own bound on a turn that never ends: the
    // frames read until then are what it asserts on.
    time::timeout(deadline, async {
        while let Some(chunk) = body.next().await {
            let Ok(bytes) = chunk else { break };
            let Ok(decoded) = str::from_utf8(&bytes) else {
                continue;
            };
            buffer.push_str(decoded);

            while let Some(end) = buffer.find("\n\n") {
                let raw = buffer[..end].to_owned();
                buffer.drain(..end + 2);
                if let Some(frame) = decode(&raw) {
                    let terminal = TERMINAL_EVENTS.contains(&frame.event.as_str());
                    frames.push(frame);
                    if terminal {
                        return;
                    }
                }
            }
        }
    })
    .await
    .unwrap_or(());

    frames
}

/// One `event:`/`data:` block, or `None` when it names no event or carries
/// no data.
fn decode(raw: &str) -> Option<Frame> {
    let mut event = String::new();
    let mut data_lines: Vec<&str> = Vec::new();
    for line in raw.split('\n') {
        if let Some(rest) = line.strip_prefix("event:") {
            field_value(rest).clone_into(&mut event);
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(field_value(rest));
        }
    }
    (!event.is_empty() && !data_lines.is_empty()).then(|| Frame {
        event,
        data: data_lines.join("\n"),
    })
}

/// A field's value: what follows the colon, less the one optional space.
fn field_value(rest: &str) -> &str {
    rest.strip_prefix(' ').unwrap_or(rest)
}
