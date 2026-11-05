// ABOUTME: Deterministic unit tests for the Guardian decision logic (taint/budget/egress).
// ABOUTME: Tests Guardian::decide() with injected policies — no env/singleton, no server harness.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Guardian policy-logic tests.
//!
//! These exercise the pure, synchronous [`Guardian::decide`] against an
//! explicit [`GuardianPolicy`], so they are fast and free of the process-wide
//! env-driven singleton. They pin the two guarantees that matter: the
//! false-positive-free always-on rules (budget caps, egress allowlist) hold,
//! and the taint→sink rules deny the dangerous combinations *without* breaking
//! the common "read then write" coaching flow.

use std::collections::HashSet;

use pierre_tool_runtime::guardian::{
    guardian_gate, CallFate, Decision, DenyReason, ExternalSendAllowlist, GateOutcome, Guardian,
    GuardianMode, GuardianPolicy, GuardianTurns, HeadlessBlock, PlanMode, TaintedDestructive,
    TurnKey, TurnState,
};
use pierre_tool_runtime::SecurityLabels;
use uuid::Uuid;

/// An enforce-mode policy with the production default budgets and the strictest
/// tainted-destructive severity, so denials are observable in the pure logic.
fn enforce_policy() -> GuardianPolicy {
    GuardianPolicy {
        mode: GuardianMode::Enforce,
        max_destructive_per_turn: 1,
        max_writes_per_turn: 5,
        external_send: ExternalSendAllowlist::None,
        tainted_destructive: TaintedDestructive::Deny,
        plan_mode: PlanMode::Off,
    }
}

#[test]
fn untainted_write_within_budget_is_allowed() {
    let guardian = Guardian::new(enforce_policy());
    let turn = TurnState::default();
    // A plain WRITES_DATA tool with no taint and no prior writes — the baseline.
    assert_eq!(
        guardian.decide(SecurityLabels::empty(), true, None, &turn),
        Decision::Allow
    );
}

#[test]
fn budget_caps_destructive_at_one_per_turn() {
    let guardian = Guardian::new(enforce_policy());
    let mut turn = TurnState::default();

    // First destructive action is allowed...
    assert_eq!(
        guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &turn),
        Decision::Allow
    );
    turn.record_success("delete_agent", SecurityLabels::IRREVERSIBLE, false);

    // ...the second in the same turn is denied — blast-radius cap, taint-independent.
    assert_eq!(
        guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &turn),
        Decision::Deny(DenyReason::BudgetExceeded)
    );
}

#[test]
fn budget_caps_writes_per_turn() {
    let guardian = Guardian::new(enforce_policy());
    let mut turn = TurnState::default();
    for _ in 0..5 {
        turn.record_success("set_goal", SecurityLabels::empty(), true);
    }
    assert_eq!(
        guardian.decide(SecurityLabels::empty(), true, None, &turn),
        Decision::Deny(DenyReason::BudgetExceeded)
    );
}

#[test]
fn external_send_denied_by_default_egress_allowlist() {
    let guardian = Guardian::new(enforce_policy());
    let turn = TurnState::default();
    // No tenant is allowlisted by default — egress is closed even untainted.
    assert_eq!(
        guardian.decide(SecurityLabels::EXTERNAL_SEND, false, None, &turn),
        Decision::Deny(DenyReason::EgressForbidden)
    );
}

#[test]
fn external_send_allowed_for_allowlisted_tenant_when_untainted() {
    let tenant = Uuid::new_v4();
    let mut policy = enforce_policy();
    policy.external_send = ExternalSendAllowlist::Only(HashSet::from([tenant]));
    let guardian = Guardian::new(policy);
    let turn = TurnState::default();
    assert_eq!(
        guardian.decide(SecurityLabels::EXTERNAL_SEND, false, Some(tenant), &turn),
        Decision::Allow
    );
}

