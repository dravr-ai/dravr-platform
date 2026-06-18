// ABOUTME: Conformance tests asserting advertised MCP resources/prompts capabilities return real content
// ABOUTME: Guards that initialize-advertised resources/prompts back non-empty list responses with usable read/get
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::{create_test_server_resources, create_test_user};
use pierre_core::models::TenantId;
use pierre_database::database::coaches::{
    CoachCategory, CoachVisibility, CoachesManager, CreateSystemCoachRequest,
};
use pierre_database::database::StoreListingsManager;
use pierre_mcp_schema::{McpRequest, McpResponse};
use pierre_mcp_server::mcp::host_seams::build_mcp_server;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Build an MCP request for the given method with optional params.
fn request(id: i64, method: &str, params: Option<Value>) -> McpRequest {
    McpRequest {
        jsonrpc: "2.0".to_owned(),
        method: method.to_owned(),
        params,
        id: Some(json!(id)),
        auth_token: None,
        headers: None,
        metadata: HashMap::new(),
    }
}

/// Publish a coach into the global marketplace so resources/list has content.
async fn publish_coach(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId, title: &str) {
    let sqlite_pool = resources.coach.database.sqlite_pool().unwrap().clone();
    let coaches_manager = CoachesManager::new(sqlite_pool.clone());
    let store_listings_manager = StoreListingsManager::new(sqlite_pool);

    let system_request = CreateSystemCoachRequest {
        title: title.to_owned(),
        description: Some(format!("Description for {title}")),
        system_prompt: format!("You are a {title} coach. Provide expert guidance."),
        category: CoachCategory::Training,
        tags: vec!["test".to_owned(), "training".to_owned()],
        visibility: CoachVisibility::Tenant,
        sample_prompts: vec!["How should I train this week?".to_owned()],
    };

    let coach = coaches_manager
        .create_system_coach(user_id, tenant_id, &system_request)
        .await
        .unwrap();

    store_listings_manager
        .submit_for_review(&coach.id.to_string(), user_id, tenant_id)
        .await
        .unwrap();
    store_listings_manager
        .approve_coach(&coach.id.to_string(), tenant_id, user_id)
        .await
        .unwrap();
}

/// Resolve the tenant id for a freshly created test user.
async fn tenant_for_user(resources: &ServerContext, user_id: Uuid) -> TenantId {
    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    tenants
        .first()
        .map_or_else(|| TenantId::from(user_id), |t| t.id)
}

/// Extract the result object from an MCP response, failing on an error response.
fn ok_result(response: McpResponse) -> Value {
    assert!(
        response.error.is_none(),
        "expected ok response, got error: {:?}",
        response.error
    );
    response.result.expect("ok response must carry a result")
}

