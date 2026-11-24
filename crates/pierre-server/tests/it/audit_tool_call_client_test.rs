// ABOUTME: Tests the tool-call audit span names the client by the OAuth client_id its token was issued to
// ABOUTME: Drives /mcp's auth hook and engine; a spoofed clientInfo is never recorded, an API key names no client
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The 2026-07-28 audit event names the client a tool call came through. That
//! client is the OAuth `client_id` the authorization server assigned at
//! registration and checked when it issued the token (RFC 9068 §2.2) — never
//! what the caller says about itself. Every request here carries a modern
//! `_meta` `clientInfo` naming a client of its own choosing, so a dispatch path
//! that recorded it would put that name in the trail and fail these tests.

use crate::common;

use std::collections::HashMap;
use std::sync::Arc;

use dravr_tronc::mcp::protocol::JsonRpcRequest;
use pierre_auth::auth::DelegatedGrant;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::host_seams::build_mcp_server;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_test_support::log_capture::{self, CapturedLog};
use serde_json::{json, Value};
use tracing::subscriber::DefaultGuard;
use uuid::Uuid;

/// The client the authorization server registered and issued the token to.
const REGISTERED_CLIENT: &str = "mcp_client_audit";
/// What the request's own `clientInfo` claims to be.
const SPOOFED_CLIENT: &str = "mcp_client_spoofed";
/// A tool every tenant member may call over `/mcp`, cheap enough to dispatch
/// inline.
const TOOL: &str = "get_connection_status";

// ── Span capture ─────────────────────────────────────────────────────────────

/// Every span opened on this thread while the guard lives. The tests run on
/// tokio's current-thread runtime, so the inline dispatch stays on it.
struct Captured {
    log: CapturedLog,
    _guard: DefaultGuard,
}

impl Captured {
    fn start() -> Self {
        let (log, guard) = log_capture::capture_events_and_spans();
        Self { log, _guard: guard }
    }

    /// The fields of the one `dispatch_tool_call` span: the audit event.
    fn audit_span(&self) -> HashMap<String, String> {
        let mut audit = self
            .log
            .spans()
            .into_iter()
            .filter(|span| span.name == "dispatch_tool_call")
            .map(|span| span.fields);
        let fields = audit.next().expect("the tool call opened its audit span");
        assert!(audit.next().is_none(), "one tool call, one audit span");
        fields
    }

    /// Whether any field of any span holds `needle`.
    fn any_field_holds(&self, needle: &str) -> bool {
        self.log
            .spans()
            .iter()
            .flat_map(|span| span.fields.values())
            .any(|value| value.contains(needle))
    }
}

// ── Fixture ──────────────────────────────────────────────────────────────────

/// A modern `tools/call` whose `_meta` `clientInfo` claims [`SPOOFED_CLIENT`],
/// presented with `credential`.
fn tool_call(credential: String) -> JsonRpcRequest {
    let mut request: JsonRpcRequest = serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": TOOL,
            "arguments": {},
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": { "name": SPOOFED_CLIENT, "version": "1" },
                "io.modelcontextprotocol/clientCapabilities": {},
            },
        },
    }))
    .unwrap();
    request.auth_token = Some(credential);
    request
}

/// Authenticate `request` through the platform's auth hook, then dispatch it
/// through the engine under the context the hook resolved — what the HTTP
/// transport does.
async fn dispatch(resources: &Arc<ServerContext>, request: JsonRpcRequest) -> Value {
    let server = build_mcp_server(resources.clone());
    let ctx = server.authenticate(&request).await.expect("authenticated");
    let response = server
        .handle_request_with_context(request, &ctx)
        .await
        .expect("a request with an id is answered");
    serde_json::to_value(response).unwrap()
}

/// A tenant member and the OAuth access token [`REGISTERED_CLIENT`] holds for
/// them, carrying every delegable scope.
async fn member_with_oauth_token(resources: &Arc<ServerContext>) -> (Uuid, String) {
    let (user, _) = common::create_test_tenant(resources, "audit-oauth@example.com")
        .await
        .unwrap();
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    let scopes: Vec<String> = OAuthScope::delegable_as_str()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let token = resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: REGISTERED_CLIENT,
                scopes: &scopes,
                audience: None,
            },
            &user.id,
            tenants.first().map(|tenant| tenant.id.to_string()),
        )
        .unwrap();
    (user.id, token)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_oauth_call_is_audited_under_the_client_its_token_was_issued_to() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, token) = member_with_oauth_token(&resources).await;

    let captured = Captured::start();
    let response = dispatch(&resources, tool_call(token)).await;

    assert!(
        response["error"].is_null() && response["result"]["isError"] != true,
        "the call is served: {response}"
    );
    let audit = captured.audit_span();
    assert_eq!(
        audit.get("client").map(String::as_str),
        Some(REGISTERED_CLIENT)
    );
    assert_eq!(audit.get("tool_name").map(String::as_str), Some(TOOL));
    assert_eq!(audit.get("user_id"), Some(&user_id.to_string()));
    assert!(audit.contains_key("arguments_hash"));
    assert!(
        !captured.any_field_holds(SPOOFED_CLIENT),
        "the request's own clientInfo reached the trail: {audit:?}"
    );
}

#[tokio::test]
async fn an_api_key_call_is_audited_with_no_client() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let (user, _) = common::create_test_tenant(&resources, "audit-key@example.com")
        .await
        .unwrap();
    let api_key = common::issue_test_api_key(&resources.agent.database, user.id, "audit key")
        .await
        .unwrap();

    let captured = Captured::start();
    let response = dispatch(&resources, tool_call(api_key)).await;

    assert!(
        response["error"].is_null() && response["result"]["isError"] != true,
        "the call is served: {response}"
    );
    let audit = captured.audit_span();
    assert_eq!(audit.get("tool_name").map(String::as_str), Some(TOOL));
    // An API key is the athlete's own credential, issued to no registered
    // client: the field stays empty rather than borrowing the request's word.
    assert_eq!(audit.get("client"), None, "{audit:?}");
    assert!(
        !captured.any_field_holds(SPOOFED_CLIENT),
        "the request's own clientInfo reached the trail: {audit:?}"
    );
}
