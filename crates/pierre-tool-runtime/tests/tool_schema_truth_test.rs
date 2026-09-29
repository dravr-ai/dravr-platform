// ABOUTME: Pins that compiled-in tool schemas state what their handlers do: params, defaults, requireds
// ABOUTME: These ship whenever the contremaitre overlay sync is unavailable and generate the TS SDK types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Each assertion is a defect a model acted on: a goal type the tracker never
//! measured, a default that was not the default, a required parameter the
//! handler defaults, a declared parameter no handler read, or a parameter a
//! handler read that no schema declared.

use dravr_tronc::mcp::schema::Tool;
use dravr_tronc::mcp::tool::McpTool;
use serde_json::{json, Value};

use pierre_tool_runtime::implementations::admin::{
    AdminCreateSystemAgentTool, AdminUpdateSystemAgentTool,
};
use pierre_tool_runtime::implementations::agents::{
    CreateAgentTool, ListAgentsTool, SearchAgentsTool, UpdateAgentTool,
};
use pierre_tool_runtime::implementations::analytics::{
    AnalyzeActivityTool, AnalyzePerformanceTrendsTool, CompareActivitiesTool, DetectPatternsTool,
    GetActivityIntelligenceTool, PredictPerformanceTool,
};
use pierre_tool_runtime::implementations::goals::{
    AnalyzeGoalFeasibilityTool, SetGoalTool, TrackProgressTool,
};
use pierre_tool_runtime::implementations::physiology::EstimateVo2maxTool;
use pierre_tool_runtime::implementations::recipes::{SaveRecipeTool, ValidateRecipeTool};
use pierre_tool_runtime::implementations::sleep::{
    AnalyzeSleepQualityTool, CalculateRecoveryScoreTool, OptimizeSleepScheduleTool,
    SuggestRestDayTool, TrackSleepTrendsTool,
};
use pierre_tool_runtime::implementations::sync::RefreshProviderDataTool;
use pierre_tool_runtime::runtime::ToolRuntime;

macro_rules! def {
    ($t:ident) => {
        <$t as McpTool<dyn ToolRuntime>>::definition(&$t)
    };
}

fn property<'a>(tool: &'a Tool, name: &str) -> Option<&'a Value> {
    tool.input_schema
        .get("properties")
        .and_then(|p| p.get(name))
}

