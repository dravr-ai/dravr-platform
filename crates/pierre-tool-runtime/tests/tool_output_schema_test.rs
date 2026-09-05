// ABOUTME: Asserts a tool's declared outputSchema actually validates the payload it returns
// ABOUTME: The schema is a promise to conforming MCP clients; this is what keeps it true
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! MCP requires a tool declaring `outputSchema` to answer with conforming
//! `structuredContent`. That makes the schema a promise about the payload, and
//! a promise nothing checks is one that quietly stops being true.
//!
//! Both halves come from the same Rust type — `answers_with::<T>` derives the
//! schema, `execute` serializes a `T` — so drift needs a deliberate effort.
//! These pin that the derivation is real: the declared schema is present, it
//! describes the fields the payload carries, and it rejects a payload missing
//! one.

use dravr_tronc::mcp::tool::McpTool;
use pierre_tool_runtime::implementations::playbooks::{
    ForgetPlaybookResult, ForgetPlaybookTool, InterventionEntry, ListCoachingPlaybooksResult,
    ListCoachingPlaybooksTool, PlaybookEntry, TriggerEntry,
};
use pierre_tool_runtime::implementations::verification::{VerifyClaimResult, VerifyClaimTool};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;

/// The schema a conforming client would validate `verify_claim` against.
fn declared_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VerifyClaimResult))
        .expect("the derived schema serializes")
}

fn sample() -> VerifyClaimResult {
    VerifyClaimResult {
        verdict_id: "verdict-1".to_owned(),
        status: "supported".to_owned(),
        evidence_strength: "strong".to_owned(),
        layer_fired: "corpus".to_owned(),
        confidence: 0.82,
        explanation: "Two RCTs support the stated range.".to_owned(),
        evidence_refs: Some("doi:10.1000/example".to_owned()),
    }
}

#[test]
fn the_declared_schema_validates_the_payload_the_tool_returns() {
    let schema = declared_schema();
    let validator = jsonschema::validator_for(&schema).expect("the derived schema compiles");
    let payload = serde_json::to_value(sample()).expect("the payload serializes");

    assert!(
        validator.is_valid(&payload),
        "the tool's own payload must satisfy the schema it declares; schema:\n{schema:#}\npayload:\n{payload:#}"
    );
}

#[test]
fn the_schema_rejects_a_payload_missing_a_required_field() {
    // Without this the first test would pass against an empty schema, which is
    // exactly the failure mode a hand-written schema decays into.
    let validator = jsonschema::validator_for(&declared_schema()).expect("compiles");
    let missing_verdict_id = json!({
        "status": "supported",
        "evidence_strength": "strong",
        "layer_fired": "corpus",
        "confidence": 0.82,
        "explanation": "Two RCTs support the stated range.",
        "evidence_refs": null,
    });

    assert!(
        !validator.is_valid(&missing_verdict_id),
        "a schema that accepts a payload with no verdict_id is not describing anything"
    );
}

#[test]
fn an_absent_citation_is_still_valid() {
    // evidence_refs is Option: the layers below the corpus cite nothing, and a
    // schema that forbade that would reject the majority of real verdicts.
    let validator = jsonschema::validator_for(&declared_schema()).expect("compiles");
    let no_citations = VerifyClaimResult {
        evidence_refs: None,
        ..sample()
    };
    let payload = serde_json::to_value(no_citations).expect("serializes");

    assert!(
        validator.is_valid(&payload),
        "a verdict with no citations must still conform: {payload:#}"
    );
}

/// The loop the other tests leave open: they check the schema DERIVED from the
/// type, not the one the tool actually hands a client. A broken `answers_with`
/// — or a call site that forgot it — would leave every other assertion here
/// green while the tool declared nothing at all.
#[test]
fn the_tool_declares_the_schema_and_it_is_the_derived_one() {
    let declared = <VerifyClaimTool as McpTool<dyn ToolRuntime>>::definition(&VerifyClaimTool)
        .output_schema
        .expect("verify_claim must declare an outputSchema");

    assert_eq!(
        declared,
        declared_schema(),
        "the declared schema must be the one derived from VerifyClaimResult, not a hand-written copy"
    );

    let validator = jsonschema::validator_for(&declared).expect("the declared schema compiles");
    let payload = serde_json::to_value(sample()).expect("the payload serializes");
    assert!(
        validator.is_valid(&payload),
        "the schema the client receives must accept the payload the tool sends"
    );
}

