// ABOUTME: End-to-end Guardian enforcement through the real ServerContext + UniversalExecutor chokepoint.
// ABOUTME: Deterministic (no LLM): zero destructive budget denies an IRREVERSIBLE tool, over /mcp too; reads pass.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Guardian enforcement e2e.
//!
//! Drives a real `ServerContext`-backed `UniversalExecutor` (the one true
//! dispatch chokepoint every transport funnels through) in `enforce` mode and
//! asserts the runtime Guardian blocks a consequential tool while leaving reads
//! untouched, then drives the same block through `POST /mcp` to pin what an MCP
//! client receives. Single test fn so the process-wide Guardian policy
//! singleton initializes once, after the env is set.

mod common;

use pierre_core::permissions::scopes::OAuthScope;
use std::env;

use axum::body::{to_bytes, Body};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Request, StatusCode};
use common::{create_test_server_resources, create_test_user, generate_test_token};
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn request(tool: &str, args: Value, user_id: Uuid, tenant: TenantId) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: args,
        user_id: user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

/// `POST /mcp` `tools/call` of `tool` with the caller's bearer `token`, through
/// the app the server serves, and the JSON-RPC body it answers.
async fn tools_call_over_http(
    resources: &Arc<ServerContext>,
    token: &str,
    tool: &str,
    args: Value,
) -> Value {
    let request = Request::post("/mcp")
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": args }
            })
            .to_string(),
        ))
        .unwrap();
    let response = ProviderToolRouter::build_http_app(resources)
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a refused tools/call is answered in-band, not as an HTTP error"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn guardian_error_code(response: &UniversalResponse) -> Option<String> {
    response
        .result
        .as_ref()
        .and_then(|r| r.get("error_code"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[tokio::test]
async fn guardian_enforce_chokepoint_denies_irreversible_allows_reads_e2e() {
    // Arrange enforce mode with a zero destructive budget BEFORE the server
    // resources are created — the guardian config registry captures the
    // GUARDIAN_* env overrides once at construction.
    env::set_var("GUARDIAN_MODE", "enforce");
    env::set_var("GUARDIAN_MAX_DESTRUCTIVE_PER_TURN", "0");

    common::init_server_config();
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    // One executor = one turn; its turn token is the Guardian's per-turn key.
    let executor = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_turn_token("e2e-turn".to_owned());

    // (a) An IRREVERSIBLE tool is denied at the chokepoint by the zero budget —
    //     before the tool body ever runs. `disconnect_provider` is classified
    //     IRREVERSIBLE in the registry.
    let deny = executor
        .execute_tool(request(
            "disconnect_provider",
            json!({ "provider": "strava" }),
            user_id,
            tenant,
        ))
        .await
        .expect("dispatch returns an in-band response");
    assert!(
        !deny.success,
        "an irreversible tool must be denied under enforce + zero destructive budget"
    );
    assert_eq!(
        guardian_error_code(&deny).as_deref(),
        Some("guardian_denied"),
        "denied response must carry the guardian_denied machine code"
    );
    assert_eq!(
        deny.result
            .as_ref()
            .and_then(|r| r.get("reason"))
            .and_then(Value::as_str),
        Some("budget_exceeded"),
        "the denial reason is the per-turn budget"
    );

    // (b) A read tool is NOT guardian-denied — enforce must not break ordinary
    //     reads. (It may still fail for lack of provider data; that's fine, as
    //     long as it is not a Guardian denial.)
    let read = executor
        .execute_tool(request(
            "get_athlete",
            json!({ "provider": "strava" }),
            user_id,
            tenant,
        ))
        .await
        .expect("dispatch returns an in-band response");
    assert_ne!(
        guardian_error_code(&read).as_deref(),
        Some("guardian_denied"),
        "a read tool must never be guardian-denied"
    );

    // (c) The same block reaches an MCP client over `POST /mcp` as an `isError`
    //     result whose data rides in `content`: the message, then the Guardian's
    //     code and reason as JSON. Never as `structuredContent` —
    //     `disconnect_provider` declares an `outputSchema` this payload does not
    //     match, and a client that validates `structuredContent` on an error
    //     result (the official TypeScript SDK does) would reject the refusal
    //     instead of reading it. A `/mcp` call is its own Guardian turn, so the
    //     zero budget denies it on the first call.
    let token = generate_test_token(&resources, &user).await;
    let body = tools_call_over_http(
        &resources,
        &token,
        "disconnect_provider",
        json!({ "provider": "strava" }),
    )
    .await;
    assert!(
        body.get("error").is_none(),
        "a Guardian block is an isError result, not a JSON-RPC error: {body}"
    );
    let result = &body["result"];
    assert_eq!(result["isError"], true, "the block is an error: {body}");
    assert!(
        result.get("structuredContent").is_none(),
        "an error result carries no structuredContent: {body}"
    );
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "message, then the data as JSON: {body}");
    assert_eq!(
        content[0]["text"],
        "Error: Tool 'disconnect_provider' was blocked by security policy"
    );
    let data: Value = serde_json::from_str(content[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        data,
        json!({ "error_code": "guardian_denied", "reason": "budget_exceeded" }),
        "the client reads the Guardian's code and reason from content: {body}"
    );
}
