// ABOUTME: What an MCP or A2A caller reads when the athlete has no provider connected, and what Dravr's own chat still reads
// ABOUTME: Drives POST /mcp with a real delegated token; the provider list comes from the registry, never from this file

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # The no-provider answer over `/mcp`
//!
//! A new athlete with nothing connected asks for activities through an MCP
//! client. The call must not fail, and it must not promise that connecting a
//! first-party-only provider makes its data visible there. The answer is the
//! deployment's own provider list, each marked readable on this interface or
//! not; Dravr's web, mobile and messaging chat keep the wording and the
//! signal their pipeline depends on.

use crate::common::{create_test_server_resources, create_test_tenant};

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_auth::auth::DelegatedGrant;
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::backend_resolver::user_facing_name;
use pierre_tool_runtime::protocol::provider_helpers::NO_PROVIDER_CONNECTED_TEXT;
use pierre_tool_runtime::protocol::{
    is_no_provider_refusal, UniversalExecutor, UniversalRequest, UniversalResponse,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::scopes::required_scopes;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

/// A delegated token for `user_id` holding exactly `scopes`.
fn token_with_scopes(
    res: &ServerContext,
    user_id: Uuid,
    tenant_id: &str,
    scopes: &[&str],
) -> String {
    let scopes: Vec<String> = scopes.iter().map(|s| (*s).to_owned()).collect();
    let token = res
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &res.auth.jwks_manager,
            &DelegatedGrant {
                client_id: "mcp_client_test",
                scopes: &scopes,
                audience: None,
            },
            &user_id,
            &[],
            Some(tenant_id.to_owned()),
        )
        .unwrap();
    format!("Bearer {token}")
}

async fn post_mcp(app: &Router, authorization: &str, body: Value) -> Value {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("authorization", authorization)
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

fn call(name: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
}

/// A user with no provider connection, in their own tenant.
async fn new_athlete(res: &Arc<ServerContext>, email: &str) -> (Uuid, String) {
    let (user, _jwt) = create_test_tenant(res, email).await.unwrap();
    let tenants = res
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    (user.id, tenants.first().unwrap().id.to_string())
}

/// Every provider name the registry knows, in every spelling a client could
/// have been told: the user-facing names, the backends and the display names.
fn provider_literals(res: &ServerContext) -> BTreeSet<String> {
    let registry = res.provider_registry();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for supported in registry.supported_providers() {
        names.insert(supported.to_ascii_lowercase());
        names.insert(user_facing_name(supported).to_ascii_lowercase());
    }
    for display in registry.first_party_only_sources() {
        names.insert(display.to_ascii_lowercase());
    }
    names.retain(|name| name.len() > 2);
    names
}

#[tokio::test]
async fn an_mcp_caller_with_nothing_connected_gets_the_offered_providers_not_a_failure() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = new_athlete(&res, "new-athlete@no-provider.test").await;
    let app = ProviderToolRouter::build_http_app(&res);
    let grant = token_with_scopes(&res, user_id, &tenant, &["fitness:read", "profile:read"]);

    let answered = post_mcp(&app, &grant, call("get_activities", &json!({}))).await;
    let result = &answered["result"];
    assert_eq!(
        result["isError"], false,
        "bug: having nothing connected is not a failure: {answered}"
    );

    let structured = &result["structuredContent"];
    assert_eq!(structured["connected"], false, "{answered}");
    let offered: Vec<&Value> = structured["providers"].as_array().unwrap().iter().collect();
    let registry = res.provider_registry();
    let expected: BTreeSet<&str> = registry
        .supported_providers()
        .into_iter()
        .map(user_facing_name)
        .collect();
    let listed: BTreeSet<&str> = offered
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(listed, expected, "every provider the deployment offers");

    // Each marks whether its data can be read here, from the registry's terms.
    let mut unreadable = 0;
    for entry in &offered {
        let name = entry["name"].as_str().unwrap();
        let first_party_only = registry
            .transport_policy(name)
            .is_some_and(TransportPolicy::is_first_party_only);
        assert_eq!(entry["readable_here"], !first_party_only, "{name}");
        assert_eq!(entry["status"], "disconnected", "{name}");
        if first_party_only {
            unreadable += 1;
            assert!(
                entry["note"]
                    .as_str()
                    .unwrap()
                    .contains("only in the Dravr app"),
                "{name}"
            );
        }
    }
    assert!(
        unreadable > 0,
        "the shipped providers include first-party-only ones"
    );

    // The generic text names no provider; the list is the data beside it.
    let message = structured["message"].as_str().unwrap().to_ascii_lowercase();
    for literal in provider_literals(&res) {
        assert!(
            !message.contains(&literal),
            "bug: the no-provider text hard-codes '{literal}': {message}"
        );
    }
    assert!(message.contains("is the athlete's choice"), "{message}");
}