fn description(tool: &Tool, name: &str) -> String {
    property(tool, name)
        .and_then(|p| p.get("description"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{} declares no described {name}", tool.name))
        .to_owned()
}

fn required(tool: &Tool) -> Vec<String> {
    tool.input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn goal_tools_advertise_only_the_goal_types_and_defaults_they_honour() {
    for tool in [def!(SetGoalTool), def!(AnalyzeGoalFeasibilityTool)] {
        let goal_type = description(&tool, "goal_type");
        for measured in [
            "'distance' (km)",
            "'duration' (hours)",
            "'frequency' (activities)",
        ] {
            assert!(goal_type.contains(measured), "{}: {goal_type}", tool.name);
        }
        for never_measured in ["'time'", "'performance'"] {
            assert!(
                !goal_type.contains(never_measured),
                "{}: {goal_type}",
                tool.name
            );
        }
        assert_eq!(required(&tool), vec!["goal_type", "target_value"]);
    }

    let set_goal = def!(SetGoalTool);
    assert!(description(&set_goal, "timeframe").contains("Default: 'month'"));
    let sport = description(&set_goal, "sport");
    assert!(
        !sport.contains("Default: 'Running'"),
        "no sport is assumed: {sport}"
    );

    let timeframe_days = description(&def!(AnalyzeGoalFeasibilityTool), "timeframe_days");
    assert!(
        timeframe_days.contains("Default: 90") && timeframe_days.contains("365"),
        "the handler defaults to 90 days and caps at 365: {timeframe_days}"
    );

    let track_progress = def!(TrackProgressTool);
    assert!(
        !track_progress.description.contains("milestone"),
        "track_progress reports no milestones: {}",
        track_progress.description
    );
}

#[test]
fn recovery_tools_declare_the_activity_source_and_no_unread_training_load() {
    for tool in [
        def!(CalculateRecoveryScoreTool),
        def!(SuggestRestDayTool),
        def!(OptimizeSleepScheduleTool),
    ] {
        assert!(
            property(&tool, "training_load").is_none(),
            "{} computes training load itself and never read the argument",
            tool.name
        );
        assert!(
            property(&tool, "activity_provider").is_some(),
            "{} reads activity_provider",
            tool.name
        );
    }
}

#[test]
fn sleep_tools_state_their_real_defaults_and_manual_data_shape() {
    let trends = def!(TrackSleepTrendsTool);
    let days = property(&trends, "days").unwrap();
    assert_eq!(days["type"], "integer");
    let text = days["description"].as_str().unwrap();
    assert!(
        text.contains("default 14") && text.contains("max 366"),
        "{text}"
    );

    let quality = def!(AnalyzeSleepQualityTool);
    let sleep_data = description(&quality, "sleep_data");
    assert!(
        sleep_data.contains("date") && sleep_data.contains("(required)"),
        "SleepData cannot deserialize without a date: {sleep_data}"
    );
}

#[test]
fn a_parameter_the_handler_defaults_is_not_required() {
    for tool in [
        def!(AnalyzeActivityTool),
        def!(GetActivityIntelligenceTool),
        def!(CompareActivitiesTool),
    ] {
        assert_eq!(required(&tool), vec!["activity_id"], "{}", tool.name);
        assert!(description(&tool, "provider").contains("Defaults to configured provider"));
    }
    assert!(required(&def!(AnalyzePerformanceTrendsTool)).is_empty());
    assert!(description(&def!(AnalyzePerformanceTrendsTool), "metric").contains("'pace' (default)"));
    assert!(required(&def!(RefreshProviderDataTool)).is_empty());
    assert!(description(&def!(RefreshProviderDataTool), "provider").contains("'all' (the default)"));
}

#[test]
fn analytics_descriptions_name_only_the_values_the_handlers_accept() {
    let patterns = description(&def!(DetectPatternsTool), "pattern_type");
    assert!(
        patterns.contains("any other value is refused"),
        "{patterns}"
    );

    let target_sport = description(&def!(PredictPerformanceTool), "target_sport");
    for unmodelled in ["'Ride'", "'Swim'"] {
        assert!(!target_sport.contains(unmodelled), "{target_sport}");
    }

    let method = description(&def!(EstimateVo2maxTool), "method");
    let listed = method.split('.').next().unwrap();
    assert!(
        listed.contains("race_result"),
        "the list of methods names every method the handler accepts: {listed}"
    );
}

#[test]
fn agent_tools_declare_what_their_handlers_read() {
    let list = def!(ListAgentsTool);
    assert!(property(&list, "include_hidden").is_some());
    assert!(description(&list, "limit").contains("max: 100"));

    for tool in [
        def!(UpdateAgentTool),
        def!(AdminCreateSystemAgentTool),
        def!(AdminUpdateSystemAgentTool),
    ] {
        assert!(
            property(&tool, "sample_prompts").is_some(),
            "{} reads sample_prompts",
            tool.name
        );
    }

    for tool in [
        def!(ListAgentsTool),
        def!(CreateAgentTool),
        def!(UpdateAgentTool),
        def!(SearchAgentsTool),
        def!(AdminCreateSystemAgentTool),
        def!(AdminUpdateSystemAgentTool),
    ] {
        let category = description(&tool, "category");
        for name in [
            "training",
            "nutrition",
            "recovery",
            "recipes",
            "mobility",
            "analysis",
            "custom",
        ] {
            assert!(category.contains(name), "{}: {category}", tool.name);
        }
    }
}

#[test]
fn recipe_tools_declare_what_their_handlers_read_and_nothing_else() {
    let validate = def!(ValidateRecipeTool);
    assert!(
        property(&validate, "name").is_none(),
        "validation reads servings and ingredients only"
    );
    let item = &property(&validate, "ingredients").unwrap()["items"];
    assert_eq!(
        item["required"],
        json!(["name", "amount"]),
        "an ingredient without a unit is read as grams"
    );

    let save = def!(SaveRecipeTool);
    for read in ["prep_time_mins", "cook_time_mins"] {
        assert!(property(&save, read).is_some(), "save_recipe reads {read}");
    }
    let item = &property(&save, "ingredients").unwrap()["items"];
    assert!(item["properties"].get("preparation").is_some());
}
