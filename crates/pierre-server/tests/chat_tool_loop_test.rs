// ABOUTME: Tests for the chat tool loop strategy module
// ABOUTME: Validates tool call parsing, catalog generation, and result formatting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for chat tool loop strategies.

use pierre_core::llm::MessageRole;
use pierre_llm::{ChatMessage, FunctionDeclaration, FunctionResponse};
use pierre_tool_runtime::tool_execution::{
    generate_tool_catalog, inject_tool_catalog_into_system_prompt, parse_lenient_tool_call_blocks,
    parse_tool_call_blocks, strip_simulation_artifacts,
};
use pierre_tool_runtime::tool_results::{extract_activity_list, format_tool_results_as_text};

#[test]
fn test_parse_single_tool_call() {
    let content = r#"Let me fetch your activities.

<tool_call>
{"name": "get_activities", "arguments": {"provider": "strava", "limit": 25}}
</tool_call>"#;

    let calls = parse_tool_call_blocks(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_activities");
    assert_eq!(calls[0].args["provider"], "strava");
    assert_eq!(calls[0].args["limit"], 25);
}

#[test]
fn test_parse_multiple_tool_calls() {
    let content = r#"I'll fetch your data.

<tool_call>
{"name": "get_activities", "arguments": {"provider": "strava", "limit": 10}}
</tool_call>

And your profile:
<tool_call>
{"name": "get_athlete", "arguments": {"provider": "strava"}}
</tool_call>"#;

    let calls = parse_tool_call_blocks(content);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "get_activities");
    assert_eq!(calls[1].name, "get_athlete");
}

#[test]
fn test_parse_no_tool_calls() {
    let content = "Here is your analysis of the data. You had a great week!";
    let calls = parse_tool_call_blocks(content);
    assert!(calls.is_empty());
}

#[test]
fn test_parse_malformed_json_skipped() {
    let content = r#"<tool_call>
{not valid json}
</tool_call>

<tool_call>
{"name": "get_stats", "arguments": {"provider": "strava"}}
</tool_call>"#;

    let calls = parse_tool_call_blocks(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_stats");
}

#[test]
fn test_lenient_parse_flat_args_shape() {
    // A native-tool model (e.g. Cohere) can emit a tool call as a text block
    // with FLAT args — parameters as top-level siblings of `name`, with no
    // nested `arguments` object. The canonical parser drops those args; the
    // lenient parser keeps them so the tool runs with the right parameters.
    // Regression for the 2026-06-17 Telegram "2022 races" leak.
    let content = r#"<tool_call>
{"name": "get_activities", "after": 1640995200, "before": 1672531200, "limit": 200}
</tool_call>"#;

    let lenient = parse_lenient_tool_call_blocks(content);
    assert_eq!(lenient.len(), 1);
    assert_eq!(lenient[0].name, "get_activities");
    assert_eq!(lenient[0].args["after"], 1_640_995_200_i64);
    assert_eq!(lenient[0].args["before"], 1_672_531_200_i64);
    assert_eq!(lenient[0].args["limit"], 200);

    // The canonical parser drops the flat args (only reads nested `arguments`).
    let canonical = parse_tool_call_blocks(content);
    assert_eq!(canonical.len(), 1);
    assert_eq!(canonical[0].name, "get_activities");
    assert_eq!(
        canonical[0].args.as_object().map(serde_json::Map::len),
        Some(0),
        "canonical parser must yield empty args for the flat shape"
    );
}

#[test]
fn test_lenient_parse_nested_arguments_shape() {
    // The lenient parser must also handle the canonical nested shape identically.
    let content = r#"<tool_call>
{"name": "get_activities", "arguments": {"provider": "strava", "limit": 25}}
</tool_call>"#;

    let calls = parse_lenient_tool_call_blocks(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_activities");
    assert_eq!(calls[0].args["provider"], "strava");
    assert_eq!(calls[0].args["limit"], 25);
    assert!(
        calls[0].args.get("arguments").is_none(),
        "nested arguments must be unwrapped, not double-nested"
    );
}

#[test]
fn test_parse_tool_call_without_arguments() {
    let content = r#"<tool_call>
{"name": "get_connection_status"}
</tool_call>"#;

    let calls = parse_tool_call_blocks(content);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_connection_status");
    assert!(calls[0].args.is_object());
}

#[test]
fn test_strip_simulation_artifacts_removes_tool_calls() {
    let content = r#"Let me fetch your data.

<tool_call>
{"name": "get_activities", "arguments": {"provider": "strava"}}
</tool_call>

And some more text."#;

    let stripped = strip_simulation_artifacts(content);
    assert_eq!(
        stripped,
        "Let me fetch your data.\n\n\n\nAnd some more text."
    );
    assert!(!stripped.contains("<tool_call>"));
}

#[test]
fn test_strip_simulation_artifacts_removes_echoed_tool_results() {
    // Weak CLI models parrot the injected `<tool_result>` turn back into their
    // reply; strip_simulation_artifacts removes that scaffolding too, not just
    // `<tool_call>` blocks, so neither leaks to the user.
    let content = r#"Here is your summary.

<tool_result>
{"activities": 3}
</tool_result>

You ran three times."#;

    let stripped = strip_simulation_artifacts(content);
    assert!(!stripped.contains("<tool_result>"));
    assert!(stripped.contains("Here is your summary."));
    assert!(stripped.contains("You ran three times."));
}

#[test]
fn test_strip_preserves_no_tool_calls() {
    let content = "Just plain text with no tool calls.";
    let stripped = strip_simulation_artifacts(content);
    assert_eq!(stripped, content);
}

#[test]
fn test_generate_tool_catalog_has_tools() {
    let declarations = vec![
        FunctionDeclaration {
            name: "get_activities".to_owned(),
            description: "Get user's recent fitness activities".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "limit": {"type": "integer"}
                },
                "required": ["provider"]
            })),
        },
        FunctionDeclaration {
            name: "get_athlete".to_owned(),
            description: "Get user's athlete profile".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"}
                },
                "required": ["provider"]
            })),
        },
    ];

    let catalog = generate_tool_catalog(&declarations);
    assert!(catalog.contains("### get_activities"));
    assert!(catalog.contains("### get_athlete"));
    assert!(catalog.contains("<tool_call>"));
    assert!(catalog.contains("`provider` (string, required)"));
    assert!(catalog.contains("`limit` (integer)"));
}

