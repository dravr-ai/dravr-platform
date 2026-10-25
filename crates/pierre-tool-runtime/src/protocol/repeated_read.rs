// ABOUTME: Answers a read the model repeats within one turn with a pointer to the result it already has
// ABOUTME: Graphicle lesson 4c: the same payload is never sent twice in a turn; a write in between resets it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Repeated reads within one turn.
//!
//! A model that calls the same read twice with the same arguments in one turn
//! already has the first result in front of it; sending it again doubles what
//! the turn costs and teaches nothing. The repeat gets a short pointer back
//! instead. Only the first repeat: a third identical call runs, because a
//! model asking again after the pointer no longer has the result (its context
//! was compacted) or wants a fresh read (a progress check).
//!
//! The memory lives in the Guardian turn store, keyed like taint, so the chat
//! loops and the Copilot-headless loopback (a fresh executor per call) share
//! it. A one-shot `/mcp` or A2A call runs under a nonce turn and so never
//! repeats: the 2026-07-28 MCP revision is stateless, and a client session
//! would need tronc to expose an id the load balancer keeps affinity on.

use crate::guardian::{GuardianTurns, ReadFingerprint, TurnKey};
use crate::protocol::types::UniversalResponse;

/// The pointer for `tool_name` when `read` repeats one this turn served, or
/// `None` when the call should run.
pub(super) fn answer(
    turns: &GuardianTurns,
    key: &TurnKey,
    read: Option<ReadFingerprint>,
    tool_name: &str,
) -> Option<UniversalResponse> {
    let fingerprint = read?;
    turns
        .repeat_of_served_read(key, fingerprint)
        .then(|| pointer(tool_name))
}

/// Record what a successful call means for later repeats: a read is
/// remembered, a write forgets every read, since it may have changed them.
pub(super) fn remember(
    turns: &GuardianTurns,
    key: &TurnKey,
    read: Option<ReadFingerprint>,
    writes_data: bool,
) {
    if let Some(fingerprint) = read {
        turns.record_served_read(key, fingerprint);
    } else if writes_data {
        turns.forget_served_reads(key);
    }
}

fn pointer(tool_name: &str) -> UniversalResponse {
    UniversalResponse {
        success: true,
        result: Some(serde_json::json!({
            "repeated_call": true,
            "note": format!(
                "You already called {tool_name} with these exact arguments in this turn, \
                 and its result is above. Use that result. If you need a fresh read, for \
                 example to check progress, call it once more and it will run."
            ),
        })),
        error: None,
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key() -> TurnKey {
        TurnKey::new(None, "turn-1")
    }

    #[test]
    fn the_first_repeat_is_pointed_back_and_the_next_runs() {
        let turns = GuardianTurns::new();
        let read = Some(ReadFingerprint::of("get_activities", &json!({"limit": 5})));
        assert!(answer(&turns, &key(), read, "get_activities").is_none());
        remember(&turns, &key(), read, false);
        let pointer = answer(&turns, &key(), read, "get_activities").unwrap();
        assert_eq!(pointer.result.unwrap()["repeated_call"], true);
        assert!(answer(&turns, &key(), read, "get_activities").is_none());
    }

    #[test]
    fn argument_order_does_not_change_the_fingerprint_and_values_do() {
        let a = ReadFingerprint::of("t", &json!({"a": 1, "b": {"c": [1, 2]}}));
        let b = ReadFingerprint::of("t", &json!({"b": {"c": [1, 2]}, "a": 1}));
        assert_eq!(a, b);
        assert_ne!(
            a,
            ReadFingerprint::of("t", &json!({"a": 2, "b": {"c": [1, 2]}}))
        );
        assert_ne!(
            a,
            ReadFingerprint::of("u", &json!({"a": 1, "b": {"c": [1, 2]}}))
        );
    }

    #[test]
    fn a_write_between_two_reads_lets_the_second_run() {
        let turns = GuardianTurns::new();
        let read = Some(ReadFingerprint::of("get_profile", &json!({})));
        remember(&turns, &key(), read, false);
        remember(&turns, &key(), None, true);
        assert!(answer(&turns, &key(), read, "get_profile").is_none());
    }

    #[test]
    fn another_turn_does_not_see_this_turns_reads() {
        let turns = GuardianTurns::new();
        let read = Some(ReadFingerprint::of("get_profile", &json!({})));
        remember(&turns, &key(), read, false);
        let other = TurnKey::new(None, "turn-2");
        assert!(answer(&turns, &other, read, "get_profile").is_none());
    }
}
