// ABOUTME: Tests for the plan-then-verify static verifier — the canonical injection attack is rejected, benign plans accepted.
// ABOUTME: Pure (fake ToolLabels, no server harness), pinning the frozen-plan dataflow guarantee.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Plan verifier tests.
//!
//! The headline guarantee: a frozen plan where a consequential sink's arguments
//! derive from an `UNTRUSTED_OUTPUT` source is rejected *before any tool runs*,
//! while the common benign "read then write" plan is accepted.

use std::collections::{HashMap, HashSet};

use pierre_tool_runtime::guardian::{
    verify, ExternalSendAllowlist, GuardianMode, GuardianPolicy, PlanMode, TaintedDestructive,
    ToolLabels, VerifyError, VerifyOutcome, Workflow,
};
use pierre_tool_runtime::SecurityLabels;
use uuid::Uuid;

/// A fake tool-label source keyed by tool name. Every key is treated as
/// chat-callable; `writes` defaults to false.
struct FakeLabels {
    labels: HashMap<String, SecurityLabels>,
    writes: HashMap<String, bool>,
}

impl FakeLabels {
    fn new() -> Self {
        Self {
            labels: HashMap::new(),
            writes: HashMap::new(),
        }
    }

    fn with(mut self, tool: &str, labels: SecurityLabels, writes: bool) -> Self {
        self.labels.insert(tool.to_owned(), labels);
        self.writes.insert(tool.to_owned(), writes);
        self
    }
}

impl ToolLabels for FakeLabels {
    fn labels(&self, tool: &str) -> Option<SecurityLabels> {
        self.labels.get(tool).copied()
    }
    fn writes_data(&self, tool: &str) -> Option<bool> {
        self.writes.get(tool).copied()
    }
    fn chat_callable(&self, tool: &str) -> bool {
        self.labels.contains_key(tool)
    }
}

/// A registry mirroring the real seed: an untrusted source, a plain write, an
/// irreversible sink, and an (armed, not-yet-real) external-send sink.
fn fake_registry() -> FakeLabels {
    FakeLabels::new()
        .with("get_activities", SecurityLabels::UNTRUSTED_OUTPUT, false)
        .with("set_goal", SecurityLabels::empty(), true)
        .with("delete_coach", SecurityLabels::IRREVERSIBLE, true)
        .with("send_email", SecurityLabels::EXTERNAL_SEND, false)
}

fn policy(
    tainted_destructive: TaintedDestructive,
    egress: ExternalSendAllowlist,
) -> GuardianPolicy {
    GuardianPolicy {
        mode: GuardianMode::Enforce,
        max_destructive_per_turn: 1,
        max_writes_per_turn: 5,
        external_send: egress,
        tainted_destructive,
        plan_mode: PlanMode::Off,
    }
}

fn parse(json: &str) -> Workflow {
    Workflow::from_json(json).expect("workflow JSON should parse")
}

#[test]
fn tainted_external_send_plan_is_rejected() {
    // get_activities (untrusted) → send_email with an arg derived from step 0.
    // Egress is allowlisted-all so this exercises the taint rule, not the allowlist.
    let wf = parse(
        r#"{ "steps": [
            { "id": 0, "tool": "get_activities", "args": { "limit": 5 } },
            { "id": 1, "tool": "send_email",
              "args": { "to": "coach@x.com",
                        "body": { "$ref": { "step": 0, "path": "/activities/0/name" } } } }
        ] }"#,
    );
    let outcome = verify(
        &wf,
        &fake_registry(),
        &policy(TaintedDestructive::Deny, ExternalSendAllowlist::All),
        None,
    );
    match outcome {
        VerifyOutcome::Reject(VerifyError::TaintedSink {
            sink_step,
            source_step,
            ..
        }) => {
            assert_eq!(sink_step, 1);
            assert_eq!(source_step, 0);
        }
        other => panic!("expected TaintedSink rejection, got {other:?}"),
    }
}

#[test]
fn benign_read_then_write_plan_is_accepted() {
    // The most common coaching flow: read activities, then set a goal with
    // LITERAL args (no $ref to the untrusted output). Must be accepted.
    let wf = parse(
        r#"{ "steps": [
            { "id": 0, "tool": "get_activities", "args": { "limit": 5 } },
            { "id": 1, "tool": "set_goal", "args": { "title": "Run 50km", "target": 50 } }
        ] }"#,
    );
    let outcome = verify(
        &wf,
        &fake_registry(),
        &policy(TaintedDestructive::Deny, ExternalSendAllowlist::None),
        None,
    );
    assert_eq!(outcome, VerifyOutcome::Accept);
}

