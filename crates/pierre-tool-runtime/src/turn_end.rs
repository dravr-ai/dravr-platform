// ABOUTME: The turn-end hand-off: a chat-turn call that wrote nothing destructive gives its destructive charge back to its turn
// ABOUTME: Scoped at the dispatch chokepoint around each tool body, which reads it once the body returns; no tool runs at the turn's end

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a chat turn's call hands its turn (carnet#1702, carnet#1841).
//!
//! `save_training_plan` is declared destructive, so each call spends one of
//! the turn's destructive charges, and a plan written in batches would meet
//! the per-turn cap of three at its fourth batch. A save that writes nothing
//! destructive — a coach's draft, the athlete's own overlay, the athlete's
//! own live plan, none of which reaches a provider calendar — hands its
//! effect over instead ([`TurnEnd::hand_over`]): once the call succeeds, the
//! dispatch chokepoint gives its destructive charge back.
//!
//! Only a chat turn has a turn to give the charge back to. The chokepoint
//! offers the hand-off to a call that answers one under its real turn token;
//! a one-shot MCP or A2A call is its own turn, so [`TurnEnd::here`] finds
//! none and the call keeps the charge it spent.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::task::futures::TaskLocalFuture;

tokio::task_local! {
    /// Where the in-flight tool call hands its effect to its turn.
    ///
    /// Scoped by the dispatch chokepoint around every tool body: a slot when
    /// the call answers a chat turn under its real turn token, read back once
    /// the body returns, and `None` otherwise, so a nested call with no turn
    /// never hands its effect into the slot of the call around it.
    static TURN_END_HANDOFF: Option<HandoffSlot>;
}

/// The slot one tool body hands its effect into, shared with the chokepoint
/// that scoped it: set once the body handed it over.
#[derive(Debug, Clone, Default)]
pub(crate) struct HandoffSlot(Arc<AtomicBool>);

/// Run a tool body with `slot` as its turn end: `None` for a call that has
/// none, which also hides the slot of any call around it.
///
/// The scope itself, not an `async fn` around it: the dispatch chain polls
/// every layer on one stack, and a deep tool in a debug build has no frame to
/// spare on a test thread's stack (carnet#1702).
pub(crate) fn with_handoff<F: Future>(
    slot: Option<HandoffSlot>,
    body: F,
) -> TaskLocalFuture<Option<HandoffSlot>, F> {
    TURN_END_HANDOFF.scope(slot, body)
}

impl HandoffSlot {
    /// Whether the body handed its effect over.
    pub(crate) fn handed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// The chat turn the in-flight tool call answers, which takes back the
/// destructive charge of a call that wrote nothing destructive.
#[derive(Debug, Clone)]
pub(crate) struct TurnEnd(HandoffSlot);

impl TurnEnd {
    /// The in-flight call's turn; `None` outside a chat turn, where the call
    /// keeps the charge it spent.
    #[must_use]
    pub(crate) fn here() -> Option<Self> {
        TURN_END_HANDOFF
            .try_with(Clone::clone)
            .ok()
            .flatten()
            .map(Self)
    }

    /// Hand the call's effect over: it reached no provider calendar, so once
    /// the call succeeds its destructive charge comes back. A call that then
    /// fails is refunded whole anyway.
    pub(crate) fn hand_over(&self) {
        self.0 .0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_a_chat_turns_call_finds_a_turn_end() {
        assert!(
            TurnEnd::here().is_none(),
            "bug: a call with no turn keeps its charge"
        );

        let slot = HandoffSlot::default();
        let found = with_handoff(Some(slot.clone()), async {
            let end = TurnEnd::here();
            if let Some(end) = &end {
                end.hand_over();
            }
            end.is_some()
        })
        .await;
        assert!(found, "bug: a chat turn's call hands over");
        assert!(slot.handed(), "bug: the chokepoint reads it");

        let untouched = HandoffSlot::default();
        with_handoff(Some(untouched.clone()), async {}).await;
        assert!(
            !untouched.handed(),
            "bug: a call that handed nothing over gives no charge back"
        );
    }

    #[tokio::test]
    async fn a_nested_call_with_no_turn_end_never_hands_into_the_outer_call() {
        let outer = HandoffSlot::default();
        let nested_found = with_handoff(
            Some(outer.clone()),
            with_handoff(None, async {
                let end = TurnEnd::here();
                if let Some(end) = &end {
                    end.hand_over();
                }
                end.is_some()
            }),
        )
        .await;
        assert!(!nested_found, "bug: the nested call found the outer slot");
        assert!(
            !outer.handed(),
            "bug: the outer call's charge came back for a nested call's effect"
        );
    }
}
