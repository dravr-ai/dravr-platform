// ABOUTME: The incomplete-window signal: one reader for the coverage sidecar, one store per turn
// ABOUTME: The store is how the signal leaves the ACP subprocess's loopback, which returns nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A count stated from an incomplete activity window is not a fact.
//!
//! `get_activities` stamps `coverage.complete: false` into its own result
//! when the window it served is partial (carnet#1481). The `ReAct` and text
//! loops hold the tool payloads and read that stamp straight off them. The
//! Copilot-headless loop holds nothing: its tools run inside an ACP
//! subprocess whose calls reach Dravr through the tool host on a separate
//! task, so the stamp leaves it the way a reconnect offer does — through a
//! shared store keyed by the turn ([`crate::reconnect::ReconnectOffers`]).
//!
//! [`incomplete_window_in_payload`] is the one reader both shapes go through,
//! and [`IncompleteWindows`] is that store.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use pierre_core::models::coverage::IncompleteReason;
use serde::Deserialize;
use serde_json::Value;

use crate::guardian::state::TURN_TTL;
use crate::guardian::TurnKey;

/// Hard cap on retained unconsumed marks. The tool loop takes a turn's mark
/// when it returns, so an entry outlives its turn only when the loop failed.
/// Overflow drops the entries past [`TURN_TTL`] first, every entry only if
/// that was not enough; a dropped mark costs the re-ask, not the answer.
const MARK_CAP: usize = 10_000;

/// Whether one tool payload is a `get_activities` window that says it is
/// incomplete (`coverage.complete` false).
///
/// Keyed on the tool name so only the envelope that carries this contract can
/// raise it — a nested field of some other tool's output cannot.
#[must_use]
pub fn incomplete_window_in_payload(tool_name: &str, payload: &Value) -> bool {
    let Some(coverage) = payload
        .get("coverage")
        .filter(|_| tool_name == "get_activities")
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
/// would answer it twice. A daily series' reasons are not a window of
/// activities' and never reach here.
const fn counted_short(reason: IncompleteReason) -> bool {
    match reason {
        IncompleteReason::LiveReadRunning
        | IncompleteReason::ReadCapped
        | IncompleteReason::HistoryWalkRunning => true,
        IncompleteReason::ProviderUnavailable
        | IncompleteReason::ShallowHistory
        | IncompleteReason::NoStoredActivities => false,
    }
}

/// Process-wide store of the turns that read an incomplete window during a
/// headless turn, shared via the runtime so the tool-host surface's executor
/// writes into the same bucket the turn's loop reads.
#[derive(Debug, Default)]
pub struct IncompleteWindows {
    marks: Mutex<HashMap<TurnKey, Instant>>,
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
        self.mark_at(key, Instant::now());
    }

    /// [`Self::mark`] as of `now`.
    fn mark_at(&self, key: &TurnKey, now: Instant) {
        if let Ok(mut marks) = self.marks.lock() {
            if marks.len() >= MARK_CAP {
                marks.retain(|_, at| now.duration_since(*at) < TURN_TTL);
                if marks.len() >= MARK_CAP {
                    marks.clear();
                }
            }
            marks.insert(key.clone(), now);
        }
    }

    /// Consume the mark recorded under `key`: whether the turn read an
    /// incomplete window.
    #[must_use]
    pub fn take(&self, key: &TurnKey) -> bool {
        self.marks
            .lock()
            .is_ok_and(|mut marks| marks.remove(key).is_some())
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

    #[test]
    fn marks_stay_bounded() {
        let store = IncompleteWindows::new();
        for turn in 0..MARK_CAP {
            store.mark(&TurnKey::new(None, format!("turn-{turn}")));
        }
        let past_the_cap = TurnKey::new(None, "turn-past-the-cap");
        store.mark(&past_the_cap);
        assert!(
            store.marks.lock().expect("lock").len() <= MARK_CAP,
            "bug: the store must stay under its cap"
        );
        assert!(
            store.take(&past_the_cap),
            "bug: the newest mark survives the overflow"
        );
    }
}
