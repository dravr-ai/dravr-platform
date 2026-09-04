// ABOUTME: A canary emitted mid-stream must never cross the turn's event sink
// ABOUTME: Pins the split-across-deltas case, which a per-delta scan cannot see

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::uninlined_format_args)]

//! The response boundary is too late for a streamed reply.
//!
//! The canary scan, the identity withhold and the artifact strip all run on the
//! assembled reply, after the tool loop returns. On the deployed provider
//! (`copilot_headless` with MCP tool calling) the reply is streamed, and the web
//! client appends each `delta` frame to visible state as it arrives — so a
//! withheld reply can only replace text the athlete already read.
//!
//! A canary is the one signal that is conclusive on its own and exact to match,
//! so the stream itself enforces it. The case that matters is the split one: the
//! model emits the token a few characters at a time, and a scan that only looked
//! at each delta would pass every fragment through and match nothing.

use embacle::types::RunnerError;
use pierre_core::prompt_fingerprint::{generate_canary, inject_canary_marker};
use pierre_llm::{
    ChatMessage, ChatRequest, HeadlessEventStream, HeadlessStreamEvent, HeadlessToolResponse,
};
use pierre_services::chat_stream::{TurnEvent, TurnEventSink};
use pierre_tool_runtime::headless_stream::{forward_headless_stream, request_canary};
use tokio::sync::mpsc;

fn done(content: &str) -> HeadlessStreamEvent {
    HeadlessStreamEvent::Done(HeadlessToolResponse {
        content: content.to_owned(),
        model: "gpt-5".to_owned(),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: Some("stop".to_owned()),
    })
}

fn stream_of(events: Vec<HeadlessStreamEvent>) -> HeadlessEventStream {
    let items: Vec<Result<HeadlessStreamEvent, RunnerError>> = events.into_iter().map(Ok).collect();
    Box::pin(tokio_stream::iter(items))
}

/// Everything the sink actually received as prose, in order.
fn drain_prose(rx: &mut mpsc::UnboundedReceiver<TurnEvent>) -> String {
    let mut seen = String::new();
    while let Ok(event) = rx.try_recv() {
        if let TurnEvent::ProseDelta(delta) = event {
            seen.push_str(&delta);
        }
    }
    seen
}

/// THE ONE THAT MATTERS: the canary arrives split across three deltas, and no
/// fragment of it reaches the sink.
#[tokio::test]
async fn a_canary_split_across_deltas_never_reaches_the_sink() {
    let canary = generate_canary("tenant-coach-salt");
    let (head, tail) = canary.split_at(11);

    let leaked = format!("Tu as couru 42 km cette semaine. {canary} et voila.");
    let events = vec![
        HeadlessStreamEvent::TextDelta("Tu as couru 42 km cette semaine. ".to_owned()),
        HeadlessStreamEvent::TextDelta(head.to_owned()),
        HeadlessStreamEvent::TextDelta(tail.to_owned()),
        HeadlessStreamEvent::TextDelta(" et voila.".to_owned()),
        done(&leaked),
    ];

    let (tx, mut rx): (TurnEventSink, _) = mpsc::unbounded_channel();
    let response = forward_headless_stream(stream_of(events), &canary, &tx)
        .await
        .expect("the stream still completes");

    let seen = drain_prose(&mut rx);
    assert!(
        !seen.contains(&canary),
        "the canary reached the athlete's screen: {seen:?}"
    );
    assert!(
        !seen.contains("CANARY-"),
        "not even the token's prefix may cross the sink: {seen:?}"
    );
    assert!(
        seen.starts_with("Tu as couru"),
        "prose before the leak must still stream; got {seen:?}"
    );
    assert!(
        !seen.contains("et voila"),
        "prose after the leak must stop, not resume: {seen:?}"
    );

    // The assembled reply is untouched: the response boundary still sees the
    // leak and withholds the whole turn, which is what the terminal frame
    // carries.
    assert!(
        response.content.contains(&canary),
        "the aggregated response must keep the evidence the boundary scans"
    );
}

/// The hold-back must not eat text. A clean turn delivers every character, in
/// order — including the tail the filter was holding when the model stopped.
#[tokio::test]
async fn a_clean_stream_is_delivered_whole_and_in_order() {
    let canary = generate_canary("tenant-coach-salt");
    let parts = [
        "Trois seances cette semaine. ",
        "Garde la sortie longue dimanche, ",
        "et repose-toi lundi.",
    ];
    let whole: String = parts.concat();

    let mut events: Vec<HeadlessStreamEvent> = parts
        .iter()
        .map(|p| HeadlessStreamEvent::TextDelta((*p).to_owned()))
        .collect();
    events.push(done(&whole));

    let (tx, mut rx): (TurnEventSink, _) = mpsc::unbounded_channel();
    forward_headless_stream(stream_of(events), &canary, &tx)
        .await
        .expect("the stream completes");

    assert_eq!(
        drain_prose(&mut rx),
        whole,
        "a turn with no leak must reach the sink byte for byte"
    );
}

/// A turn whose prompt carries no marker streams unfiltered, and one that does
/// yields the token the filter is armed with.
#[tokio::test]
async fn the_turn_canary_is_recovered_from_the_prompt_it_hardened() {
    let canary = generate_canary("tenant-coach-salt");
    let hardened = inject_canary_marker("Tu es Coach Alex.", &canary);

    let armed = ChatRequest::new(vec![
        ChatMessage::system(hardened),
        ChatMessage::user("Comment va ma semaine ?"),
    ]);
    assert_eq!(
        request_canary(&armed),
        Some(canary.as_str()),
        "the filter must arm itself with the same token the prompt was hardened with"
    );

    let bare = ChatRequest::new(vec![ChatMessage::system("Tu es Coach Alex.")]);
    assert_eq!(
        request_canary(&bare),
        None,
        "an unhardened prompt arms nothing rather than an empty token"
    );
}
