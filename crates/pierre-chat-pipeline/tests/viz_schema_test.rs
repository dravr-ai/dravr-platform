// ABOUTME: The dravr-viz block schema must compile as JSON Schema and accept/reject the right blocks
// ABOUTME: A schema that is valid JSON but invalid JSON Schema yields no validator, silently

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use dravr_contremaitre::schemas::DRAVR_VIZ_SCHEMA;
use serde_json::{json, Value};

/// Compile the shipped schema the same way the pipeline registry does — the
/// `$schema`/`$id` URIs are stripped so compilation never reaches the network.
fn validator() -> jsonschema::Validator {
    let mut schema: Value = serde_json::from_str(DRAVR_VIZ_SCHEMA).expect("schema is valid JSON");
    let obj = schema.as_object_mut().expect("schema is an object");
    obj.remove("$schema");
    obj.remove("$id");
    jsonschema::validator_for(&schema).expect("schema must compile as JSON Schema")
}

fn chart() -> Value {
    json!({
        "type": "chart",
        "kind": "line",
        "source_tool": "analyze_training_load",
        "title": "CTL / ATL — 42 days",
        "x": { "label": "Date", "type": "time" },
        "series": [{
            "label": "CTL",
            "accent": "activity",
            "points": [["2026-07-01", 42.0], ["2026-07-02", 43.1]]
        }]
    })
}

fn table() -> Value {
    json!({
        "type": "table",
        "source_tool": "get_activities",
        "columns": ["Day", "Session", "Distance"],
        "rows": [["Tuesday", "Threshold", "12 km"], ["Sunday", "Long run", "24 km"]]
    })
}

#[test]
fn accepts_a_well_formed_chart_and_table() {
    let v = validator();
    assert!(v.is_valid(&chart()), "a well-formed chart must validate");
    assert!(v.is_valid(&table()), "a well-formed table must validate");
}

#[test]
fn source_tool_is_mandatory() {
    let v = validator();
    for mut block in [chart(), table()] {
        block.as_object_mut().unwrap().remove("source_tool");
        assert!(
            !v.is_valid(&block),
            "a block without source_tool must be rejected — attribution is the whole point"
        );
    }
}

#[test]
fn rejects_chart_kinds_outside_the_v1_vocabulary() {
    let v = validator();
    let mut block = chart();
    block["kind"] = json!("scatter");
    assert!(
        !v.is_valid(&block),
        "scatter is not in the v1 vocabulary and nothing can render it yet"
    );
}

#[test]
fn rejects_a_single_point_series() {
    // One point is not a trend; it is a number, and prose says it better.
    let v = validator();
    let mut block = chart();
    block["series"][0]["points"] = json!([["2026-07-01", 42.0]]);
    assert!(!v.is_valid(&block), "a one-point series must be rejected");
}

#[test]
fn accepts_a_null_y_as_a_genuine_gap() {
    // A missing measurement must be expressible, so the renderer can break the
    // line rather than draw a plunge to zero.
    let v = validator();
    let mut block = chart();
    block["series"][0]["points"] = json!([["2026-07-01", 42.0], ["2026-07-02", null]]);
    assert!(v.is_valid(&block), "a null y must be accepted as a gap");
}

#[test]
fn rejects_a_single_column_table() {
    let v = validator();
    let mut block = table();
    block["columns"] = json!(["Day"]);
    block["rows"] = json!([["Tuesday"], ["Sunday"]]);
    assert!(
        !v.is_valid(&block),
        "one column is a list, and prose reads better than a one-column table"
    );
}

#[test]
fn rejects_an_unknown_block_type() {
    let v = validator();
    let mut block = chart();
    block["type"] = json!("sparkline");
    assert!(!v.is_valid(&block), "only chart and table exist in v1");
}

#[test]
fn rejects_unknown_properties() {
    // Guards against a coach inventing fields the renderer would silently drop.
    let v = validator();
    let mut block = chart();
    block["render_hint"] = json!("big");
    assert!(!v.is_valid(&block), "unknown properties must be rejected");
}