/// The three things that must hold for any tool declaring a schema, in one
/// place so adding a tool costs one call rather than four copied tests.
///
/// Takes the DECLARED schema off the tool rather than deriving it here — that
/// is what catches a call site missing its `answers_with`.
fn assert_declares_and_accepts<T: serde::Serialize>(
    declared: Option<serde_json::Value>,
    derived: serde_json::Value,
    sample: &T,
    tool: &str,
) {
    let declared = declared.unwrap_or_else(|| panic!("{tool} must declare an outputSchema"));
    assert_eq!(
        declared, derived,
        "{tool}'s declared schema must be the one derived from its result type"
    );
    let validator =
        jsonschema::validator_for(&declared).unwrap_or_else(|e| panic!("{tool} schema: {e}"));
    let payload = serde_json::to_value(sample).expect("sample serializes");
    assert!(
        validator.is_valid(&payload),
        "{tool}: the declared schema rejected the payload the tool sends:\n{payload:#}"
    );
}

#[test]
fn list_coaching_playbooks_declares_a_schema_that_accepts_its_payload() {
    let sample = ListCoachingPlaybooksResult {
        playbooks: vec![PlaybookEntry {
            id: "pb-1".to_owned(),
            trigger: TriggerEntry {
                kind: "load_spike".to_owned(),
                sport: Some("run".to_owned()),
                magnitude: "high".to_owned(),
            },
            intervention: InterventionEntry {
                kind: "reduce_volume".to_owned(),
                magnitude: Some(20),
            },
            success_count: 18,
            failure_count: 2,
            neutral_count: 1,
            confidence: 0.71,
            last_outcome_at: Some("2026-09-01T10:00:00+00:00".to_owned()),
        }],
        count: 1,
    };

    assert_declares_and_accepts(
        <ListCoachingPlaybooksTool as McpTool<dyn ToolRuntime>>::definition(
            &ListCoachingPlaybooksTool,
        )
        .output_schema,
        serde_json::to_value(schemars::schema_for!(ListCoachingPlaybooksResult)).expect("derives"),
        &sample,
        "list_coaching_playbooks",
    );
}

#[test]
fn an_athlete_with_no_playbooks_still_conforms() {
    // The common first-run case, and the one an over-strict schema breaks.
    let empty = ListCoachingPlaybooksResult {
        playbooks: vec![],
        count: 0,
    };
    assert_declares_and_accepts(
        <ListCoachingPlaybooksTool as McpTool<dyn ToolRuntime>>::definition(
            &ListCoachingPlaybooksTool,
        )
        .output_schema,
        serde_json::to_value(schemars::schema_for!(ListCoachingPlaybooksResult)).expect("derives"),
        &empty,
        "list_coaching_playbooks (empty)",
    );
}

#[test]
fn forget_playbook_declares_a_schema_that_accepts_its_payload() {
    // `deleted` is a row COUNT, not a flag — 0 means "not yours or not there",
    // deliberately indistinguishable. The schema has to say number, not boolean.
    let sample = ForgetPlaybookResult {
        deleted: 0,
        playbook_id: "pb-missing".to_owned(),
    };
    assert_declares_and_accepts(
        <ForgetPlaybookTool as McpTool<dyn ToolRuntime>>::definition(&ForgetPlaybookTool)
            .output_schema,
        serde_json::to_value(schemars::schema_for!(ForgetPlaybookResult)).expect("derives"),
        &sample,
        "forget_playbook",
    );
}

#[test]
fn forget_playbook_schema_rejects_a_boolean_deleted() {
    // The wart typing exposed: the field reads like a flag and is a count. A
    // client that assumed boolean would have been wrong, and the schema now
    // says so out loud.
    let validator = jsonschema::validator_for(
        &serde_json::to_value(schemars::schema_for!(ForgetPlaybookResult)).expect("derives"),
    )
    .expect("compiles");

    assert!(
        !validator.is_valid(&json!({"deleted": true, "playbook_id": "pb-1"})),
        "deleted is a count; a schema that accepts `true` is not describing it"
    );
}
