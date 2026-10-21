// ABOUTME: Pins the tool-calling wire shape now that the function types are embacle's own
// ABOUTME: A Tool and a ChatResponseWithTools round-trip through JSON as {name,description,parameters} and {name,args}

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `FunctionCall`, `FunctionResponse` and `FunctionDeclaration` are embacle's
//! types, re-exported, with no platform copy and no rename between them: what
//! embacle's text-tool parser returns goes into a `ChatResponseWithTools` as it
//! is. The JSON a tool-carrying response serializes to is unchanged by that,
//! and this file pins it: a stored or forwarded call still reads
//! `{"name": …, "args": …}`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use embacle::tool_simulation::parse_tool_call_blocks;
use pierre_llm::{ChatResponseWithTools, FunctionDeclaration, FunctionResponse, Tool};
use serde_json::json;

#[test]
fn embacles_parsed_calls_ride_a_tool_response_and_round_trip_as_name_and_args() {
    let calls = parse_tool_call_blocks(
        r#"<tool_call>{"name": "get_activities", "arguments": {"provider": "strava", "limit": 5}}</tool_call>"#,
    );
    let response = ChatResponseWithTools {
        content: None,
        function_calls: Some(calls),
        model: "claude-sonnet-5".to_owned(),
        usage: None,
        finish_reason: Some("tool_calls".to_owned()),
        warnings: None,
    };

    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(
        wire["function_calls"],
        json!([{"name": "get_activities", "args": {"provider": "strava", "limit": 5}}])
    );

    let back: ChatResponseWithTools = serde_json::from_value(wire).unwrap();
    let calls = back.function_calls.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_activities");
    assert_eq!(calls[0].args, json!({"provider": "strava", "limit": 5}));
    assert_eq!(back.model, "claude-sonnet-5");
}

#[test]
fn a_tool_surface_serializes_its_declarations_and_omits_absent_parameters() {
    let tool = Tool {
        function_declarations: vec![
            FunctionDeclaration {
                name: "get_activities".to_owned(),
                description: "List recent activities".to_owned(),
                parameters: Some(
                    json!({"type": "object", "properties": {"limit": {"type": "integer"}}}),
                ),
            },
            FunctionDeclaration {
                name: "get_athlete".to_owned(),
                description: "Profile".to_owned(),
                parameters: None,
            },
        ],
    };

    let wire = serde_json::to_value(&tool).unwrap();
    assert_eq!(
        wire,
        json!({"function_declarations": [
            {
                "name": "get_activities",
                "description": "List recent activities",
                "parameters": {"type": "object", "properties": {"limit": {"type": "integer"}}}
            },
            {"name": "get_athlete", "description": "Profile"}
        ]})
    );

    let back: Tool = serde_json::from_value(wire).unwrap();
    assert_eq!(back.function_declarations.len(), 2);
    assert_eq!(back.function_declarations[1].parameters, None);
}

#[test]
fn a_function_response_round_trips_as_name_and_response() {
    let response = FunctionResponse {
        name: "get_activities".to_owned(),
        response: json!({"activities": [{"id": 1}]}),
    };
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(
        wire,
        json!({"name": "get_activities", "response": {"activities": [{"id": 1}]}})
    );
    let back: FunctionResponse = serde_json::from_value(wire).unwrap();
    assert_eq!(back.name, "get_activities");
    assert_eq!(back.response, json!({"activities": [{"id": 1}]}));
}
