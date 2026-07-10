// ABOUTME: Integration tests for protocol converter functionality
// ABOUTME: Tests MCP/universal conversion and protocol detection (incl. A2A 1.0 method names)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_schema::{Content, ToolCall};
use pierre_tool_runtime::protocols::converter::ProtocolConverter;
use pierre_tool_runtime::protocols::ProtocolType;
use pierre_tool_runtime::protocols::UniversalResponse;

#[test]
fn test_mcp_to_universal_conversion() {
    let mcp_call = ToolCall {
        name: "get_activities".into(),
        arguments: Some(serde_json::json!({"limit": 5})),
    };

    let universal = ProtocolConverter::mcp_to_universal(mcp_call, "test_user", None);

    assert_eq!(universal.tool_name, "get_activities");
    assert_eq!(universal.user_id, "test_user");
    assert_eq!(universal.protocol, "mcp");
    assert_eq!(
        universal.parameters.get("limit").unwrap().as_u64().unwrap(),
        5
    );
}

#[test]
fn test_universal_to_mcp_conversion_success() {
    let universal_response = UniversalResponse {
        success: true,
        result: Some(serde_json::json!({"data": "test"})),
        error: None,
        metadata: None,
    };

    let mcp_response = ProtocolConverter::universal_to_mcp(universal_response);

    assert!(!mcp_response.is_error);
    assert_eq!(mcp_response.content.len(), 1);
    match &mcp_response.content[0] {
        Content::Text { text } => {
            assert!(text.contains("\"data\""));
            assert!(text.contains("\"test\""));
        }
        Content::Image { .. } => {
            panic!("Expected text content, got image");
        }
        Content::Resource { .. } => {
            panic!("Expected text content, got resource");
        }
        Content::Progress { .. } => {
            panic!("Expected text content, got progress");
        }
    }
}

#[test]
fn test_universal_to_mcp_conversion_error() {
    let universal_response = UniversalResponse {
        success: false,
        result: None,
        error: Some("Invalid parameters".into()),
        metadata: None,
    };

    let mcp_response = ProtocolConverter::universal_to_mcp(universal_response);

    assert!(mcp_response.is_error);
    assert_eq!(mcp_response.content.len(), 1);
    match &mcp_response.content[0] {
        Content::Text { text } => {
            assert!(text.contains("Invalid parameters"));
        }
        Content::Image { .. } => {
            panic!("Expected text content, got image");
        }
        Content::Resource { .. } => {
            panic!("Expected text content, got resource");
        }
        Content::Progress { .. } => {
            panic!("Expected text content, got progress");
        }
    }
}

#[test]
fn test_detect_protocol_a2a() {
    let a2a_request = r#"{"jsonrpc": "2.0", "method": "SendMessage", "id": 1}"#;
    let protocol = ProtocolConverter::detect_protocol(a2a_request).unwrap();
    assert_eq!(protocol, ProtocolType::A2A);

    let a2a_subscribe = r#"{"jsonrpc": "2.0", "method": "SubscribeToTask", "id": 2}"#;
    let protocol = ProtocolConverter::detect_protocol(a2a_subscribe).unwrap();
    assert_eq!(protocol, ProtocolType::A2A);
}

#[test]
fn test_detect_protocol_mcp() {
    let mcp_request = r#"{"method": "tools/call", "params": {}}"#;
    let protocol = ProtocolConverter::detect_protocol(mcp_request).unwrap();
    assert_eq!(protocol, ProtocolType::MCP);
}

#[test]
fn test_detect_protocol_unknown() {
    let unknown_request = r#"{"some": "data"}"#;
    let result = ProtocolConverter::detect_protocol(unknown_request);
    assert!(result.is_err());
}
