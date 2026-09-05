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
