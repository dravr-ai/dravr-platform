// ABOUTME: The fortnight rail's turn budget — the only thing that makes a flow which asks nothing end
// ABOUTME: An interview ends when it runs out of topics; a rail that asks none would otherwise own the conversation for good

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_chat_pipeline::stages::onboarding::FORTNIGHT_FLOW_TURNS;
use pierre_core::models::{GuidedFlow, OnboardingState};

fn opened() -> OnboardingState {
    OnboardingState::start(Utc::now().to_rfc3339(), GuidedFlow::Fortnight)
}

#[test]
fn a_freshly_opened_rail_has_spent_nothing() {
    assert_eq!(opened().turns_owned, 0);
}

#[test]
fn the_budget_covers_the_draft_and_the_negotiation() {
    // The drafting turn plus two to argue with it. One would be the one-turn
    // shape this replaced; unbounded would be a rail that answers an athlete
    // who has moved on to something else entirely.
    assert_eq!(
        FORTNIGHT_FLOW_TURNS, 3,
        "changing this changes how long the rail holds the conversation — say \
         why in the same commit"
    );
}

#[test]
fn every_owned_turn_spends_exactly_one() {
    let mut state = opened();
    for expected in 1..=FORTNIGHT_FLOW_TURNS {
        state = state.with_owned_turn();
        assert_eq!(state.turns_owned, expected);
    }
    assert!(
        state.turns_owned >= FORTNIGHT_FLOW_TURNS,
        "the budget is spent, and the next turn hands the conversation back"
    );
}

#[test]
fn a_rail_that_somehow_outlives_its_budget_still_stops() {
    // `turns_owned` saturates rather than wrapping. A u8 that wrapped would
    // take a long-lived rail from 255 back to 0 and re-open it — the one way
    // a bounded flow becomes unbounded again.
    let mut state = opened();
    state.turns_owned = u8::MAX;
    assert_eq!(
        state.with_owned_turn().turns_owned,
        u8::MAX,
        "saturating, never wrapping"
    );
}

#[test]
fn the_budget_is_the_rails_own_and_never_the_probe_ledger() {
    // The interviews bound themselves with `probed`, whose entries are topic
    // slugs. Spending a fortnight turn must not write one: `answered_target`
    // parses that ledger against the three topic vocabularies, and a slug it
    // cannot parse stamps the athlete's next fact with the wrong provenance.
    let state = opened().with_owned_turn().with_owned_turn();
    assert_eq!(state.turns_owned, 2);
    assert!(
        state.probed.is_empty(),
        "a rail that asked nothing recorded no question: {:?}",
        state.probed
    );
}
