// ABOUTME: Deterministic unit tests for the Guardian decision logic (budgets, egress, the turn block).
// ABOUTME: Tests Guardian::decide() with injected policies — no env/singleton, no server harness.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guardian policy-logic tests.
//!
//! These exercise the pure, synchronous [`Guardian::decide`] against an
//! explicit [`GuardianPolicy`], so they are fast and free of the process-wide
//! env-driven singleton. They pin what the Guardian still decides: the budget
//! caps, the hard deny of an external send, and a turn that stops committing
//! behind its first block.

use pierre_tool_runtime::guardian::{
    guardian_gate, CallFate, Decision, DenyReason, GateOutcome, Guardian, GuardianMode,
    GuardianPolicy, GuardianTurns, HeadlessBlock, TurnKey, TurnState,
};
use pierre_tool_runtime::SecurityLabels;

/// An enforce-mode policy with budgets small enough that a denial takes one or
/// a few calls to reach in the pure logic.
fn enforce_policy() -> GuardianPolicy {
    GuardianPolicy {
        mode: GuardianMode::Enforce,
        max_destructive_per_turn: 1,
        max_writes_per_turn: 5,
    }
}

#[test]
fn write_within_budget_is_allowed() {
    let guardian = Guardian::new(enforce_policy());
    let turn = TurnState::default();
    // A plain WRITES_DATA tool with no prior writes — the baseline.
    assert_eq!(
        guardian.decide(SecurityLabels::empty(), true, &turn),
        Decision::Allow
    );
}

#[test]
fn budget_caps_destructive_at_the_policys_limit() {
    let guardian = Guardian::new(enforce_policy());
    let mut turn = TurnState::default();

    // First destructive action is allowed...
    assert_eq!(
        guardian.decide(SecurityLabels::IRREVERSIBLE, false, &turn),
        Decision::Allow
    );
    turn.reserve(SecurityLabels::IRREVERSIBLE, false);

    // ...the second in the same turn is denied — the blast-radius cap of one.
    assert_eq!(
        guardian.decide(SecurityLabels::IRREVERSIBLE, false, &turn),
        Decision::Deny(DenyReason::BudgetExceeded)
    );
}

#[test]
fn budget_caps_writes_per_turn() {
    let guardian = Guardian::new(enforce_policy());
    let mut turn = TurnState::default();
    for _ in 0..5 {
        turn.reserve(SecurityLabels::empty(), true);
    }
    assert_eq!(
        guardian.decide(SecurityLabels::empty(), true, &turn),
        Decision::Deny(DenyReason::BudgetExceeded)
    );
}

#[test]
fn an_external_send_is_denied_with_budget_to_spare() {
    let guardian = Guardian::new(enforce_policy());
    let turn = TurnState::default();
    // No allowlist exists: egress is closed for every tenant.
    assert_eq!(
        guardian.decide(SecurityLabels::EXTERNAL_SEND, false, &turn),
        Decision::Deny(DenyReason::EgressForbidden)
    );
}

#[test]
fn turn_state_counts_reservations_and_refunds() {
    let mut turn = TurnState::default();
    turn.reserve(SecurityLabels::empty(), false);
    turn.reserve(SecurityLabels::empty(), true);
    turn.reserve(SecurityLabels::IRREVERSIBLE, false);
    assert_eq!(turn.write_count(), 1);
    assert_eq!(turn.destructive_count(), 1);

    turn.refund(SecurityLabels::IRREVERSIBLE, true);
    assert_eq!(turn.write_count(), 0);
    assert_eq!(turn.destructive_count(), 0);

    // A refund with nothing left to give back does not wrap.
    turn.refund(SecurityLabels::IRREVERSIBLE, true);
    assert_eq!(turn.write_count(), 0);
    assert_eq!(turn.destructive_count(), 0);
}

// ---------------------------------------------------------------------------
// GuardianTurns store: atomic decide-and-reserve (E2), per-turn-key isolation
// (E1). `snapshot` was removed, so state is observed via a follow-up
// `decide_and_reserve` whose `decide` closure reads the live state.
// ---------------------------------------------------------------------------

