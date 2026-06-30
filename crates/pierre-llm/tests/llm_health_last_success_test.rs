// ABOUTME: Tests the real-traffic liveness clock on LlmHealthState (note_success / since_last_success)
// ABOUTME: This clock lets the periodic health probe skip its billed copilot --acp round-trip
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the lock-free last-success clock on
//! [`pierre_llm::health::LlmHealthState`]. The chat pipeline stamps it after
//! every real served turn; the periodic probe reads it to decide whether it
//! can skip a synthetic (billed) round-trip.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::thread::sleep;
use std::time::Duration;

use pierre_llm::health::LlmHealthState;

#[test]
fn since_last_success_is_none_before_any_success() {
    let state = LlmHealthState::new();
    assert!(
        state.since_last_success().is_none(),
        "a fresh state has never served a real turn"
    );
}

#[test]
fn note_success_records_a_recent_timestamp() {
    let state = LlmHealthState::new();
    state.note_success();
    let since = state
        .since_last_success()
        .expect("a success was just recorded");
    assert!(
        since < Duration::from_secs(5),
        "a just-recorded success should read as very recent, got {since:?}"
    );
}

#[test]
fn since_last_success_grows_with_elapsed_time() {
    let state = LlmHealthState::new();
    state.note_success();
    let first = state.since_last_success().expect("recorded");
    sleep(Duration::from_millis(25));
    let second = state.since_last_success().expect("recorded");
    assert!(
        second >= first,
        "elapsed-since-success must be monotonic: {first:?} -> {second:?}"
    );
    assert!(
        second >= Duration::from_millis(20),
        "at least the sleep should have elapsed, got {second:?}"
    );
}

#[test]
fn note_success_is_idempotent_and_advances_the_clock_forward() {
    let state = LlmHealthState::new();
    state.note_success();
    sleep(Duration::from_millis(25));
    // A second real turn resets the clock back toward zero.
    state.note_success();
    let after_second = state.since_last_success().expect("recorded");
    assert!(
        after_second < Duration::from_millis(20),
        "the most recent success should reset the elapsed window, got {after_second:?}"
    );
}
