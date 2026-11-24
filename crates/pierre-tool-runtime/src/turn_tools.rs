// ABOUTME: The tools an outside caller's chat turn ran, kept per turn so a turn that fails still names them
// ABOUTME: Written at each model-asked dispatch of an A2A or API-key turn, taken once when the turn ends (carnet#1543)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a turn an outside caller drove ran, whatever became of it.
//!
//! A served turn names its tools in its telemetry. A turn that ends in an
//! error carries none, yet the tools it ran before failing read the athlete's
//! data, and the turn's one usage row (the athlete's "who read my data" feed)
//! must name them. Each dispatch the model asked for in an A2A or API-key turn
//! is recorded here under the turn, on whichever task it ran (the
//! Copilot-headless loop's tools run on the tool host's own executor), and the
//! turn takes its list once when it ends.

use pierre_core::transport::Transport;

use crate::guardian::TurnKey;
use crate::turn_store::{Stamp, TurnStore};

/// Whether a turn served over `transport` records the tools it runs.
///
/// One an outside caller drove through Dravr's agent (an A2A agent, an API
/// key) does. Other turns write no outside caller's row, so they record
/// nothing that nothing would take.
#[must_use]
pub const fn records_turn_tools(transport: Option<Transport>) -> bool {
    matches!(transport, Some(Transport::A2a | Transport::ApiKey))
}

/// Process-wide store of the tools each outside caller's turn ran, shared via
/// the runtime so the tool host's executor writes where the turn reads.
#[derive(Debug, Default)]
pub struct TurnTools {
    /// Each turn's tools, in the order they ran. A turn takes its entry when
    /// it ends, so one outlives its turn only when the turn was dropped
    /// (cancelled, timed out); the store's cap bounds those.
    calls: TurnStore<Vec<String>>,
}

impl TurnTools {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the turn under `key` ran `tool_name`.
    pub fn record(&self, key: &TurnKey, tool_name: &str) {
        self.calls.record(key, Stamp::Latest, |held| {
            let mut tools = held.unwrap_or_default();
            tools.push(tool_name.to_owned());
            tools
        });
    }

    /// The tools the turn under `key` ran, in order, consumed.
    #[must_use]
    pub fn take(&self, key: &TurnKey) -> Vec<String> {
        self.calls.take(key).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turns_tools_are_kept_in_order_and_taken_once() {
        let store = TurnTools::new();
        let turn = TurnKey::new(None, "turn-1");
        let other = TurnKey::new(None, "turn-2");
        store.record(&turn, "get_connection_status");
        store.record(&other, "get_athlete");
        store.record(&turn, "get_activities");
        assert_eq!(
            store.take(&turn),
            vec![
                "get_connection_status".to_owned(),
                "get_activities".to_owned()
            ]
        );
        assert!(
            store.take(&turn).is_empty(),
            "bug: a turn's list is taken once"
        );
        assert_eq!(store.take(&other), vec!["get_athlete".to_owned()]);
    }

    #[test]
    fn only_an_outside_callers_turn_records_its_tools() {
        for transport in Transport::ALL {
            assert_eq!(
                records_turn_tools(Some(transport)),
                matches!(transport, Transport::A2a | Transport::ApiKey),
                "{transport:?}"
            );
        }
        assert!(!records_turn_tools(None));
    }
}