/// Read a field of the live turn state without reserving anything.
fn peek<T>(store: &GuardianTurns, key: &TurnKey, read: impl Fn(&TurnState) -> T) -> T {
    let (value, _) = store.decide_and_reserve(
        key,
        SecurityLabels::empty(),
        false,
        |state| read(state),
        |_| CallFate::Unchanged,
    );
    value
}

#[test]
fn reserve_increments_and_is_visible_to_the_next_decide() {
    let store = GuardianTurns::new();
    let key = TurnKey::new(None, "turn-1");
    // Reserve one destructive slot (the "will execute" gate is true).
    let (_, reserved) = store.decide_and_reserve(
        &key,
        SecurityLabels::IRREVERSIBLE,
        false,
        |_| true,
        |_| CallFate::Runs,
    );
    assert!(reserved);
    assert_eq!(peek(&store, &key, TurnState::destructive_count), 1);
}

#[test]
fn no_reserve_when_will_not_execute() {
    // An enforce-mode block does not execute → nothing is reserved (E2: a blocked
    // call must not consume budget).
    let store = GuardianTurns::new();
    let key = TurnKey::new(None, "turn-1");
    let (_, reserved) = store.decide_and_reserve(
        &key,
        SecurityLabels::IRREVERSIBLE,
        false,
        |_| true,
        |_| CallFate::Blocked,
    );
    assert!(!reserved);
    assert_eq!(peek(&store, &key, TurnState::destructive_count), 0);
}

#[test]
fn refund_releases_a_reserved_slot_on_failure() {
    let store = GuardianTurns::new();
    let key = TurnKey::new(None, "turn-1");
    store.decide_and_reserve(
        &key,
        SecurityLabels::IRREVERSIBLE,
        false,
        |_| true,
        |_| CallFate::Runs,
    );
    store.refund(&key, SecurityLabels::IRREVERSIBLE, false);
    assert_eq!(peek(&store, &key, TurnState::destructive_count), 0);
}

#[test]
fn per_turn_keys_do_not_share_budget() {
    // E1: distinct turn tokens are isolated buckets — one utterance's reservation
    // never leaks into the next.
    let store = GuardianTurns::new();
    let k1 = TurnKey::new(None, "turn-1");
    let k2 = TurnKey::new(None, "turn-2");
    store.decide_and_reserve(
        &k1,
        SecurityLabels::IRREVERSIBLE,
        false,
        |_| true,
        |_| CallFate::Runs,
    );
    assert_eq!(peek(&store, &k1, TurnState::destructive_count), 1);
    assert_eq!(peek(&store, &k2, TurnState::destructive_count), 0);
}

// `guardian_gate` is the SINGLE gate every dispatch path calls — the
// chokepoint executor, which `/mcp` routes every registry tool through. These
// pin that shared gate's decisions.

#[test]
fn guardian_gate_blocks_second_destructive_in_enforce() {
    let guardian = Guardian::new(enforce_policy());
    let turns = GuardianTurns::new();
    let key = TurnKey::new(None, "turn-1");

    // First IRREVERSIBLE dispatch this turn: allowed, reserves the 1 destructive slot.
    let (first, reserved) = guardian_gate(
        &guardian,
        &turns,
        &key,
        "disconnect_provider",
        SecurityLabels::IRREVERSIBLE,
        true,
    );
    assert_eq!(first, GateOutcome::Proceed);
    assert!(reserved, "an allowed destructive call reserves budget");

    // Second in the SAME turn: per-turn destructive budget exhausted → blocked,
    // exactly as the chokepoint would block it.
    let (second, _) = guardian_gate(
        &guardian,
        &turns,
        &key,
        "disconnect_provider",
        SecurityLabels::IRREVERSIBLE,
        true,
    );
    assert_eq!(second, GateOutcome::Blocked(DenyReason::BudgetExceeded));
}

