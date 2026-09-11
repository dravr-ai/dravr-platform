// ABOUTME: Every tool answering with Formatted<T> must declare `format` in its input schema
// ABOUTME: The served schema is the only place a caller learns the parameter exists
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A tool whose handler routes through `apply_format` reads `format` and acts on
//! it. Nothing else advertises that: the input schema a client fetches is the only
//! place the option is visible, so honouring it while declaring it nowhere makes it
//! undiscoverable to every client and to any model reading the tool catalogue to
//! decide how to call the tool.
//!
//! Six tool modules did exactly that — `agents`, `analytics`, `store`, `recipes`,
//! `sleep` and `admin`, module and tool identifiers that kept the older spelling —
//! 26 tools that answered `format=toon` correctly and never said so (registre#394).
//!
//! The check is derived, not listed: a tool is subject to it when its own declared
//! `outputSchema` carries the `Formatted` envelope, which is the `toon` arm that
//! `answers_with::<Formatted<T>>` emits. A new tool that answers with `Formatted`
//! is therefore covered the moment it is added to the table below, and the
//! assertion cannot be satisfied by an empty schema.

use dravr_tronc::mcp::schema::Tool;
use dravr_tronc::mcp::tool::McpTool;
use pierre_tool_runtime::implementations::admin::{
    AdminGetSystemAgentTool, AdminListSystemAgentsTool,
};
use pierre_tool_runtime::implementations::agents::{
    GetActiveAgentTool, GetAgentTool, ListAgentsTool, ListHiddenAgentsTool, SearchAgentsTool,
};
use pierre_tool_runtime::implementations::analytics::{
    AnalyzeActivityTool, AnalyzePerformanceTrendsTool, AnalyzeTrainingLoadTool,
    CalculateFitnessScoreTool, CalculateMetricsTool, CompareActivitiesTool, DetectPatternsTool,
    GenerateRecommendationsTool, GetActivityIntelligenceTool, PredictPerformanceTool,
};
use pierre_tool_runtime::implementations::athlete_stats::{GetAthleteTool, GetStatsTool};
use pierre_tool_runtime::implementations::recipes::{
    GetRecipeTool, ListRecipesTool, SearchRecipesTool,
};
use pierre_tool_runtime::implementations::sleep::{
    AnalyzeSleepQualityTool, CalculateRecoveryScoreTool, TrackSleepTrendsTool,
};
use pierre_tool_runtime::implementations::store::{
    BrowseAgentStoreTool, InstallAgentFromStoreTool, SearchAgentStoreTool,
};
use pierre_tool_runtime::implementations::stored_data::{
    GetHealthSnapshotsTool, GetRecoveryMetricsTool, GetSleepSessionsTool, ListDataSourcesTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;

/// Every tool that answers with a `Formatted<T>` envelope.
fn formatted_tools() -> Vec<Tool> {
    // Every one of these is a unit struct, so the value is the type name.
    macro_rules! def {
        ($t:ident) => {
            <$t as McpTool<dyn ToolRuntime>>::definition(&$t)
        };
    }
    vec![
        def!(AdminListSystemAgentsTool),
        def!(AdminGetSystemAgentTool),
        def!(AnalyzeTrainingLoadTool),
        def!(DetectPatternsTool),
        def!(CalculateFitnessScoreTool),
        def!(AnalyzeActivityTool),
        def!(GetActivityIntelligenceTool),
        def!(CalculateMetricsTool),
        def!(AnalyzePerformanceTrendsTool),
        def!(CompareActivitiesTool),
        def!(GenerateRecommendationsTool),
        def!(PredictPerformanceTool),
        def!(GetAthleteTool),
        def!(GetStatsTool),
        def!(ListAgentsTool),
        def!(GetAgentTool),
        def!(SearchAgentsTool),
        def!(GetActiveAgentTool),
        def!(ListHiddenAgentsTool),
        def!(ListRecipesTool),
        def!(GetRecipeTool),
        def!(SearchRecipesTool),
        def!(AnalyzeSleepQualityTool),
        def!(CalculateRecoveryScoreTool),
        def!(TrackSleepTrendsTool),
        def!(BrowseAgentStoreTool),
        def!(SearchAgentStoreTool),
        def!(InstallAgentFromStoreTool),
        def!(GetSleepSessionsTool),
        def!(GetRecoveryMetricsTool),
        def!(GetHealthSnapshotsTool),
        def!(ListDataSourcesTool),
    ]
}

/// True when the declared output schema is the `Formatted` envelope — the arm
/// carrying `toon` is what `answers_with::<Formatted<T>>` emits, and no other
/// payload in the registry has it.
fn answers_with_formatted(tool: &Tool) -> bool {
    tool.output_schema.as_ref().is_some_and(|s| {
        serde_json::to_string(s)
            .unwrap_or_default()
            .contains("\"toon\"")
    })
}

#[test]
fn every_formatted_tool_declares_the_format_parameter() {
    let tools = formatted_tools();
    assert_eq!(
        tools.len(),
        32,
        "the Formatted tool set changed; add or remove the tool here so it stays covered"
    );

    let mut undeclared = Vec::new();
    let mut checked = 0_usize;
    for tool in &tools {
        assert!(
            answers_with_formatted(tool),
            "{} is in the Formatted table but its outputSchema carries no toon arm",
            tool.name
        );
        checked += 1;
        let declares = tool
            .input_schema
            .get("properties")
            .and_then(|p| p.get("format"))
            .is_some();
        if !declares {
            undeclared.push(tool.name.clone());
        }
    }

    assert_eq!(checked, 32, "every tool in the table must be examined");
    assert!(
        undeclared.is_empty(),
        "these tools honour format=toon but declare it in no schema, so no client can \
         discover the option: {undeclared:?}"
    );
}

#[test]
fn the_declared_format_parameter_is_a_documented_string() {
    for tool in &formatted_tools() {
        let format = tool
            .input_schema
            .get("properties")
            .and_then(|p| p.get("format"))
            .unwrap_or_else(|| panic!("{} declares no format property", tool.name));

        assert_eq!(
            format.get("type").and_then(serde_json::Value::as_str),
            Some("string"),
            "{}: format must be a string",
            tool.name
        );

        let description = format
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("{} declares format with no description", tool.name));
        assert!(
            description.contains("toon"),
            "{}: the description must name toon, or a caller cannot learn the option \
             exists — got {description:?}",
            tool.name
        );
    }
}
