// ABOUTME: Tests for the chat tool loop strategy module
// ABOUTME: Validates tool call parsing, catalog generation, and result formatting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for chat tool loop strategies.

use pierre_core::llm::MessageRole;
use pierre_mcp_server::llm::{ChatMessage, FunctionDeclaration, FunctionResponse};
use pierre_mcp_server::routes::chat_tool_loop::{
    extract_activity_list, format_tool_results_as_text, generate_tool_catalog,
    inject_tool_catalog_into_system_prompt, parse_tool_call_blocks, strip_tool_call_blocks,
};

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
fn test_strip_tool_call_blocks() {
    let content = r#"Let me fetch your data.

<tool_call>
{"name": "get_activities", "arguments": {"provider": "strava"}}
</tool_call>

And some more text."#;

    let stripped = strip_tool_call_blocks(content);
    assert_eq!(
        stripped,
        "Let me fetch your data.\n\n\n\nAnd some more text."
    );
    assert!(!stripped.contains("<tool_call>"));
}

#[test]
fn test_strip_preserves_no_tool_calls() {
    let content = "Just plain text with no tool calls.";
    let stripped = strip_tool_call_blocks(content);
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
