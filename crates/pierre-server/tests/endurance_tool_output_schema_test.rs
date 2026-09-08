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
use pierre_mcp_server::tools::implementations::endurance_export::{
    DossierExport, ExportDossierTool, ExportLatestSnapshotTool,
};
use pierre_mcp_server::tools::implementations::endurance_history::{
    ComputeTrainingHistoryResult, ComputeTrainingHistoryTool, GetTrainingHistoryTool,
    TrainingHistoryResult,
};
use pierre_mcp_server::tools::implementations::endurance_intervals::{
    ActivityStreamsResult, ExportIntervalsTool, ExportRoutesTool, ExtractActivityStreamsTool,
};
use pierre_tool_runtime::conversions::output_schema_for;
use pierre_tool_runtime::runtime::ToolRuntime;

#[test]
fn each_endurance_schema_is_attached_to_the_tool_it_names() {
    for (tool_name, declared, derived) in [
        (
            "export_latest_snapshot",
            <ExportLatestSnapshotTool as McpTool<dyn ToolRuntime>>::definition(
                &ExportLatestSnapshotTool,
            ),
            output_schema_for::<LatestSnapshot>(),
        ),
        (
            "export_intervals",
            <ExportIntervalsTool as McpTool<dyn ToolRuntime>>::definition(&ExportIntervalsTool),
            output_schema_for::<IntervalsExport>(),
        ),
        (
            "export_routes",
            <ExportRoutesTool as McpTool<dyn ToolRuntime>>::definition(&ExportRoutesTool),
            output_schema_for::<RouteSummary>(),
        ),
        (
            "compute_training_history",
            <ComputeTrainingHistoryTool as McpTool<dyn ToolRuntime>>::definition(
                &ComputeTrainingHistoryTool,
            ),
            output_schema_for::<ComputeTrainingHistoryResult>(),
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
    let schema = output_schema_for::<LatestSnapshot>();
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
    let derived = output_schema_for::<ComputeTrainingHistoryResult>();
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

// ============================================================================
// the three the cageux release unblocked
// ============================================================================

#[test]
fn the_cageux_dependent_endurance_tools_declare_their_shapes() {
    for (tool_name, declared, derived) in [
        (
            "get_training_history",
            <GetTrainingHistoryTool as McpTool<dyn ToolRuntime>>::definition(
                &GetTrainingHistoryTool,
            ),
            output_schema_for::<TrainingHistoryResult>(),
        ),
        (
            "extract_activity_streams",
            <ExtractActivityStreamsTool as McpTool<dyn ToolRuntime>>::definition(
                &ExtractActivityStreamsTool,
            ),
            output_schema_for::<ActivityStreamsResult>(),
        ),
        (
            "export_dossier",
            <ExportDossierTool as McpTool<dyn ToolRuntime>>::definition(&ExportDossierTool),
            output_schema_for::<DossierExport>(),
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

/// Every training-history day carries its form reading, and the payload
/// carries the method that produced it.
///
/// Shipping bare ctl/atl/tsb floats is how a raw `-77` reached an athlete as a
/// diagnosis nobody could explain (registre#199). The schema is where that
/// becomes a promise rather than a habit.
#[test]
fn a_training_history_day_never_ships_a_bare_form_number() {
    let schema = output_schema_for::<TrainingHistoryResult>();
    let rendered = serde_json::to_string(&schema).expect("serializes");

    for required in ["tsb_pct_of_ctl", "form_band", "interpretation"] {
        assert!(
            rendered.contains(required),
            "a form-bearing payload must declare {required}"
        );
    }

    // And the interpretation must state the method, not just the bands.
    for stated in ["method", "deep_fatigue_is_not_overtraining"] {
        assert!(
            rendered.contains(stated),
            "the interpretation must declare {stated} — a coach asked to explain \
             the number has to be able to"
        );
    }
}

#[test]
fn the_interpretation_reaches_the_schema_with_its_framing_intact() {
    // What a SCHEMA can promise is the shape and the field descriptions — the
    // disclaimer sentences themselves are runtime values, and pierre-core's
    // own form_reading_test pins those. This asserts the part that travels to
    // an MCP client: the interpretation block is declared, all seven keys of
    // it, and form_band's description says what a band is not.
    //
    // Two earlier versions of this test were wrong in the same way: they
    // matched prose. One banned "injury risk" and failed on a doc comment
    // that FORBIDS the framing; one asserted a runtime string that a schema
    // never carries. Assert the structure, and the one description you wrote.
    let schema = output_schema_for::<TrainingHistoryResult>();
    let interpretation = &schema["$defs"]["FormInterpretation"]["properties"];

    for key in [
        "ctl",
        "atl",
        "tsb",
        "tsb_pct_of_ctl",
        "form_band",
        "method",
        "deep_fatigue_is_not_overtraining",
    ] {
        assert!(
            !interpretation[key].is_null(),
            "the interpretation must declare {key}: {interpretation:#}"
        );
    }

    let band = interpretation["form_band"]["description"]
        .as_str()
        .expect("form_band carries a description");
    assert!(
        band.contains("not an injury prediction"),
        "a band is a fatigue reading, and the schema has to say so to every \
         client rather than only to ours: {band}"
    );
}
