// ABOUTME: One integration-test binary for pierre-tool-runtime: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the tool runtime crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod activity_coverage_framing_test;
#[cfg(feature = "tools-data")]
mod activity_detail_route_overview_test;
mod activity_float_precision_test;
#[cfg(feature = "tools-data")]
mod activity_list_local_clock_test;
#[cfg(feature = "tools-data")]
mod activity_list_merge_note_test;
#[cfg(feature = "tools-data")]
mod activity_name_neutralized_test;
mod activity_revalidation_dedup_test;
#[cfg(feature = "tools-data")]
mod activity_self_report_render_test;
#[cfg(feature = "tools-data")]
mod backfill_coverage_logic_test;
#[cfg(feature = "client-chat")]
mod cli_loop_request_mcp_servers_test;
#[cfg(feature = "tools-analytics")]
mod fitness_score_window_test;
#[cfg(all(
    feature = "tools-admin",
    feature = "tools-analytics",
    feature = "tools-agents",
    feature = "tools-data",
    feature = "tools-recipes",
    feature = "tools-sleep"
))]
mod format_parameter_declared_test;
#[cfg(feature = "tools-goals")]
mod goal_activities_test;
#[cfg(feature = "tools-groups")]
mod group_fitness_weekly_metrics_test;
#[cfg(feature = "client-chat")]
mod headless_lost_turn_test;
#[cfg(feature = "tools-sleep")]
mod sleep_payload_value_test;
#[cfg(feature = "tools-data")]
mod sport_type_filter_test;
#[cfg(feature = "tools-data")]
mod stored_data_range_test;
mod tool_error_text_test;
#[cfg(feature = "client-chat")]
mod tool_loop_fallback_classification_test;
#[cfg(all(
    feature = "tools-admin",
    feature = "tools-analytics",
    feature = "tools-agents",
    feature = "tools-config",
    feature = "tools-connection",
    feature = "tools-data",
    feature = "tools-goals",
    feature = "tools-groups",
    feature = "tools-memory",
    feature = "tools-mobility",
    feature = "tools-nutrition",
    feature = "tools-recipes",
    feature = "tools-sleep",
    feature = "tools-verification"
))]
mod tool_output_schema_test;
mod tool_result_float_precision_test;
#[cfg(all(
    feature = "tools-admin",
    feature = "tools-agents",
    feature = "tools-analytics",
    feature = "tools-config",
    feature = "tools-connection",
    feature = "tools-goals",
    feature = "tools-recipes",
    feature = "tools-sleep"
))]
mod tool_schema_truth_test;
#[cfg(feature = "tools-nutrition")]
mod usda_ingredient_guard_test;