#[tokio::test]
async fn connect_provider_with_no_provider_named_starts_nothing_and_lists_the_choices() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = new_athlete(&res, "choosing@no-provider.test").await;
    let app = ProviderToolRouter::build_http_app(&res);
    let grant = token_with_scopes(
        &res,
        user_id,
        &tenant,
        OAuthScope::delegable_as_str().as_slice(),
    );

    let answered = post_mcp(&app, &grant, call("connect_provider", &json!({}))).await;
    let text = answered["result"]["content"].to_string();
    assert!(text.contains("nothing was started"), "{answered}");
    assert!(
        !text.contains("authorization_url"),
        "no flow started: {answered}"
    );

    let registry = res.provider_registry();
    for name in registry
        .supported_providers()
        .into_iter()
        .map(user_facing_name)
    {
        assert!(text.contains(name), "{name} is offered: {answered}");
    }
    assert!(
        text.contains("readable only in the Dravr app"),
        "first-party-only providers say so: {answered}"
    );
}

fn request_for(user_id: Uuid, tenant: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: "get_activities".to_owned(),
        parameters: json!({}),
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant.to_owned()),
    }
}

async fn dispatch_over(
    res: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: &str,
    transport: Option<Transport>,
) -> UniversalResponse {
    let runtime: Arc<dyn ToolRuntime> = res.clone();
    let mut executor = UniversalExecutor::new(runtime).with_scopes(OAuthScope::self_grant());
    if let Some(transport) = transport {
        executor = executor.with_transport(transport);
    }
    executor
        .execute_tool(request_for(user_id, tenant))
        .await
        .expect("an in-band answer")
}

#[tokio::test]
async fn dravrs_own_chat_keeps_its_wording_and_its_signal() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = new_athlete(&res, "chat@no-provider.test").await;

    for transport in [
        Some(Transport::WebApp),
        Some(Transport::MobileApp),
        Some(Transport::Messaging),
        None,
    ] {
        let response = dispatch_over(&res, user_id, &tenant, transport).await;
        assert!(!response.success, "{transport:?}: {response:?}");
        assert!(is_no_provider_refusal(&response), "{transport:?}");
        assert_eq!(
            response.error.as_deref(),
            Some(NO_PROVIDER_CONNECTED_TEXT),
            "{transport:?}: the first-party wording is exactly the pipeline's"
        );
    }
}

#[tokio::test]
async fn an_a2a_caller_gets_the_same_ordinary_answer_as_mcp() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = new_athlete(&res, "agent@no-provider.test").await;

    for transport in [Transport::A2a, Transport::McpHttp, Transport::ApiKey] {
        let response = dispatch_over(&res, user_id, &tenant, Some(transport)).await;
        assert!(response.success, "{transport:?}: {response:?}");
        assert!(
            is_no_provider_refusal(&response),
            "the in-process signal survives"
        );
        assert!(response.error.is_none());
    }
}