#[tokio::test]
async fn test_advertised_resources_and_prompts_return_real_content() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database).await.unwrap();
    let tenant_id = tenant_for_user(&resources, user_id).await;

    publish_coach(&resources, user_id, tenant_id, "Marathon Coach").await;

    let server = build_mcp_server(Arc::clone(&resources));

    // initialize must advertise both resources and prompts capabilities.
    // A spec-conformant initialize request carries protocolVersion, clientInfo,
    // and capabilities; the handler rejects a paramless request with -32602.
    let init = ok_result(
        server
            .handle_request(request(
                1,
                "initialize",
                Some(json!({
                    "protocolVersion": "2025-11-25",
                    "clientInfo": { "name": "conformance-test", "version": "0.0.0" },
                    "capabilities": {}
                })),
            ))
            .await
            .expect("initialize returns a response"),
    );
    let capabilities = init
        .get("capabilities")
        .expect("initialize result has capabilities");
    assert!(
        capabilities.get("resources").is_some(),
        "resources capability must be advertised"
    );
    assert!(
        capabilities.get("prompts").is_some(),
        "prompts capability must be advertised"
    );

    // Capability honesty: subscribe/listChanged are false (not implemented).
    assert_eq!(capabilities["resources"]["subscribe"], json!(false));
    assert_eq!(capabilities["resources"]["listChanged"], json!(false));
    assert_eq!(capabilities["prompts"]["listChanged"], json!(false));

    // Advertised resources => resources/list returns >= 1 entry.
    let list = ok_result(
        server
            .handle_request(request(2, "resources/list", None))
            .await
            .expect("resources/list returns a response"),
    );
    let listed = list["resources"]
        .as_array()
        .expect("resources/list returns an array");
    assert!(
        !listed.is_empty(),
        "advertised resources capability must back a non-empty resources/list"
    );
    let first_uri = listed[0]["uri"].as_str().expect("resource entry has a uri");
    assert!(
        first_uri.starts_with("dravr://coaches/"),
        "coach resource uses dravr://coaches/ scheme, got {first_uri}"
    );
    assert_eq!(listed[0]["mimeType"], json!("text/markdown"));

    // resources/read returns the coach markdown for that uri.
    let read = ok_result(
        server
            .handle_request(request(
                3,
                "resources/read",
                Some(json!({ "uri": first_uri })),
            ))
            .await
            .expect("resources/read returns a response"),
    );
    let contents = read["contents"]
        .as_array()
        .expect("resources/read returns contents array");
    assert_eq!(contents.len(), 1);
    let text = contents[0]["text"].as_str().expect("content has text");
    assert!(
        text.contains("Marathon Coach"),
        "coach markdown should contain the coach title"
    );
    assert_eq!(contents[0]["mimeType"], json!("text/markdown"));

    // Advertised prompts => prompts/list returns >= 1 entry with arguments.
    let prompts = ok_result(
        server
            .handle_request(request(4, "prompts/list", None))
            .await
            .expect("prompts/list returns a response"),
    );
    let prompt_list = prompts["prompts"]
        .as_array()
        .expect("prompts/list returns an array");
    assert!(
        !prompt_list.is_empty(),
        "advertised prompts capability must back a non-empty prompts/list"
    );
    let race_readiness = prompt_list
        .iter()
        .find(|p| p["name"] == json!("race_readiness"))
        .expect("race_readiness template is advertised");
    let arguments = race_readiness["arguments"]
        .as_array()
        .expect("template declares arguments");
    assert!(
        arguments.iter().any(|a| a["name"] == json!("race_date")),
        "race_readiness declares a race_date argument"
    );

    // prompts/get assembles the prompt messages for supplied args.
    let assembled = ok_result(
        server
            .handle_request(request(
                5,
                "prompts/get",
                Some(json!({
                    "name": "race_readiness",
                    "arguments": { "race_date": "2026-09-20", "distance": "half marathon" }
                })),
            ))
            .await
            .expect("prompts/get returns a response"),
    );
    let messages = assembled["messages"]
        .as_array()
        .expect("prompts/get returns messages");
    assert_eq!(messages.len(), 1);
    let body = messages[0]["content"]["text"]
        .as_str()
        .expect("message has text content");
    assert!(
        body.contains("2026-09-20") && body.contains("half marathon"),
        "assembled prompt should interpolate supplied arguments"
    );
}

#[tokio::test]
async fn test_resources_read_rejects_unknown_uri() {
    let resources = create_test_server_resources().await.unwrap();
    let server = build_mcp_server(Arc::clone(&resources));

    let response = server
        .handle_request(request(
            1,
            "resources/read",
            Some(json!({ "uri": "dravr://coaches/00000000-0000-0000-0000-000000000001" })),
        ))
        .await
        .expect("resources/read returns a response");
    assert!(
        response.error.is_some(),
        "reading a missing coach resource must return an error"
    );
}

#[tokio::test]
async fn test_prompts_get_missing_required_argument_errors() {
    let resources = create_test_server_resources().await.unwrap();
    let server = build_mcp_server(Arc::clone(&resources));

    let response = server
        .handle_request(request(
            1,
            "prompts/get",
            Some(json!({ "name": "race_readiness", "arguments": {} })),
        ))
        .await
        .expect("prompts/get returns a response");
    assert!(
        response.error.is_some(),
        "missing required argument must return an error"
    );
}
