// ABOUTME: The LLM chain a server is handed in its config is the one its request paths use
// ABOUTME: Admin settings, a new conversation's model and the turn's provider follow ServerConfig::llm_chain, never the env

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `PIERRE_LLM_CHAIN` is read once, at boot, into `ServerConfig::llm_chain`.
//! Each test here builds a server on a chain of its own — one no environment
//! sets — and reads it back out of a request path, so a path that went back to
//! the environment would answer with some other chain and fail.

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::common::{
    create_test_server_resources_with_chat_provider_and_config,
    create_test_server_resources_with_config, create_test_user_with_plan, generate_test_token,
    test_server_config,
};
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::recording_llm::RecordingProvider;
use pierre_config::environment::ServerConfig;
use pierre_llm::config::LlmChain;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, ConversationResponse};
use pierre_routes_admin::llm_settings::LlmSettingsRoutes;

/// A model id no deployment and no developer chain names.
const CHAIN_MODEL: &str = "chain-config-test-model";

fn config_with_chain(chain: &str) -> ServerConfig {
    ServerConfig {
        llm_chain: LlmChain::parse(chain),
        ..test_server_config()
    }
}

async fn bearer_for(resources: &Arc<ServerContext>, email: &str) -> String {
    let (_user_id, user, _tenant) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    format!("Bearer {}", generate_test_token(resources, &user).await)
}

async fn llm_settings(resources: &Arc<ServerContext>, auth: &str) -> Value {
    let resp = AxumTestRequest::get("/api/user/llm-settings")
        .header("authorization", auth)
        .send(LlmSettingsRoutes::routes(Arc::clone(resources)))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json()
}

#[tokio::test]
async fn admin_settings_name_the_head_of_the_chain_the_server_was_given() {
    let resources = create_test_server_resources_with_config(config_with_chain(&format!(
        "copilot_sdk={CHAIN_MODEL},gemini=gemini-flash-latest"
    )))
    .await
    .unwrap();
    let auth = bearer_for(&resources, "chain-settings@test.com").await;

    let body = llm_settings(&resources, &auth).await;
    let system = &body["system_provider"];
    assert_eq!(system["name"], "copilot_sdk", "{body}");
    assert_eq!(system["display_name"], "Copilot SDK (Embacle)", "{body}");
    assert_eq!(system["model"], CHAIN_MODEL, "{body}");
}

#[tokio::test]
async fn admin_settings_show_no_system_provider_for_the_platform_default() {
    let resources = create_test_server_resources_with_config(ServerConfig {
        llm_chain: LlmChain::default(),
        ..test_server_config()
    })
    .await
    .unwrap();
    let auth = bearer_for(&resources, "chain-default@test.com").await;

    let body = llm_settings(&resources, &auth).await;
    assert!(
        body.get("system_provider").is_none(),
        "an operator who chose no chain has no system provider to show: {body}"
    );
}

#[tokio::test]
async fn a_new_conversation_records_the_model_of_the_chain_the_server_was_given() {
    let provider = Arc::new(RecordingProvider::answering("Noted."));
    let resources = create_test_server_resources_with_chat_provider_and_config(
        provider,
        config_with_chain(&format!("gemini={CHAIN_MODEL}")),
    )
    .await
    .unwrap();
    let auth = bearer_for(&resources, "chain-conversation@test.com").await;

    let created = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &auth)
        .json(&json!({}))
        .send(ChatRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(created.status_code(), StatusCode::CREATED);
    let conversation: ConversationResponse = created.json();
    assert_eq!(conversation.model, CHAIN_MODEL);
}