#[tokio::test]
async fn tools_list_offers_only_what_the_grant_covers() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = new_athlete(&res, "scoped@no-provider.test").await;
    let app = ProviderToolRouter::build_http_app(&res);
    let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });

    let read_only = token_with_scopes(&res, user_id, &tenant, &["fitness:read", "profile:read"]);
    let full = token_with_scopes(
        &res,
        user_id,
        &tenant,
        OAuthScope::delegable_as_str().as_slice(),
    );
    let names = |answered: &Value| -> BTreeSet<String> {
        answered["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let read_listed = names(&post_mcp(&app, &read_only, list.clone()).await);
    let full_listed = names(&post_mcp(&app, &full, list).await);

    assert!(read_listed.contains("get_activities"));
    let granted = [OAuthScope::FitnessRead, OAuthScope::ProfileRead];
    for name in &read_listed {
        let tool = res.mcp.tool_registry.get(name).unwrap();
        for needed in required_scopes(tool.capabilities()) {
            assert!(
                granted.contains(&needed),
                "bug: '{name}' needs {needed} and is listed to a read-only grant"
            );
        }
    }
    for hidden in ["connect_provider", "disconnect_provider"] {
        assert!(!read_listed.contains(hidden), "{hidden} writes the profile");
        assert!(
            full_listed.contains(hidden),
            "{hidden} is listed with profile:write"
        );
    }
    assert!(
        read_listed.len() < full_listed.len(),
        "the write tools are what the read-only grant lacks"
    );
}

/// A client validates an ordinary result against the tool's declared output and
/// fails the call on a mismatch, so the no-provider answer has to satisfy the
/// schema `tools/list` declares for every tool that can give it.
#[tokio::test]
async fn the_no_provider_answer_satisfies_each_tools_declared_output() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = new_athlete(&res, "schema@no-provider.test").await;
    let app = ProviderToolRouter::build_http_app(&res);
    let grant = token_with_scopes(&res, user_id, &tenant, &["fitness:read", "profile:read"]);

    let answered = post_mcp(&app, &grant, call("get_activities", &json!({}))).await;
    let structured = answered["result"]["structuredContent"].clone();
    assert_eq!(structured["connected"], false, "{answered}");

    let listed = post_mcp(
        &app,
        &grant,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    )
    .await;
    let mut checked = 0;
    for tool in listed["result"]["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        let requires_provider = res.mcp.tool_registry.get(name).is_some_and(|t| {
            t.capabilities()
                .contains(ToolCapabilities::REQUIRES_PROVIDER)
        });
        let Some(schema) = tool.get("outputSchema") else {
            continue;
        };
        if !requires_provider {
            continue;
        }
        let validator = jsonschema::validator_for(schema).unwrap();
        assert!(
            validator.is_valid(&structured),
            "bug: a strict client fails '{name}' on the no-provider answer"
        );
        checked += 1;
    }
    assert!(checked > 0, "no provider-requiring tool declares an output");
}

/// The 401 asks for a read-only grant, which does not list `connect_provider`,
/// so that grant's answer must not send the model to a tool it was not offered.
#[tokio::test]
async fn a_read_only_grant_is_not_sent_to_a_tool_it_was_not_offered() {
    let res = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = new_athlete(&res, "route@no-provider.test").await;
    let app = ProviderToolRouter::build_http_app(&res);

    let read_only = token_with_scopes(&res, user_id, &tenant, &["fitness:read", "profile:read"]);
    let answered = post_mcp(&app, &read_only, call("get_activities", &json!({}))).await;
    let message = answered["result"]["structuredContent"]["message"]
        .as_str()
        .unwrap();
    assert!(!message.contains("connect_provider"), "{message}");
    assert!(message.contains("connects a provider"), "{message}");

    let full = token_with_scopes(
        &res,
        user_id,
        &tenant,
        OAuthScope::delegable_as_str().as_slice(),
    );
    let answered = post_mcp(&app, &full, call("get_activities", &json!({}))).await;
    let message = answered["result"]["structuredContent"]["message"]
        .as_str()
        .unwrap();
    assert!(message.contains("connect_provider"), "{message}");
}
