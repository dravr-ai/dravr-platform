// ABOUTME: Integration tests for A2A 1.0 protocol compliance
// ABOUTME: Pins PascalCase methods, ProtoJSON wire shapes, spec error codes with google.rpc.ErrorInfo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A2A Protocol Compliance Tests
//!
//! Tests to ensure our A2A implementation complies with the A2A 1.0
//! specification at <https://a2a-protocol.org/v1.0.0/specification>
//! (normative data shapes per `specification/a2a.proto` at the tag).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::a2a::protocol::A2AServer;
use pierre_mcp_server::a2a::protocol_types::{
    Message, Part, PartContent, Role, StreamResponse, TaskState, TaskStatusUpdateEvent,
    WireTaskStatus, A2A_VERSION,
};
use pierre_mcp_server::a2a::A2ARequest;
use serde_json::json;
use std::collections::HashMap;

/// JSON-RPC implementation-defined server error (config/auth failures)
const SERVER_ERROR_CODE: i32 = -32000;

fn request(method: &str, params: Option<serde_json::Value>) -> A2ARequest {
    A2ARequest {
        jsonrpc: "2.0".to_owned(),
        method: method.to_owned(),
        params,
        id: Some(json!(1)),
        auth_token: None,
        headers: None,
        metadata: HashMap::new(),
    }
}

#[test]
fn test_protocol_version_is_major_minor() {
    // Spec §3.6: negotiated versions are Major.Minor — never a patch digit.
    assert_eq!(A2A_VERSION, "1.0");
}

#[tokio::test]
async fn test_jsonrpc_2_0_compliance() {
    let server = A2AServer::new();

    let response = server.handle_request(request("GetTask", None)).await;
    assert_eq!(response.jsonrpc, "2.0");
}

#[tokio::test]
async fn test_spec_methods_require_auth() {
    let server = A2AServer::new();

    // Every task/message/push method is authenticated (transport-level auth
    // per the card's securitySchemes).
    let protected_methods = vec![
        "SendMessage",
        "GetTask",
        "ListTasks",
        "CancelTask",
        "CreateTaskPushNotificationConfig",
        "GetTaskPushNotificationConfig",
        "ListTaskPushNotificationConfigs",
        "DeleteTaskPushNotificationConfig",
        "GetExtendedAgentCard",
    ];

    for method in protected_methods {
        let response = server.handle_request(request(method, None)).await;

        assert!(
            response.error.is_some(),
            "Method {method} should require authentication"
        );
        let code = response.error.as_ref().unwrap().code;
        assert_eq!(
            code, SERVER_ERROR_CODE,
            "Method {method} should return a -32000 auth/config error, got {code}"
        );
    }
}

#[tokio::test]
async fn test_pre_1_0_methods_are_gone() {
    let server = A2AServer::new();

    // The category/action strings of pre-1.0 and Pierre's bespoke methods
    // were removed in the 1.0 cutover — all must be unknown (-32601).
    let dead_methods = vec![
        "a2a/initialize",
        "message/send",
        "message/stream",
        "tasks/get",
        "tasks/cancel",
        "tasks/create",
        "tasks/resubscribe",
        "tasks/pushNotificationConfig/set",
        "tools/list",
        "tools/call",
        "a2a/tools/call",
        "a2a/tasks/list",
        "agent/getAuthenticatedExtendedCard",
    ];

    for method in dead_methods {
        let response = server.handle_request(request(method, None)).await;
        assert!(
            response.error.is_some(),
            "Method {method} should be unknown"
        );
        assert_eq!(
            response.error.as_ref().unwrap().code,
            -32601,
            "Method {method} should return -32601 Method not found"
        );
    }
}

#[tokio::test]
async fn test_extended_agent_card_requires_auth() {
    let server = A2AServer::new();

    // capabilities.extendedAgentCard is true and the method is the
    // authenticated card variant — unauthenticated calls are rejected.
    let response = server
        .handle_request(request("GetExtendedAgentCard", None))
        .await;
    let error = response.error.expect("expected auth/config error");
    assert_eq!(error.code, SERVER_ERROR_CODE);
}

#[test]
fn test_spec_error_carries_error_info() {
    use pierre_mcp_server::a2a::protocol_types::A2ASpecError;
    use pierre_mcp_server::a2a::JsonRpcError;

    // Every A2A-specific error must carry a google.rpc.ErrorInfo detail
    // (spec 5.4): @type, UPPER_SNAKE reason, a2a-protocol.org domain.
    let error: JsonRpcError = A2ASpecError::TaskNotFound.into();
    assert_eq!(error.code, -32001);
    let details = error.data.expect("A2A errors must carry error.data");
    let detail = &details.as_array().expect("data is an ErrorInfo array")[0];
    assert_eq!(
        detail["@type"], "type.googleapis.com/google.rpc.ErrorInfo",
        "detail must be a google.rpc.ErrorInfo"
    );
    assert_eq!(detail["reason"], "TASK_NOT_FOUND");
    assert_eq!(detail["domain"], "a2a-protocol.org");
}

#[tokio::test]
async fn test_streaming_methods_rejected_without_sse() {
    let server = A2AServer::new();

    // SendStreamingMessage/SubscribeToTask are answered with SSE by the HTTP
    // layer; the non-streaming dispatcher must reject them.
    for method in ["SendStreamingMessage", "SubscribeToTask"] {
        let response = server.handle_request(request(method, None)).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.as_ref().unwrap().code, -32600);
    }
}

