// ABOUTME: Pins the still-fetching placeholder to the delivery path the caller actually has
// ABOUTME: A chat caller is told the window comes back; only a caller nothing pushes to is told to re-ask
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The `get_activities` placeholder for a historical window still being fetched.
//!
//! carnet#243: every caller used to be told to ask again, including the chat
//! athletes whose finished window is delivered back to them on its own. The
//! copy now forks on whether a follow-up is actually coming, and these pin
//! both halves of that fork — a returns-the-same-string implementation fails
//! them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_tool_runtime::implementations::data_helpers::backfill_placeholder_message;

/// A chat caller is told the answer is on its way here, and explicitly not to
/// re-ask. This is the user-visible half of carnet#243.
#[test]
fn a_followed_up_caller_is_never_told_to_ask_again() {
    let message = backfill_placeholder_message("Garmin", true, true);

    assert!(
        message.contains("comes back into this conversation"),
        "the model must be told the window is delivered here: {message}"
    );
    assert!(
        message.contains("do not ask them to repeat the question"),
        "the instruction not to re-ask is the point of the fork: {message}"
    );
    assert!(
        !message.contains("Ask me again"),
        "telling an athlete to re-ask when the answer is coming is the defect: {message}"
    );
    assert!(
        message.contains("Garmin"),
        "the provider the athlete asked about is named: {message}"
    );
}

/// A direct MCP or A2A caller has no conversation to deliver into, so re-asking
/// really is the only way that window gets served — and the copy must keep
/// saying so rather than promising a delivery that cannot happen.
#[test]
fn a_caller_with_no_follow_up_path_is_still_told_to_ask_again() {
    let message = backfill_placeholder_message("Strava", true, false);

    assert!(
        message.contains("Ask me again shortly"),
        "nothing will notify this caller, so re-asking is the real instruction: {message}"
    );
    assert!(
        !message.contains("comes back into this conversation"),
        "no conversation exists to promise a delivery into: {message}"
    );
}

/// A re-ask while the same window is already in flight never repeats the
/// ask-again line: the athlete has asked once already and hearing it twice
/// reads as no progress. It still tells a chat caller the window is coming.
#[test]
fn an_in_flight_window_does_not_repeat_the_ask_again_line() {
    let chat = backfill_placeholder_message("Garmin", false, true);
    assert!(
        !chat.contains("Ask me again"),
        "a second ask must not be answered with the same ask-again line: {chat}"
    );
    assert!(
        chat.contains("comes back into this conversation on its own when it lands"),
        "a chat caller still learns the window is on its way: {chat}"
    );
    assert!(
        chat.contains("still being fetched from an earlier request"),
        "the athlete is told the fetch is already running: {chat}"
    );

    let direct = backfill_placeholder_message("Strava", false, false);
    assert!(
        !direct.contains("Ask me again"),
        "the no-progress rule holds with or without a follow-up path: {direct}"
    );
    assert!(
        !direct.contains("comes back into this conversation"),
        "no conversation to promise: {direct}"
    );
}