#[test]
fn tainted_external_send_is_hard_denied() {
    // The headline exfiltration primitive: untrusted content steering an
    // outbound send. Denied even for an allowlisted tenant.
    let tenant = Uuid::new_v4();
    let mut policy = enforce_policy();
    policy.external_send = ExternalSendAllowlist::Only(HashSet::from([tenant]));
    let guardian = Guardian::new(policy);

    let mut turn = TurnState::default();
    turn.record_success("get_activities", SecurityLabels::UNTRUSTED_OUTPUT, false);
    assert!(turn.is_tainted());

    assert_eq!(
        guardian.decide(SecurityLabels::EXTERNAL_SEND, false, Some(tenant), &turn),
        Decision::Deny(DenyReason::TaintedSink)
    );
}

#[test]
fn tainted_irreversible_denied_under_deny_policy() {
    let guardian = Guardian::new(enforce_policy());
    let mut turn = TurnState::default();
    turn.record_success("get_activities", SecurityLabels::UNTRUSTED_OUTPUT, false);
    assert_eq!(
        guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &turn),
        Decision::Deny(DenyReason::TaintedSink)
    );
}

#[test]
fn tainted_irreversible_allowed_under_log_policy() {
    // Phase 1 default: destructive-after-untrusted is log-only (plausibly legit),
    // so decide() allows it (the caller emits the would-be-deny telemetry).
    let mut policy = enforce_policy();
    policy.tainted_destructive = TaintedDestructive::Log;
    let guardian = Guardian::new(policy);

    let mut turn = TurnState::default();
    turn.record_success("get_activities", SecurityLabels::UNTRUSTED_OUTPUT, false);
    assert_eq!(
        guardian.decide(SecurityLabels::IRREVERSIBLE, false, None, &turn),
        Decision::Allow
    );
}

#[test]
fn tainted_ordinary_write_is_allowed() {
    // "Show my activities, then set me a goal" — the single most common coaching
    // sequence. An untrusted source preceding a plain write must NOT be gated
    // (no symbolic dataflow), or we break ~every session.
    let guardian = Guardian::new(enforce_policy());
    let mut turn = TurnState::default();
    turn.record_success("get_activities", SecurityLabels::UNTRUSTED_OUTPUT, false);
    assert_eq!(
        guardian.decide(SecurityLabels::empty(), true, None, &turn),
        Decision::Allow
    );
}