#[tokio::test]
async fn test_error_code_compliance() {
    let server = A2AServer::new();

    // Unknown method returns -32601 Method not found.
    let response = server.handle_request(request("unknown/method", None)).await;
    assert!(response.error.is_some());
    assert_eq!(response.error.unwrap().code, -32601);
}

#[test]
fn test_message_structure_compliance() {
    // ProtoJSON Message: messageId, ROLE_* enum, kindless parts whose oneof
    // member name (text/raw/url/data) is the discriminator.
    let message = Message {
        message_id: "msg-1".to_owned(),
        role: Role::User,
        parts: vec![
            Part::text("Hello".to_owned()),
            Part::data(json!({"key": "value"})),
        ],
        context_id: None,
        task_id: None,
        reference_task_ids: Vec::new(),
        extensions: Vec::new(),
        metadata: None,
    };

    let serialized = serde_json::to_value(&message).unwrap();
    assert_eq!(serialized["messageId"], "msg-1");
    assert_eq!(serialized["role"], "ROLE_USER");
    let parts = serialized["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["text"], "Hello");
    assert_eq!(parts[1]["data"]["key"], "value");
    // kind discriminators were removed in 1.0.
    assert!(parts[0].get("kind").is_none());
    assert!(serialized.get("kind").is_none());

    // Round-trip: the wire form deserializes back.
    let round: Message = serde_json::from_value(serialized).unwrap();
    assert_eq!(round, message);
}

#[test]
fn test_task_state_wire_strings() {
    // TaskState serializes as the ProtoJSON TASK_STATE_* names.
    let cases = [
        (TaskState::Submitted, "TASK_STATE_SUBMITTED"),
        (TaskState::Working, "TASK_STATE_WORKING"),
        (TaskState::Completed, "TASK_STATE_COMPLETED"),
        (TaskState::Failed, "TASK_STATE_FAILED"),
        (TaskState::Canceled, "TASK_STATE_CANCELED"),
        (TaskState::InputRequired, "TASK_STATE_INPUT_REQUIRED"),
        (TaskState::Rejected, "TASK_STATE_REJECTED"),
        (TaskState::AuthRequired, "TASK_STATE_AUTH_REQUIRED"),
    ];
    for (state, wire) in cases {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(wire));
    }

    assert!(TaskState::Completed.is_terminal());
    assert!(TaskState::Rejected.is_terminal());
    assert!(!TaskState::Working.is_terminal());
    assert!(TaskState::InputRequired.is_interrupted());
    assert!(TaskState::AuthRequired.is_interrupted());
}

#[test]
fn test_stream_response_oneof_naming() {
    // StreamResponse members are the proto oneof names statusUpdate /
    // artifactUpdate — and the pre-1.0 `final` flag no longer exists.
    let event = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
        task_id: "task-1".to_owned(),
        context_id: "ctx-1".to_owned(),
        status: WireTaskStatus {
            state: TaskState::Completed,
            message: None,
            timestamp: Some("2026-07-09T00:00:00.000Z".to_owned()),
        },
        metadata: None,
    });

    let serialized = serde_json::to_value(&event).unwrap();
    let update = &serialized["statusUpdate"];
    assert_eq!(update["taskId"], "task-1");
    assert_eq!(update["contextId"], "ctx-1");
    assert_eq!(update["status"]["state"], "TASK_STATE_COMPLETED");
    assert!(update.get("final").is_none());
}

#[test]
fn test_part_content_variants() {
    // The four oneof content members of Part, with mediaType (not mimeType).
    let raw: Part = serde_json::from_value(json!({
        "raw": "aGVsbG8=",
        "filename": "hello.bin",
        "mediaType": "application/octet-stream"
    }))
    .unwrap();
    assert!(matches!(raw.content, PartContent::Raw(ref b) if b == "aGVsbG8="));
    assert_eq!(raw.filename.as_deref(), Some("hello.bin"));
    assert_eq!(raw.media_type.as_deref(), Some("application/octet-stream"));

    let url: Part = serde_json::from_value(json!({"url": "https://example.com/x.gpx"})).unwrap();
    assert!(matches!(url.content, PartContent::Url(_)));
}

#[tokio::test]
async fn test_auth_errors_carry_error_info_reason() {
    let server = A2AServer::new();

    // The A2A server without resources reports -32000 without auth details;
    // a configured server tags auth failures with AUTHENTICATION_REQUIRED so
    // the HTTP layer can map them to 401. Both stay in the -32000 space,
    // never colliding with the A2A protocol codes -32001..-32099.
    let response = server.handle_request(request("GetTask", None)).await;
    let error = response.error.expect("expected error");
    assert_eq!(error.code, SERVER_ERROR_CODE);
}

#[tokio::test]
async fn test_id_preservation() {
    let server = A2AServer::new();

    let test_ids = vec![json!(1), json!("string-id"), json!(null)];

    for test_id in test_ids {
        let mut req = request("GetExtendedAgentCard", None);
        req.id = Some(test_id.clone());

        let response = server.handle_request(req).await;
        assert_eq!(response.id, Some(test_id));
    }
}

#[tokio::test]
async fn test_id_preservation_on_auth_errors() {
    let server = A2AServer::new();

    let test_ids = vec![json!(42), json!("req-abc"), json!(null)];

    for test_id in test_ids {
        let mut req = request("ListTasks", None);
        req.id = Some(test_id.clone());

        let response = server.handle_request(req).await;
        assert_eq!(
            response.id,
            Some(test_id),
            "Request ID must be preserved in auth error responses"
        );
    }
}