#[test]
fn guardian_gate_observe_reports_but_proceeds() {
    let mut policy = enforce_policy();
    policy.mode = GuardianMode::Observe;
    let guardian = Guardian::new(policy);
    let turns = GuardianTurns::new();
    let key = TurnKey::new(None, "turn-2");

    // Exhaust the destructive budget in observe.
    let _ = guardian_gate(
        &guardian,
        &turns,
        &key,
        "disconnect_provider",
        SecurityLabels::IRREVERSIBLE,
        true,
    );
    // A would-deny in observe still PROCEEDS (logs, runs anyway) and still
    // reserves budget for the call that will run.
    let (outcome, reserved) = guardian_gate(
        &guardian,
        &turns,
        &key,
        "disconnect_provider",
        SecurityLabels::IRREVERSIBLE,
        true,
    );
    assert_eq!(outcome, GateOutcome::Proceed);
    assert!(reserved);
}

// #10: the block the chokepoint records and the turn's tool loop consumes so one
// that fired inside an ACP agent's tool-host session surfaces as a deterministic
// refusal. Taken exactly once.
#[test]
fn turn_block_records_and_takes_once() {
    let store = GuardianTurns::new();
    let key = TurnKey::new(None, "turn");
    let denied = HeadlessBlock {
        tool_name: "disconnect_provider".to_owned(),
        reason: DenyReason::BudgetExceeded,
    };

    // Nothing recorded yet.
    assert_eq!(store.take_block(&key), None);

    // The chokepoint records a block; the tool loop takes it once.
    store.record_block(&key, denied.clone());
    assert_eq!(store.take_block(&key), Some(denied));
    // Single consumption — a second take sees nothing.
    assert_eq!(store.take_block(&key), None);
}

/// After a call in the turn is denied, nothing further commits behind the reply,
/// whichever path it arrives on. Reads carry on so the model finishes.
#[test]
fn a_block_stops_the_turn_committing_through_the_real_gate_and_store() {
    let guardian = Guardian::new(GuardianPolicy {
        max_destructive_per_turn: 0,
        ..enforce_policy()
    });
    let turns = GuardianTurns::new();
    let key = TurnKey::new(None, "latched-turn");
    let gate = |tool: &str, labels: SecurityLabels, writes: bool| {
        guardian_gate(&guardian, &turns, &key, tool, labels, writes)
    };

    // A write before the block: nothing is latched yet.
    assert_eq!(
        gate("set_goal", SecurityLabels::empty(), true),
        (GateOutcome::Proceed, true)
    );
    // The first block keeps its own reason.
    assert_eq!(
        gate("disconnect_provider", SecurityLabels::IRREVERSIBLE, true),
        (GateOutcome::Blocked(DenyReason::BudgetExceeded), false)
    );
    // Every kind of consequential call after it is refused for that reason, none
    // reserving budget, including one the budgets alone would have let through.
    for (tool, labels, writes) in [
        ("set_goal", SecurityLabels::empty(), true),
        ("disconnect_provider", SecurityLabels::IRREVERSIBLE, false),
        ("send_email", SecurityLabels::EXTERNAL_SEND, false),
    ] {
        assert_eq!(
            gate(tool, labels, writes),
            (GateOutcome::Blocked(DenyReason::TurnBlocked), false),
            "{tool}"
        );
    }
    assert_eq!(peek(&turns, &key, TurnState::write_count), 1);
    // Reads stay allowed.
    assert_eq!(
        gate("get_connection_status", SecurityLabels::empty(), false),
        (GateOutcome::Proceed, true)
    );
}

/// Observe logs the would-deny and runs the call, so it never latches: the next
/// call gets the same answer it would have had with no earlier flag.
#[test]
fn observe_never_latches_a_turn() {
    let guardian = Guardian::new(GuardianPolicy {
        mode: GuardianMode::Observe,
        max_destructive_per_turn: 0,
        ..enforce_policy()
    });
    let turns = GuardianTurns::new();
    let key = TurnKey::new(None, "unlatched-turn");
    let (flagged, _) = guardian_gate(
        &guardian,
        &turns,
        &key,
        "disconnect_provider",
        SecurityLabels::IRREVERSIBLE,
        false,
    );
    assert_eq!(flagged, GateOutcome::Proceed);
    assert!(!peek(&turns, &key, TurnState::is_blocked));
}
