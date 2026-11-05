// ABOUTME: Holds one dispatch's reserved Guardian budget until the call's outcome is known
// ABOUTME: A returned failure refunds it; an abandoned running call may have landed, so its destructive charge stays spent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The budget one dispatch reserved, until it is known what became of the call.
//!
//! The Guardian reserves budget before a tool runs, so a concurrent call cannot
//! also pass a cap. A settlement written after the `await` never runs when the
//! caller drops the future, which the tool surface's bound on one tool call does,
//! so this is held across the `await` and every exit settles it:
//!
//! - The call succeeded ([`BudgetReservation::keep`]): everything stays spent.
//! - The call returned a failure ([`BudgetReservation::refund`]): everything is
//!   refunded, since it reported that it did nothing.
//! - It was dropped before the tool started: everything is refunded, since nothing
//!   ran.
//! - It was dropped with the tool running ([`BudgetReservation::start`] was called
//!   and nothing settled it): the outcome is unknown, and a provider push may have
//!   landed. The write count is refunded, but a destructive charge stays spent, so
//!   a timed-out destructive call can never run twice in one turn on a budget of
//!   one.

use tracing::warn;

use crate::guardian::{GuardianTurns, TurnKey};
use crate::security::SecurityLabels;

/// How far the dispatch holding a reservation has got.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Reserved, and the tool has not started.
    Reserved,
    /// The tool is running.
    Running,
    /// [`BudgetReservation::keep`] or [`BudgetReservation::refund`] ran.
    Settled,
}

/// Budget reserved for one dispatch, settled by [`Self::keep`], by [`Self::refund`],
/// or when dropped.
#[must_use = "dropping the reservation settles it"]
pub(super) struct BudgetReservation<'dispatch> {
    turns: &'dispatch GuardianTurns,
    turn_key: &'dispatch TurnKey,
    tool_name: &'dispatch str,
    labels: SecurityLabels,
    writes_data: bool,
    stage: Stage,
}

impl<'dispatch> BudgetReservation<'dispatch> {
    /// Hold the budget `labels` and `writes_data` reserved under `turn_key`.
    pub(super) const fn new(
        turns: &'dispatch GuardianTurns,
        turn_key: &'dispatch TurnKey,
        tool_name: &'dispatch str,
        labels: SecurityLabels,
        writes_data: bool,
    ) -> Self {
        Self {
            turns,
            turn_key,
            tool_name,
            labels,
            writes_data,
            stage: Stage::Reserved,
        }
    }

    /// The tool is about to run: from here a drop no longer means nothing happened.
    pub(super) const fn start(&mut self) {
        self.stage = Stage::Running;
    }

    /// The call succeeded: its budget stays spent.
    pub(super) fn keep(mut self) {
        self.stage = Stage::Settled;
    }

    /// The call returned a failure: all of its budget comes back.
    pub(super) fn refund(mut self) {
        self.stage = Stage::Settled;
        self.turns
            .refund(self.turn_key, self.labels, self.writes_data);
    }
}

impl Drop for BudgetReservation<'_> {
    fn drop(&mut self) {
        if self.stage == Stage::Settled {
            return;
        }
        if self.stage == Stage::Reserved || !self.labels.is_irreversible() {
            self.turns
                .refund(self.turn_key, self.labels, self.writes_data);
            return;
        }
        self.turns
            .refund(self.turn_key, SecurityLabels::empty(), self.writes_data);
        warn!(
            tool_name = %self.tool_name,
            "guardian: call abandoned while running, outcome unknown, destructive budget stays spent"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardian::{CallFate, TurnState};

    fn reserve(turns: &GuardianTurns, key: &TurnKey) {
        let ((), reserved) = turns.decide_and_reserve(
            key,
            "tool",
            SecurityLabels::IRREVERSIBLE,
            true,
            |_| (),
            |()| CallFate::Runs,
        );
        assert!(reserved);
    }

    fn spent(turns: &GuardianTurns, key: &TurnKey) -> (u16, u16) {
        turns
            .decide_and_reserve(
                key,
                "peek",
                SecurityLabels::empty(),
                false,
                |state: &TurnState| (state.destructive_count(), state.write_count()),
                |_| CallFate::Unchanged,
            )
            .0
    }

    fn held<'a>(turns: &'a GuardianTurns, key: &'a TurnKey) -> BudgetReservation<'a> {
        BudgetReservation::new(turns, key, "tool", SecurityLabels::IRREVERSIBLE, true)
    }

    #[test]
    fn dropping_it_before_the_tool_starts_refunds_everything() {
        let turns = GuardianTurns::new();
        let key = TurnKey::new(None, "turn");
        reserve(&turns, &key);
        let reservation = held(&turns, &key);
        assert_eq!(spent(&turns, &key), (1, 1));
        drop(reservation);
        assert_eq!(spent(&turns, &key), (0, 0), "nothing ran");
    }

    #[test]
    fn dropping_it_with_the_tool_running_leaves_the_destructive_charge_spent() {
        let turns = GuardianTurns::new();
        let key = TurnKey::new(None, "turn");
        reserve(&turns, &key);
        let mut reservation = held(&turns, &key);
        reservation.start();
        drop(reservation);
        assert_eq!(
            spent(&turns, &key),
            (1, 0),
            "a push may have landed, so only the write count comes back"
        );
    }

    #[test]
    fn dropping_a_running_write_that_is_not_destructive_refunds_it() {
        let turns = GuardianTurns::new();
        let key = TurnKey::new(None, "turn");
        let ((), reserved) = turns.decide_and_reserve(
            &key,
            "tool",
            SecurityLabels::empty(),
            true,
            |_| (),
            |()| CallFate::Runs,
        );
        assert!(reserved);
        let mut reservation =
            BudgetReservation::new(&turns, &key, "tool", SecurityLabels::empty(), true);
        reservation.start();
        drop(reservation);
        assert_eq!(spent(&turns, &key), (0, 0));
    }

    #[test]
    fn a_returned_failure_refunds_both_counts() {
        let turns = GuardianTurns::new();
        let key = TurnKey::new(None, "turn");
        reserve(&turns, &key);
        let mut reservation = held(&turns, &key);
        reservation.start();
        reservation.refund();
        assert_eq!(
            spent(&turns, &key),
            (0, 0),
            "the call said it failed, so nothing is charged, once"
        );
    }

    #[test]
    fn keeping_the_reservation_leaves_the_budget_spent() {
        let turns = GuardianTurns::new();
        let key = TurnKey::new(None, "turn");
        reserve(&turns, &key);
        let mut reservation = held(&turns, &key);
        reservation.start();
        reservation.keep();
        assert_eq!(spent(&turns, &key), (1, 1));
    }
}
