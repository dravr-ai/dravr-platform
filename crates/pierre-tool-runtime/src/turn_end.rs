// ABOUTME: The turn-end hand-off: a call hands an effect it would repeat to one tool its chat turn runs once, after the reply
// ABOUTME: Recorded per turn at the dispatch chokepoint, which gives back the call's destructive charge; taken by the chat pipeline

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a chat turn runs once, after its reply (carnet#1702).
//!
//! A plan written in batches is saved in several calls. Were each of them to
//! put the plan on the athlete's calendar, each would spend a destructive
//! charge, and the fourth batch of one plan would meet the per-turn cap of
//! three. A call whose effect the turn's next call would only repeat hands
//! that effect over instead (`TurnEnd::hand_over`): the dispatch chokepoint
//! gives back the call's destructive charge, since the call itself destroyed
//! nothing, and records the tool that carries the effect for the turn, with
//! the seat the call ran on — when there is one: an effect whose gates
//! refused it has nothing to carry. When the turn ends, the chat pipeline
//! runs each recorded tool once, through the same chokepoint, where it spends
//! the destructive charge the effect costs.
//!
//! Only a chat turn has an end to run anything at. The chokepoint offers the
//! hand-off to a call that answers one under its real turn token; a one-shot
//! MCP or A2A call is its own turn, so `TurnEnd::here` finds none and the
//! call carries its effect itself.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::task::futures::TaskLocalFuture;

use crate::coach_seat::TurnSeat;
use crate::guardian::TurnKey;
use crate::turn_store::{Stamp, TurnStore};

tokio::task_local! {
    /// Where the in-flight tool call hands an effect to its turn's end.
    ///
    /// Scoped by the dispatch chokepoint around every tool body: a slot when
    /// the call answers a chat turn under its real turn token, read back once
    /// the body returns, and `None` otherwise, so a nested call with no turn
    /// end never hands its effect into the slot of the call around it.
    static TURN_END_HANDOFF: Option<HandoffSlot>;
}

/// A tool a chat turn runs once after its reply, and the tool whose calls
/// handed it over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnEndDispatch {
    /// Registry name of the tool the turn runs.
    pub tool: &'static str,
    /// Registry name of the tool that handed it over, which the run is
    /// reported as triggered by.
    pub trigger: &'static str,
}

/// A recorded hand-off: what to run, and the seat the handing call ran on,
/// which the run takes too so it reaches no further than the call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandedOver {
    /// What to run.
    pub dispatch: TurnEndDispatch,
    /// The seat of the call that handed it over.
    pub seat: TurnSeat,
}

/// What a tool body handed its turn's end. Either way, once the call
/// succeeds its own destructive charge comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Handoff {
    /// The turn runs nothing for it: the effect has nothing to carry, as when
    /// its gates refused it.
    NothingToCarry,
    /// The tool the turn runs once after its reply to carry the effect.
    CarriedBy(TurnEndDispatch),
}

/// The slot one tool body hands its destructive effect into, shared with the
/// chokepoint that scoped it: `Some` once the body handed it over.
#[derive(Debug, Clone, Default)]
pub(crate) struct HandoffSlot(Arc<Mutex<Option<Handoff>>>);

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
    /// What the body handed its turn's end; `None` when it handed nothing.
    pub(crate) fn handed(&self) -> Option<Handoff> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The end of the chat turn the in-flight tool call answers, which runs what
/// the call hands it once the reply is written.
#[derive(Debug, Clone)]
pub(crate) struct TurnEnd(HandoffSlot);

impl TurnEnd {
    /// The in-flight call's turn end; `None` outside a chat turn, where the
    /// call carries its effect itself.
    #[must_use]
    pub(crate) fn here() -> Option<Self> {
        TURN_END_HANDOFF
            .try_with(Clone::clone)
            .ok()
            .flatten()
            .map(Self)
    }

    /// Hand the call's destructive effect to the turn's end, as `handoff`
    /// says. Once the call succeeds its own destructive charge comes back,
    /// and the turn runs the tool a [`Handoff::CarriedBy`] names once after
    /// its reply, however many of its calls handed it over. A call that then
    /// fails hands nothing over.
    pub(crate) fn hand_over(&self, handoff: Handoff) {
        *self.0 .0.lock().unwrap_or_else(PoisonError::into_inner) = Some(handoff);
    }
}

/// Process-wide store of what each chat turn was handed, shared via the
/// runtime so a headless turn's tool host writes where the pipeline reads.
#[derive(Debug, Default)]
pub struct TurnEndDispatches {
    /// The pipeline takes and seals a turn's entry when the turn ends, so a
    /// call that settles after it (a headless tool host's) reaches the sink
    /// the turn left. An entry outlives its turn only when the turn was
    /// abandoned before its end: what it carried waits for the next call that
    /// carries it (for a push, the next save or push, and the plan's calendar
    /// reads as behind until then).
    handed: TurnStore<Vec<HandedOver>>,
}

