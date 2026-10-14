// ABOUTME: The dravr-viz block schema must compile as JSON Schema and accept/reject the right blocks
// ABOUTME: A schema that is valid JSON but invalid JSON Schema yields no validator, silently

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap/expect/panic (valid in tests per CLAUDE.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use dravr_contremaitre::schemas::DRAVR_VIZ_SCHEMA;
use dravr_contremaitre::system::VISUAL_BLOCKS as DRAVR_VIZ_DIRECTIVE;
use pierre_chat_pipeline::stages::viz_blocks::schema_contract;
use pierre_chat_pipeline::stages::viz_schema::SchemaTexts;
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

fn route() -> Value {
    json!({
        "type": "route",
        "source_tool": "get_activities",
        "activity_id": "a1b2c3d4",
        "title": "Sunday long run",
        "highlight": "climbs"
    })
}

#[test]
fn accepts_a_well_formed_chart_and_table() {
    let v = validator();
    assert!(v.is_valid(&chart()), "a well-formed chart must validate");
    assert!(v.is_valid(&table()), "a well-formed table must validate");
}

#[test]
fn accepts_a_well_formed_route() {
    // A route is a reference: the activity id names a recorded track the
    // platform reads itself, and highlight is the only rendering choice offered.
    let v = validator();
    assert!(v.is_valid(&route()), "a well-formed route must validate");
}

#[test]
fn rejects_a_route_carrying_coordinates() {
    // The whole point of reference-not-payload: an agent that transcribes
    // geometry has invented it, and the block must be refused outright rather
    // than drawn from whatever it wrote down.
    let v = validator();
    let mut block = route();
    block["coordinates"] = json!([[45.5017, -73.5673], [45.5088, -73.5878]]);
    assert!(
        !v.is_valid(&block),
        "a route carrying coordinates must be rejected — the track comes from the activity, never from the agent"
    );
}

#[test]
fn rejects_a_route_without_activity_id() {
    let v = validator();
    let mut block = route();
    block.as_object_mut().unwrap().remove("activity_id");
    assert!(
        !v.is_valid(&block),
        "a route with neither activity_id nor route_ref names no line and cannot be drawn"
    );
}

/// A route the coach proposes, by the handle `discover_routes` returned.
fn proposed_route() -> Value {
    json!({
        "type": "route",
        "source_tool": "discover_routes",
        "route_ref": "rt_3f9a0c41d27b8e65",
        "title": "Piste du canal de Lachine"
    })
}

#[test]
fn accepts_a_route_named_by_its_route_ref() {
    let v = validator();
    assert!(v.is_valid(&proposed_route()), "one discovered stretch");
    let mut itinerary = proposed_route();
    itinerary["source_tool"] = json!("compose_route");
    itinerary["route_ref"] = json!("rt_c07e5d2a9b146f38");
    assert!(v.is_valid(&itinerary), "an itinerary compose_route joined");
}

#[test]
fn rejects_a_route_naming_both_an_activity_and_a_route_ref() {
    // A block names one line. Both ids would leave the platform choosing which
    // to draw, and the caption would describe whichever it did not.
    let v = validator();
    let mut block = proposed_route();
    block["activity_id"] = json!("14523911087");
    assert!(
        !v.is_valid(&block),
        "activity_id and route_ref are one of two"
    );
}

#[test]
fn rejects_climbs_on_a_proposed_route() {
    // A public mapped line carries no vertical to find a climb in.
    let v = validator();
    let mut block = proposed_route();
    block["highlight"] = json!("climbs");
    assert!(
        !v.is_valid(&block),
        "highlight belongs to a recorded activity only"
    );
}

#[test]
fn rejects_a_route_ref_in_any_other_shape() {
    let v = validator();
    for malformed in [
        "rt_3F9A0C41D27B8E65",
        "rt_3f9a0c41",
        "3f9a0c41d27b8e65",
        "route_3f9a0c41d27b8e65",
    ] {
        let mut block = proposed_route();
        block["route_ref"] = json!(malformed);
        assert!(
            !v.is_valid(&block),
            "{malformed} is not a ref the route tools write"
        );
    }
}

#[test]
fn rejects_route_highlights_outside_the_v1_vocabulary() {
    let v = validator();
    let mut block = route();
    block["highlight"] = json!("elevation");
    assert!(
        !v.is_valid(&block),
        "elevation is not a route highlight; only none and climbs are rendered"
    );
}

#[test]
fn source_tool_is_mandatory() {
    let v = validator();
    for mut block in [chart(), table(), route()] {
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
    assert!(
        !v.is_valid(&block),
        "sparkline is not a block type; chart, table and route are the whole vocabulary"
    );
}

#[test]
fn rejects_unknown_properties() {
    // Guards against an agent inventing fields the renderer would silently drop.
    let v = validator();
    let mut block = chart();
    block["render_hint"] = json!("big");
    assert!(!v.is_valid(&block), "unknown properties must be rejected");
}

/// The generated contract must state the bound the hand-written prose omitted.
///
/// The directive used to list "at most 4 series and 400 points" and never that
/// `points` has `minItems: 2`; it now defers to this generated section entirely. An agent cannot obey a rule it is not told, and on
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
    for kind in ["chart", "table", "route"] {
        assert!(
            contract.contains(&format!("**`{kind}`**")),
            "every block kind must be described, {kind} is missing: {contract}"
        );
    }
}

/// The route arm's top-level `required` is only `type` and `source_tool`: the
/// id that names the line sits in the arm's own `oneOf`. A contract reading
/// only the top level would tell the agent `required: type, source_tool` and
/// never that a route needs an `activity_id` or a `route_ref`, nor that
/// `highlight` belongs to the first alone.
#[test]
fn generated_contract_states_the_route_alternatives() {
    let mut schemas = SchemaTexts::new();
    schemas.insert("dravr-viz".to_owned(), DRAVR_VIZ_SCHEMA.to_owned());
    let contract = schema_contract(&schemas);
    let route = contract
        .split("**`route`**")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .unwrap_or_else(|| panic!("the route arm must be described: {contract}"));

    assert!(
        route.contains("- exactly one of: `activity_id` | `route_ref`"),
        "both ids, and that a block names exactly one: {route}"
    );
    assert!(
        route.contains("- `highlight` only with `activity_id`"),
        "the property one alternative forbids is tied to the other: {route}"
    );
    assert!(
        route.contains("- `route_ref`: a string matching `^rt_[0-9a-f]{16}$`"),
        "the ref's shape is stated: {route}"
    );
    assert!(
        route.contains("- `highlight`: one of none, climbs"),
        "the enum line still stands: {route}"
    );

    // An arm with no alternation or pattern of its own states neither.
    for kind in ["chart", "table"] {
        let arm = contract
            .split(&format!("**`{kind}`**"))
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .unwrap_or_default();
        assert!(
            !arm.contains("exactly one of") && !arm.contains("a string matching"),
            "{kind} has no alternatives or patterns to state: {arm}"
        );
    }
}

/// Every `dravr-viz` example in the shipped directive must validate against the
/// shipped schema.
///
/// The prose teaches by example, so an example the validator rejects teaches the
/// agent a shape that will be refused at runtime — the athlete loses the visual
/// and nothing says why. That is not hypothetical: the directive's only example
/// was a time series, and asked to compare two athletes the agent produced the
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