#[test]
fn test_format_tool_results_as_text() {
    let responses = vec![FunctionResponse {
        name: "get_stats".to_owned(),
        response: serde_json::json!({"total_distance_km": 1234.5}),
    }];

    let text = format_tool_results_as_text(&responses);
    assert!(text.contains("<tool_result name=\"get_stats\">"));
    assert!(text.contains("1234.5"));
    assert!(text.contains("</tool_result>"));
}

#[test]
fn test_inject_tool_catalog_appends_to_system() {
    let mut messages = vec![
        ChatMessage::system("You are a fitness coach."),
        ChatMessage::user("Hello"),
    ];
    let catalog = "\n\n## Tools\nSome tools here.";

    inject_tool_catalog_into_system_prompt(&mut messages, catalog);

    assert_eq!(messages.len(), 2);
    assert!(messages[0].content.contains("You are a fitness coach."));
    assert!(messages[0].content.contains("## Tools"));
}

#[test]
fn test_inject_tool_catalog_creates_system_when_missing() {
    let mut messages = vec![ChatMessage::user("Hello")];
    let catalog = "## Tools\nSome tools here.";

    inject_tool_catalog_into_system_prompt(&mut messages, catalog);

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::System);
    assert!(messages[0].content.contains("## Tools"));
}

#[test]
fn test_extract_activity_list_found() {
    let responses = vec![FunctionResponse {
        name: "get_activities".to_owned(),
        response: serde_json::json!({
            "activities": [],
            "activity_list": "| # | Date | Sport |\n|---|------|-------|\n| 1 | Today | Run |"
        }),
    }];

    let list = extract_activity_list(&responses);
    assert!(list.is_some());
    assert!(list.unwrap_or_default().contains("| # | Date | Sport |"));
}

#[test]
fn test_extract_activity_list_not_found() {
    let responses = vec![FunctionResponse {
        name: "get_stats".to_owned(),
        response: serde_json::json!({"total": 100}),
    }];

    assert!(extract_activity_list(&responses).is_none());
}
