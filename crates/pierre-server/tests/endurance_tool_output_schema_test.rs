// ABOUTME: Asserts the endurance export tools declare the schemas their result types derive
// ABOUTME: These four already answered with typed structs; the schema is the promise made of them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! MCP requires a tool declaring `outputSchema` to answer with conforming
//! `structuredContent`. These four endurance tools were already the easy
//! case — each serializes a typed struct rather than a `json!` literal — so
//! declaring the contract cost only the derive.
//!
//! The tools live in `pierre-server` rather than `pierre-tool-runtime`, which
//! is why they are pinned here and not in that crate's schema test.

use dravr_tronc::mcp::tool::McpTool;
use pierre_fitness_compute::intervals::IntervalsExport;
use pierre_fitness_compute::latest_snapshot::LatestSnapshot;
use pierre_fitness_compute::routes::RouteSummary;
use pierre_mcp_server::tools::implementations::endurance_export::ExportLatestSnapshotTool;
use pierre_mcp_server::tools::implementations::endurance_history::{
    ComputeTrainingHistoryResult, ComputeTrainingHistoryTool,
};
use pierre_mcp_server::tools::implementations::endurance_intervals::{
    ExportIntervalsTool, ExportRoutesTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;

#[test]
fn each_endurance_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "export_latest_snapshot",
            <ExportLatestSnapshotTool as McpTool<dyn ToolRuntime>>::definition(
                &ExportLatestSnapshotTool,
            ),
            serde_json::to_value(schemars::schema_for!(LatestSnapshot)).expect("derives"),
        ),
        (
            "export_intervals",
            <ExportIntervalsTool as McpTool<dyn ToolRuntime>>::definition(&ExportIntervalsTool),
            serde_json::to_value(schemars::schema_for!(IntervalsExport)).expect("derives"),
        ),
        (
            "export_routes",
            <ExportRoutesTool as McpTool<dyn ToolRuntime>>::definition(&ExportRoutesTool),
            serde_json::to_value(schemars::schema_for!(RouteSummary)).expect("derives"),
        ),
        (
            "compute_training_history",
            <ComputeTrainingHistoryTool as McpTool<dyn ToolRuntime>>::definition(
                &ComputeTrainingHistoryTool,
            ),
            serde_json::to_value(schemars::schema_for!(ComputeTrainingHistoryResult))
                .expect("derives"),
        ),
    ] {
        assert_eq!(
            declared.name, tool_name,
            "the tool struct under test is not the tool it was paired with"
        );
        assert_eq!(
            declared
                .output_schema
                .unwrap_or_else(|| panic!("{tool_name} must declare an outputSchema")),
            derived,
            "{tool_name} declares a schema derived from a DIFFERENT result type"
        );
    }
}

#[test]
fn the_snapshot_schema_describes_the_metrics_the_contract_names() {
    // The tool's own description promises intensity factor, efficiency
    // factor, variability index, aerobic decoupling and time in zone. A
    // schema that did not describe them would be advertising a shape the
    // athlete cannot rely on.
    let schema = serde_json::to_value(schemars::schema_for!(LatestSnapshot)).expect("derives");
    let rendered = serde_json::to_string(&schema).expect("serializes");

    for promised in [
        "intensity_factor",
        "efficiency_factor",
        "variability_index",
        "decoupling",
        "zone_distribution",
    ] {
        assert!(
            rendered.contains(promised),
            "the snapshot contract names {promised}, so the schema must describe it"
        );
    }
}

#[test]
fn compute_training_history_reports_the_window_it_actually_used() {
    // The caller may pass no window and take the default, so the answer
    // echoes the range it computed. rows_upserted is how a coach tells a
    // recompute that had days to work with from one that did not — zero is
    // a valid answer for a window the athlete did not train in.
    let derived =
        serde_json::to_value(schemars::schema_for!(ComputeTrainingHistoryResult)).expect("derives");
    let validator = jsonschema::validator_for(&derived).expect("compiles");

    let empty_window = serde_json::to_value(ComputeTrainingHistoryResult {
        from: "2026-08-01".to_owned(),
        to: "2026-08-31".to_owned(),
        rows_upserted: 0,
    })
    .expect("serializes");
    assert!(
        validator.is_valid(&empty_window),
        "a window with nothing in it must still validate:\n{empty_window:#}"
    );

    for required in ["from", "to", "rows_upserted"] {
        let mut partial = empty_window.clone();
        partial.as_object_mut().expect("object").remove(required);
        assert!(
            !validator.is_valid(&partial),
            "dropping {required} must fail the schema, or it is describing nothing"
        );
    }
}
