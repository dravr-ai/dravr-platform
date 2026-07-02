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
    Decision, DenyReason, ExternalSendAllowlist, Guardian, GuardianMode, GuardianPolicy,
    GuardianTurns, PlanMode, TaintedDestructive, TurnKey, TurnState,
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
    turn.record_success("delete_coach", SecurityLabels::IRREVERSIBLE, false);

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
fn security_labels_describe_is_stable() {
    assert_eq!(SecurityLabels::empty().describe(), "none");
    assert_eq!(
        SecurityLabels::UNTRUSTED_OUTPUT.describe(),
        "untrusted_output"
    );
    assert_eq!(SecurityLabels::EXTERNAL_SEND.describe(), "external_send");
    assert_eq!(SecurityLabels::IRREVERSIBLE.describe(), "irreversible");
}

#[test]
fn turn_state_accumulates_taint_and_counts() {
    let mut turn = TurnState::default();
    assert!(!turn.is_tainted());
    turn.record_success("get_activities", SecurityLabels::UNTRUSTED_OUTPUT, false);
    turn.record_success("set_goal", SecurityLabels::empty(), true);
    turn.record_success("delete_coach", SecurityLabels::IRREVERSIBLE, false);
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
        |_| false, // never reserve
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
        |_| true,
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
        |_| false,
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
        |_| true,
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
        |_| true, // will execute
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
        |_| false, // blocked → will NOT execute
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
        |_| true,
    );
    store.record_taint(&k1, "get_activities", SecurityLabels::UNTRUSTED_OUTPUT);
    assert_eq!(peek(&store, &k2, TurnState::destructive_count), 0);
    assert!(!peek(&store, &k2, TurnState::is_tainted));
}