#[test]
fn enforce_denies_a_tainted_outbound_write_under_deny_only() {
    // A plain tainted write stays ungated; the outbound flag is what opts in.
    let guardian = Guardian::new(enforce_policy());
    let mut turn = TurnState::default();
    turn.add_taint("get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
    assert_eq!(
        guardian.decide(SecurityLabels::OUTBOUND_WRITE, true, None, &turn),
        Decision::Deny(DenyReason::TaintedSink)
    );
    assert_eq!(
        guardian.decide(SecurityLabels::empty(), true, None, &turn),
        Decision::Allow
    );
}

#[test]
fn turn_state_accumulates_taint_and_counts() {
    let mut turn = TurnState::default();
    assert!(!turn.is_tainted());
    turn.record_success("get_activities", SecurityLabels::UNTRUSTED_OUTPUT, false);
    turn.record_success("set_goal", SecurityLabels::empty(), true);
    turn.record_success("delete_agent", SecurityLabels::IRREVERSIBLE, false);
    assert!(turn.is_tainted());
    assert_eq!(turn.write_count(), 1);
    assert_eq!(turn.destructive_count(), 1);
    assert_eq!(turn.taint_sources(), ["get_activities"]);
}

// ---------------------------------------------------------------------------
// GuardianTurns store: atomic decide-and-reserve (E2), error-path taint (S1),
// per-turn-key isolation (E1). `snapshot` was removed, so state is observed via
// a follow-up `decide_and_reserve` whose `decide` closure reads the live state.
// ---------------------------------------------------------------------------

/// Read a field of the live turn state without reserving anything.
fn peek<T>(store: &GuardianTurns, key: &TurnKey, read: impl Fn(&TurnState) -> T) -> T {
    let (value, _) = store.decide_and_reserve(
        key,
        "peek",
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
        "test_tool",
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
        "test_tool",
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
        "test_tool",
        SecurityLabels::IRREVERSIBLE,
        false,
        |_| true,
        |_| CallFate::Runs,
    );
    store.refund(&key, SecurityLabels::IRREVERSIBLE, false);
    assert_eq!(peek(&store, &key, TurnState::destructive_count), 0);
}

#[test]
fn decide_and_reserve_folds_taint_atomically_for_untrusted_sources() {
    // #5: an UNTRUSTED_OUTPUT source taints the turn at reserve time — under the
    // same lock as the decision, before its body runs — so a concurrent
    // same-turn sink deciding afterward sees the taint (no post-execution TOCTOU).
    let store = GuardianTurns::new();
    let key = TurnKey::new(None, "turn-1");
    assert!(!peek(&store, &key, TurnState::is_tainted));
    let (_, reserved) = store.decide_and_reserve(
        &key,
        "get_activities",
        SecurityLabels::UNTRUSTED_OUTPUT,
        false,
        |_| true,
        |_| CallFate::Runs,
    );
    assert!(reserved);
    assert!(
        peek(&store, &key, TurnState::is_tainted),
        "an untrusted source taints the turn the instant it is cleared to run"
    );
}

#[test]
fn decide_and_reserve_does_not_taint_a_blocked_source() {
    // A source blocked before execution (enforce deny) never runs, so it must not
    // taint — taint tracks only content that actually reached the LLM.
    let store = GuardianTurns::new();
    let key = TurnKey::new(None, "turn-1");
    let (_, reserved) = store.decide_and_reserve(
        &key,
        "get_activities",
        SecurityLabels::UNTRUSTED_OUTPUT,
        false,
        |_| true,
        |_| CallFate::Blocked,
    );
    assert!(!reserved);
    assert!(!peek(&store, &key, TurnState::is_tainted));
}

#[test]
fn record_taint_taints_regardless_of_success() {
    // S1: a failed UNTRUSTED_OUTPUT tool still surfaced content to the LLM, so
    // the executor folds taint unconditionally via record_taint.
    let store = GuardianTurns::new();
    let key = TurnKey::new(None, "turn-1");
    assert!(!peek(&store, &key, TurnState::is_tainted));
    store.record_taint(&key, "get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
    assert!(peek(&store, &key, TurnState::is_tainted));
}

#[test]
fn per_turn_keys_do_not_share_budget_or_taint() {
    // E1: distinct turn tokens are isolated buckets — one utterance's reservation
    // and taint never leak into the next.
    let store = GuardianTurns::new();
    let k1 = TurnKey::new(None, "turn-1");
    let k2 = TurnKey::new(None, "turn-2");
    store.decide_and_reserve(
        &k1,
        "test_tool",
        SecurityLabels::IRREVERSIBLE,
        false,
        |_| true,
        |_| CallFate::Runs,
    );
    store.record_taint(&k1, "get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
    assert_eq!(peek(&store, &k2, TurnState::destructive_count), 0);
    assert!(!peek(&store, &k2, TurnState::is_tainted));
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
        None,
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
        None,
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
        None,
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
        None,
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
    let denied = HeadlessBlock::Denied {
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

    // A confirm-required park travels the same channel with its claim token.
    store.record_block(
        &key,
        HeadlessBlock::ConfirmRequired {
            tool_name: "disconnect_provider".to_owned(),
            pending_id: "abc123".to_owned(),
        },
    );
    assert_eq!(
        store.take_block(&key),
        Some(HeadlessBlock::ConfirmRequired {
            tool_name: "disconnect_provider".to_owned(),
            pending_id: "abc123".to_owned(),
        })
    );
}

/// After a call in the turn is denied or parked, nothing further commits behind
/// the reply, whichever path it arrives on. Reads carry on so the model finishes.
#[test]
fn a_block_stops_the_turn_committing_through_the_real_gate_and_store() {
    let guardian = Guardian::new(GuardianPolicy {
        max_destructive_per_turn: 0,
        ..enforce_policy()
    });
    let turns = GuardianTurns::new();
    let key = TurnKey::new(None, "latched-turn");
    let gate = |tool: &str, labels: SecurityLabels, writes: bool| {
        guardian_gate(&guardian, &turns, &key, tool, labels, writes, None)
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
        ("prescribe_workout", SecurityLabels::OUTBOUND_WRITE, false),
    ] {
        assert_eq!(
            gate(tool, labels, writes),
            (GateOutcome::Blocked(DenyReason::TurnBlocked), false),
            "{tool}"
        );
    }
    assert_eq!(peek(&turns, &key, TurnState::write_count), 1);
    // Reads stay allowed, tainting ones too.
    assert_eq!(
        gate("get_connection_status", SecurityLabels::empty(), false),
        (GateOutcome::Proceed, true)
    );
    assert_eq!(
        gate("get_activities", SecurityLabels::UNTRUSTED_OUTPUT, false),
        (GateOutcome::Proceed, true)
    );
}

#[test]
fn a_parked_call_stops_the_turn_committing_too() {
    let guardian = Guardian::new(GuardianPolicy {
        tainted_destructive: TaintedDestructive::Confirm,
        ..enforce_policy()
    });
    let turns = GuardianTurns::new();
    let key = TurnKey::new(None, "parked-turn");
    turns.record_taint(&key, "get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
    let (parked, _) = guardian_gate(
        &guardian,
        &turns,
        &key,
        "disconnect_provider",
        SecurityLabels::IRREVERSIBLE,
        true,
        None,
    );
    assert_eq!(parked, GateOutcome::ConfirmRequired);
    let (next, reserved) = guardian_gate(
        &guardian,
        &turns,
        &key,
        "set_goal",
        SecurityLabels::empty(),
        true,
        None,
    );
    assert_eq!(next, GateOutcome::Blocked(DenyReason::TurnBlocked));
    assert!(!reserved);
}

/// Observe and off log the would-deny and run the call, so they never latch: the
/// next call gets the same answer it would have had with no earlier flag.
#[test]
fn observe_and_off_never_latch_a_turn() {
    for mode in [GuardianMode::Observe, GuardianMode::Off] {
        let guardian = Guardian::new(GuardianPolicy {
            mode,
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
            None,
        );
        assert_eq!(flagged, GateOutcome::Proceed, "{mode:?}");
        assert!(!peek(&turns, &key, TurnState::is_blocked), "{mode:?}");
    }
}

/// Every `GateOutcome` a caller can see must be one it explicitly handles.
///
/// Regression guard for a live-stack finding: a dispatch path that matched
/// only `Blocked` with an `if let` let the parked call fall through and
/// **execute** when `ConfirmRequired` was added — turning a refusal into an
/// allow on that transport. `decide` is the source of those outcomes, so pin
/// that a tainted destructive call under `Confirm` yields the confirm outcome
/// (never `Proceed`) and that enforce mode surfaces it as a non-`Proceed` gate
/// result every caller must branch on.
#[test]
fn confirm_policy_never_yields_proceed_for_a_tainted_destructive_call() {
    let policy = GuardianPolicy {
        tainted_destructive: TaintedDestructive::Confirm,
        ..enforce_policy()
    };
    let guardian = Guardian::new(policy);

    let mut tainted = TurnState::default();
    tainted.add_taint("get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
    assert_eq!(
        guardian.decide(SecurityLabels::IRREVERSIBLE, true, None, &tainted),
        Decision::ConfirmRequired,
        "Confirm must park a tainted destructive call, not allow it"
    );

    // Through the shared gate in enforce mode the caller sees ConfirmRequired —
    // NOT Proceed. A caller that only checks `Blocked` would execute here.
    let turns = GuardianTurns::new();
    let key = TurnKey::new(None, "confirm-outcome-turn");
    turns.record_taint(&key, "get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
    let (outcome, _reserved) = guardian_gate(
        &guardian,
        &turns,
        &key,
        "disconnect_provider",
        SecurityLabels::IRREVERSIBLE,
        true,
        None,
    );
    assert_eq!(outcome, GateOutcome::ConfirmRequired);
    assert_ne!(
        outcome,
        GateOutcome::Proceed,
        "a parked call must never look like a green light to any dispatch path"
    );
}