impl TurnEndDispatches {
    /// Create an empty store.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record that a call of the turn under `key` handed `handed` over. A
    /// second hand-off of the same tool on the same seat is the same run.
    fn record(&self, key: &TurnKey, handed: HandedOver) {
        self.handed.record(key, Stamp::Latest, |held| {
            let mut held = held.unwrap_or_default();
            if !held.contains(&handed) {
                held.push(handed);
            }
            held
        });
    }

    /// Record what a call of the turn under `key`, on `seat`, handed over:
    /// a run only when a tool carries its effect.
    pub(crate) fn record_handoff(&self, key: &TurnKey, handed: Option<Handoff>, seat: TurnSeat) {
        if let Some(Handoff::CarriedBy(dispatch)) = handed {
            self.record(key, HandedOver { dispatch, seat });
        }
    }

    /// Consume what the turn under `key` was handed, in the order it was
    /// first handed over.
    #[must_use]
    pub fn take(&self, key: &TurnKey) -> Vec<HandedOver> {
        self.handed.take(key).unwrap_or_default()
    }

    /// [`Self::take`] for a turn that ends while a call of it may still be
    /// running: a hand-off recorded after it goes to `late` instead of the
    /// store, so it is run rather than left behind.
    #[must_use]
    pub fn take_and_seal(
        &self,
        key: &TurnKey,
        late: impl Fn(Vec<HandedOver>) + Send + Sync + 'static,
    ) -> Vec<HandedOver> {
        self.handed.seal(key, Arc::new(late)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUSH: TurnEndDispatch = TurnEndDispatch {
        tool: "push_training_plan",
        trigger: "save_training_plan",
    };

    fn on(seat: TurnSeat) -> HandedOver {
        HandedOver {
            dispatch: PUSH,
            seat,
        }
    }

    #[test]
    fn four_hand_offs_of_one_tool_are_one_run() {
        let store = TurnEndDispatches::new();
        let turn = TurnKey::new(None, "batched-plan");
        for _ in 0..4 {
            store.record(&turn, on(TurnSeat::Subject));
        }
        assert_eq!(
            store.take(&turn),
            vec![on(TurnSeat::Subject)],
            "bug: a batched save must reach the calendar once"
        );
        assert!(store.take(&turn).is_empty(), "bug: taken once");
    }

    #[test]
    fn a_hand_off_recorded_after_the_turn_ended_reaches_its_sink() {
        let store = TurnEndDispatches::new();
        let turn = TurnKey::new(None, "ended-turn");
        store.record(&turn, on(TurnSeat::Subject));
        let late = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&late);
        let taken = store.take_and_seal(&turn, move |handed| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(handed);
        });
        assert_eq!(taken, vec![on(TurnSeat::Subject)], "the end takes it");

        store.record(&turn, on(TurnSeat::Subject));
        assert_eq!(
            *late.lock().unwrap_or_else(PoisonError::into_inner),
            vec![on(TurnSeat::Subject)],
            "bug: a save that settled after its turn ended never reached the calendar"
        );
        assert!(store.take(&turn).is_empty(), "nothing is left behind");
    }

    #[test]
    fn a_turn_takes_only_its_own_hand_offs() {
        let store = TurnEndDispatches::new();
        store.record(&TurnKey::new(None, "turn-a"), on(TurnSeat::Subject));
        assert!(
            store.take(&TurnKey::new(None, "turn-b")).is_empty(),
            "bug: another turn must not run this turn's hand-off"
        );
    }

    #[tokio::test]
    async fn only_a_chat_turns_call_finds_a_turn_end() {
        assert!(
            TurnEnd::here().is_none(),
            "bug: a call with no turn end must carry its effect itself"
        );

        let slot = HandoffSlot::default();
        let found = with_handoff(Some(slot.clone()), async {
            let end = TurnEnd::here();
            if let Some(end) = &end {
                end.hand_over(Handoff::CarriedBy(PUSH));
            }
            end.is_some()
        })
        .await;
        assert!(found, "bug: a chat turn's call hands over");
        assert_eq!(
            slot.handed(),
            Some(Handoff::CarriedBy(PUSH)),
            "bug: the chokepoint reads it"
        );

        let refused = HandoffSlot::default();
        with_handoff(Some(refused.clone()), async {
            if let Some(end) = TurnEnd::here() {
                end.hand_over(Handoff::NothingToCarry);
            }
        })
        .await;
        assert_eq!(
            refused.handed(),
            Some(Handoff::NothingToCarry),
            "bug: an effect its gates refused still gives its charge back, with nothing to run"
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
                    end.hand_over(Handoff::CarriedBy(PUSH));
                }
                end.is_some()
            }),
        )
        .await;
        assert!(!nested_found, "bug: the nested call found the outer slot");
        assert_eq!(
            outer.handed(),
            None,
            "bug: the outer call's charge came back for a nested call's effect"
        );
    }
}
