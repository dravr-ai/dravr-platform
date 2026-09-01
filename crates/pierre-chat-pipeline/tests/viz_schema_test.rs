// ABOUTME: The dravr-viz block schema must compile as JSON Schema and accept/reject the right blocks
// ABOUTME: A schema that is valid JSON but invalid JSON Schema yields no validator, silently

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use dravr_contremaitre::schemas::DRAVR_VIZ_SCHEMA;
use dravr_contremaitre::system::VISUAL_BLOCKS as DRAVR_VIZ_DIRECTIVE;
use pierre_chat_pipeline::stages::structured_output::SchemaTexts;
use pierre_chat_pipeline::stages::viz_blocks::schema_contract;
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

/// The generated contract must state the bound the hand-written prose omitted.
///
/// The directive used to list "at most 4 series and 400 points" and never that
/// `points` has `minItems: 2`; it now defers to this generated section entirely. A coach cannot obey a rule it is not told, and on
/// 2026-08-31 one did not: a two-athlete comparison written as one series per
/// athlete was refused on every pass. Asserting the minimum specifically —
/// rather than that the text is non-empty — is the difference between this test
/// and one a stub would pass.
#[test]
fn generated_contract_states_the_points_minimum() {
    let mut schemas = SchemaTexts::new();
    schemas.insert("dravr-viz".to_owned(), DRAVR_VIZ_SCHEMA.to_owned());
    let contract = schema_contract(&schemas);

    assert!(
        contract.contains("series[].points"),
        "the per-series points bound must be stated: {contract}"
    );
    assert!(
        contract.contains("2 to 400 entries"),
        "the minimum of 2 points per series is the rule that was missing: {contract}"
    );
    assert!(
        contract.contains("1 to 4 entries"),
        "the series bound must survive too: {contract}"
    );
    for kind in ["line", "bar", "area"] {
        assert!(contract.contains(kind), "chart kind {kind} must be listed");
    }
    for kind in ["chart", "table"] {
        assert!(
            contract.contains(&format!("**`{kind}`**")),
            "both block kinds must be described: {contract}"
        );
    }
}

/// Every `dravr-viz` example in the shipped directive must validate against the
/// shipped schema.
///
/// The prose teaches by example, so an example the validator rejects teaches the
/// coach a shape that will be refused at runtime — the athlete loses the visual
/// and nothing says why. That is not hypothetical: the directive's only example
/// was a time series, and asked to compare two athletes the coach produced the
/// natural-looking generalisation (one series each, one point apiece) which
/// `points`' `minItems: 2` rejects. The fix was a second, category-shaped
/// example; this keeps the next one honest.
///
/// Both texts are compiled-in contremaitre constants, so this costs no I/O and
/// fails the moment the directive and the schema disagree.
#[test]
fn every_example_in_the_directive_validates() {
    let validator = validator();
    let examples: Vec<&str> = DRAVR_VIZ_DIRECTIVE
        .split("```dravr-viz")
        .skip(1)
        .filter_map(|rest| rest.split("```").next())
        .collect();

    assert!(
        examples.len() >= 2,
        "the directive must keep a time-series AND a category example; found {}",
        examples.len()
    );

    for (i, body) in examples.iter().enumerate() {
        let block: Value = serde_json::from_str(body.trim())
            .unwrap_or_else(|e| panic!("directive example {i} is not valid JSON: {e}\n{body}"));
        let errors: Vec<String> = validator
            .iter_errors(&block)
            .map(|e| format!("{}: {e}", e.instance_path()))
            .collect();
        assert!(
            errors.is_empty(),
            "directive example {i} does not validate — it teaches a block the \
             platform will refuse: {}\n{body}",
            errors.join("; ")
        );
    }

    // The category example is the one the corpus needed; assert its shape
    // specifically so a well-meaning edit cannot collapse both examples back
    // into two time series.
    let has_category = examples.iter().any(|b| {
        serde_json::from_str::<Value>(b.trim()).is_ok_and(|v| {
            v.pointer("/x/type").and_then(Value::as_str) == Some("category")
                && v.get("series")
                    .and_then(Value::as_array)
                    .is_some_and(|s| s.len() == 1)
        })
    });
    assert!(
        has_category,
        "the directive must show a category comparison as ONE series whose \
         points are the categories — the shape the coach otherwise gets wrong"
    );
}
