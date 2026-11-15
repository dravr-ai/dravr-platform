// ABOUTME: The incomplete-window signal: one reader for the coverage sidecar, one store per turn
// ABOUTME: The store is how the signal leaves the ACP subprocess's loopback, which returns nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A count stated from an incomplete activity window is not a fact.
//!
//! `get_activities` stamps `coverage.complete: false` into its own result
//! when the window it served is partial (carnet#1481), and
//! `get_segment_history` does the same when the segment efforts of some of
//! the window's activities were not read (carnet#1483). The `ReAct` and text
//! loops hold the tool payloads and read that stamp straight off them. The
//! Copilot-headless loop holds nothing: its tools run inside an ACP
//! subprocess whose calls reach Dravr through the tool host on a separate
//! task, so the stamp leaves it the way a reconnect offer does — through a
//! shared store keyed by the turn ([`crate::reconnect::ReconnectOffers`]).
//!
//! [`incomplete_window_in_payload`] is the one reader both shapes go through,
//! and [`IncompleteWindows`] is that store.

use pierre_core::models::coverage::IncompleteReason;
use serde_json::Value;

use crate::guardian::TurnKey;
use crate::turn_store::{Stamp, TurnStore};

/// The tools whose result carries a window's coverage under `coverage`: a
/// count stated over one that is short of the ask is re-asked.
const COUNTED_WINDOW_TOOLS: [&str; 2] = ["get_activities", "get_segment_history"];

/// Whether one tool payload is a `get_activities` or `get_segment_history`
/// window that says it is incomplete (`coverage.complete` false).
///
/// Keyed on the tool name so only the envelope that carries this contract can
/// raise it — a nested field of some other tool's output cannot.
#[must_use]
pub fn incomplete_window_in_payload(tool_name: &str, payload: &Value) -> bool {
    use serde::Deserialize;

    let Some(coverage) = payload
        .get("coverage")
        .filter(|_| COUNTED_WINDOW_TOOLS.contains(&tool_name))
    else {
        return false;
    };
    coverage.get("complete").and_then(Value::as_bool) == Some(false)
        && coverage
            .get("incomplete_reason")
            .and_then(|reason| IncompleteReason::deserialize(reason).ok())
            .is_some_and(counted_short)
}

/// Mark `turn` in `store` when `payload`, a `tool_name` result, is an
/// incomplete window: a headless reply that counts over it is then re-asked
/// like any other.
pub fn mark_if_incomplete(
    store: &IncompleteWindows,
    turn: &TurnKey,
    tool_name: &str,
    payload: Option<&Value>,
) {
    if payload.is_some_and(|payload| incomplete_window_in_payload(tool_name, payload)) {
        store.mark(turn);
    }
}

/// Whether `reason` leaves the window short of the period asked. A provider
/// that could not be read (`provider_unavailable`) does not: the reconnect
/// offer answers it, and re-asking every reply until the athlete reconnects
/// would answer it twice. Nor do segment efforts no backend the athlete holds
/// reads (`segment_detail_unavailable`), for the same reason: no re-ask
/// changes them until the athlete connects one. A daily series' reasons are
/// not a window of activities' and never reach here.
const fn counted_short(reason: IncompleteReason) -> bool {
    match reason {
        IncompleteReason::LiveReadRunning
        | IncompleteReason::ReadCapped
        | IncompleteReason::HistoryWalkRunning
        | IncompleteReason::SegmentDetailWalking
        | IncompleteReason::SegmentDetailPartial { .. }
        | IncompleteReason::SegmentDetailUnread => true,
        IncompleteReason::ProviderUnavailable
        | IncompleteReason::ShallowHistory
        | IncompleteReason::NoStoredActivities
        | IncompleteReason::SegmentDetailUnavailable => false,
    }
}

/// Process-wide store of the turns that read an incomplete window during a
/// headless turn, shared via the runtime so the tool-host surface's executor
/// writes into the same bucket the turn's loop reads.
#[derive(Debug, Default)]
pub struct IncompleteWindows {
    /// The tool loop takes a turn's mark when it returns, so an entry outlives
    /// its turn only when the loop failed. A mark the store's overflow drops
    /// costs the re-ask, not the answer.
    marks: TurnStore<()>,
}

impl IncompleteWindows {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the turn under `key` read an incomplete window. A poisoned
    /// lock is skipped: the turn then keeps the coverage note the model read,
    /// without the re-ask, never a panic.
    pub fn mark(&self, key: &TurnKey) {
        self.marks.record(key, Stamp::Latest, |_| ());
    }

    /// Consume the mark recorded under `key`: whether the turn read an
    /// incomplete window.
    #[must_use]
    pub fn take(&self, key: &TurnKey) -> bool {
        self.marks.take(key).is_some()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn only_an_incomplete_activity_window_is_read_as_one() {
        let incomplete = json!({
            "coverage": { "complete": false, "incomplete_reason": "live_read_running" }
        });
        assert!(
            incomplete_window_in_payload("get_activities", &incomplete),
            "bug: an incomplete get_activities window must be seen"
        );
        assert!(
            incomplete_window_in_payload(
                "get_activities",
                &json!({ "coverage": { "complete": false, "incomplete_reason": "history_walk_running" } })
            ),
            "bug: a window the history walk has not reached is short of the ask"
        );
        assert!(
            !incomplete_window_in_payload(
                "get_activities",
                &json!({ "coverage": { "complete": false, "incomplete_reason": "provider_unavailable" } })
            ),
            "bug: an unreachable provider is the reconnect offer's, not the count guard's"
        );
        assert!(
            !incomplete_window_in_payload("get_stats", &incomplete),
            "bug: another tool's coverage field must not raise the signal"
        );
        assert!(
            !incomplete_window_in_payload(
                "get_activities",
                &json!({ "coverage": { "complete": true } })
            ),
            "bug: a complete window is not incomplete"
        );
        assert!(
            !incomplete_window_in_payload("get_activities", &json!({})),
            "bug: a coverage-less window is not incomplete"
        );
    }

    #[test]
    fn a_partial_segments_window_is_short_of_the_ask() {
        let walking = json!({
            "coverage": { "complete": false, "incomplete_reason": "segment_detail_walking" }
        });
        assert!(
            incomplete_window_in_payload("get_segment_history", &walking),
            "bug: a segments window the detail walk has not finished is short of the ask"
        );
        assert!(
            incomplete_window_in_payload(
                "get_segment_history",
                &json!({ "coverage": {
                    "complete": false,
                    "incomplete_reason": { "segment_detail_partial": { "unreadable": 3 } }
                } })
            ),
            "bug: a segments window with unreadable activities is short of the ask"
        );
        assert!(
            !incomplete_window_in_payload(
                "get_segment_history",
                &json!({ "coverage": { "complete": false, "incomplete_reason": "segment_detail_unavailable" } })
            ),
            "bug: segments no backend reads are the connection's to answer, not the count guard's"
        );
    }

    #[test]
    fn a_mark_is_taken_once_and_only_by_its_turn() {
        let store = IncompleteWindows::new();
        let turn = TurnKey::new(None, "turn-a");
        store.mark(&turn);
        assert!(
            !store.take(&TurnKey::new(None, "turn-b")),
            "bug: another turn must not take this turn's mark"
        );
        assert!(store.take(&turn), "bug: the marking turn must take it");
        assert!(!store.take(&turn), "bug: a mark is consumed once");
    }
}