#[test]
fn tainted_irreversible_respects_policy() {
    // delete_coach with an arg derived (via JSON pointer) from the untrusted source.
    let wf = parse(
        r#"{ "steps": [
            { "id": 0, "tool": "get_activities", "args": { "limit": 5 } },
            { "id": 1, "tool": "delete_coach",
              "args": { "coach_id": { "$ref": { "step": 0, "path": "/activities/0/coach" } } } }
        ] }"#,
    );
    // Under Log, the precise dataflow rule does NOT block (graduated telemetry).
    assert_eq!(
        verify(
            &wf,
            &fake_registry(),
            &policy(TaintedDestructive::Log, ExternalSendAllowlist::None),
            None
        ),
        VerifyOutcome::Accept
    );
    // Under Deny, the tainted irreversible sink is rejected.
    assert!(matches!(
        verify(
            &wf,
            &fake_registry(),
            &policy(TaintedDestructive::Deny, ExternalSendAllowlist::None),
            None
        ),
        VerifyOutcome::Reject(VerifyError::TaintedSink { .. })
    ));
    // Under Confirm, the static verifier deliberately passes: the runtime
    // chokepoint parks the step for human confirmation at execution time,
    // keeping Confirm uniform across the ReAct and planned paths.
    assert_eq!(
        verify(
            &wf,
            &fake_registry(),
            &policy(TaintedDestructive::Confirm, ExternalSendAllowlist::None),
            None
        ),
        VerifyOutcome::Accept
    );
}

#[test]
fn external_send_egress_allowlist_respected() {
    // send_email with NO untrusted source and NO tenant on the allowlist → egress forbidden.
    let wf = parse(
        r#"{ "steps": [
            { "id": 0, "tool": "send_email", "args": { "to": "x@y.com", "body": "hi" } }
        ] }"#,
    );
    assert!(matches!(
        verify(
            &wf,
            &fake_registry(),
            &policy(TaintedDestructive::Deny, ExternalSendAllowlist::None),
            None
        ),
        VerifyOutcome::Reject(VerifyError::EgressForbidden(_))
    ));
}

#[test]
fn unknown_tool_is_rejected() {
    let wf = parse(r#"{ "steps": [ { "id": 0, "tool": "no_such_tool", "args": {} } ] }"#);
    assert!(matches!(
        verify(
            &wf,
            &fake_registry(),
            &policy(TaintedDestructive::Deny, ExternalSendAllowlist::None),
            None
        ),
        VerifyOutcome::Reject(VerifyError::UnknownTool(_))
    ));
}

#[test]
fn forward_or_dangling_reference_is_rejected() {
    // Step 0 references step 1 (a forward reference) — not strictly earlier.
    let wf = parse(
        r#"{ "steps": [
            { "id": 0, "tool": "set_goal", "args": { "x": { "$ref": { "step": 1 } } } },
            { "id": 1, "tool": "get_activities", "args": {} }
        ] }"#,
    );
    assert!(matches!(
        verify(
            &wf,
            &fake_registry(),
            &policy(TaintedDestructive::Deny, ExternalSendAllowlist::None),
            None
        ),
        VerifyOutcome::Reject(VerifyError::DanglingReference(1))
    ));
}

#[test]
fn plan_over_write_budget_is_rejected() {
    // 6 writes with a max of 5.
    let steps: Vec<String> = (0..6)
        .map(|i| format!(r#"{{ "id": {i}, "tool": "set_goal", "args": {{}} }}"#))
        .collect();
    let wf = parse(&format!(r#"{{ "steps": [ {} ] }}"#, steps.join(",")));
    assert!(matches!(
        verify(
            &wf,
            &fake_registry(),
            &policy(TaintedDestructive::Deny, ExternalSendAllowlist::None),
            None
        ),
        VerifyOutcome::Reject(VerifyError::BudgetExceeded { kind: "writes", .. })
    ));
}

#[test]
fn external_send_allowed_for_allowlisted_tenant_without_taint() {
    let tenant = Uuid::new_v4();
    let wf = parse(
        r#"{ "steps": [
            { "id": 0, "tool": "send_email", "args": { "to": "x@y.com", "body": "hi" } }
        ] }"#,
    );
    let allow = ExternalSendAllowlist::Only(HashSet::from([tenant]));
    assert_eq!(
        verify(
            &wf,
            &fake_registry(),
            &policy(TaintedDestructive::Deny, allow),
            Some(tenant)
        ),
        VerifyOutcome::Accept
    );
}

#[test]
fn resolve_args_binds_symref_value_and_json_pointer() {
    use std::collections::HashMap;

    use pierre_tool_runtime::guardian::workflow::resolve_args;
    use serde_json::json;

    // Step 1 threads step 0's output (via a JSON pointer) plus a literal. This is
    // the WorkflowExecutor's core binding step: SymRef → real value, literal as-is.
    let wf = parse(
        r#"{ "steps": [
            { "id": 0, "tool": "get_activities", "args": {} },
            { "id": 1, "tool": "set_goal",
              "args": { "name": { "$ref": { "step": 0, "path": "/best/0/name" } }, "limit": 7 } }
        ] }"#,
    );
    let mut bindings = HashMap::new();
    bindings.insert(0_u32, json!({ "best": [ { "name": "Long run" } ] }));

    let resolved = resolve_args(&wf.steps[1].args, &bindings).expect("resolution should succeed");
    assert_eq!(resolved, json!({ "name": "Long run", "limit": 7 }));
}
