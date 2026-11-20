// ABOUTME: The host seam serving the JSON-RPC methods the generic MCP engine leaves open: resources/* and prompts/*
// ABOUTME: Publishes the coach catalogue as resources and assembles the curated prompt templates

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::mcp::host::MethodHandler;
use dravr_tronc::mcp::protocol::JsonRpcResponse;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_database::repositories::CoachTools;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use tracing::{debug, error};

use crate::constants::errors::{
    ERROR_INTERNAL_ERROR, ERROR_INVALID_PARAMS, ERROR_RESOURCE_NOT_FOUND,
};
use crate::mcp::prompt_templates::{self, PromptGetOutcome};
use crate::mcp::request_span::record_caller;
use crate::mcp::resource_catalog;
use crate::mcp::resources::ServerContext;

/// Handler for the JSON-RPC methods the generic engine leaves open.
///
/// Serves `resources/*` and `prompts/*`; `initialize`/`ping`/`tools/*` are
/// served by the engine. Anything else is left to the engine's -32601.
#[derive(Debug)]
pub struct PierreMethodHandler {
    /// Shared server resources backing resources/prompts.
    pub resources: Arc<ServerContext>,
}

#[async_trait]
impl MethodHandler<dyn ToolRuntime> for PierreMethodHandler {
    async fn handle(
        &self,
        method: &str,
        id: Option<Value>,
        params: Option<Value>,
        _state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
    ) -> Option<JsonRpcResponse> {
        record_caller(ctx);
        match method {
            "resources/list" => Some(self.handle_resources_list(id).await),
            "resources/read" => Some(self.handle_resources_read(id, params.as_ref()).await),
            "resources/templates/list" => Some(JsonRpcResponse::success(
                id,
                resource_catalog::list_resource_templates(),
            )),
            "prompts/list" => Some(JsonRpcResponse::success(
                id,
                prompt_templates::list_prompts(),
            )),
            "prompts/get" => Some(Self::handle_prompts_get(id, params.as_ref())),
            _ => None,
        }
    }
}

impl PierreMethodHandler {
    /// Serve `resources/list` — the global, publicly-published agent catalog.
    async fn handle_resources_list(&self, id: Option<Value>) -> JsonRpcResponse {
        debug!("Handling resources/list request");

        match self
            .resources
            .store_listings_repository()
            // A public catalogue with no viewer to ask: coach-facing agents
            // are left out, as for any athlete.
            .get_published_agents(
                None,
                CoachTools::Exclude,
                None,
                Some(resource_catalog::list_limit()),
                Some(0),
            )
            .await
        {
            Ok(published) => {
                JsonRpcResponse::success(id, resource_catalog::list_resources(&published))
            }
            Err(e) => {
                error!("resources/list: failed to load published coaches: {e}");
                JsonRpcResponse::error(id, ERROR_INTERNAL_ERROR, "Failed to load coach catalog")
            }
        }
    }

    /// Serve `resources/read` — a single published agent by `dravr://agents/{id}` URI.
    async fn handle_resources_read(
        &self,
        id: Option<Value>,
        params: Option<&Value>,
    ) -> JsonRpcResponse {
        debug!("Handling resources/read request");

        let Some(uri) = params.and_then(|p| p.get("uri")).and_then(Value::as_str) else {
            return JsonRpcResponse::error(id, ERROR_INVALID_PARAMS, "Missing 'uri' parameter");
        };

        let Some(agent_id) = resource_catalog::agent_id_from_uri(uri) else {
            return JsonRpcResponse::error(
                id,
                ERROR_INVALID_PARAMS,
                "Unsupported resource URI scheme",
            );
        };

        let published = match self
            .resources
            .store_listings_repository()
            .get_published_agent(agent_id)
            .await
        {
            Ok(Some(agent_with_listing)) => agent_with_listing,
            Ok(None) => {
                return JsonRpcResponse::error(id, ERROR_RESOURCE_NOT_FOUND, "Resource not found");
            }
            Err(e) => {
                error!("resources/read: failed to load coach: {e}");
                return JsonRpcResponse::error(id, ERROR_INTERNAL_ERROR, "Failed to load coach");
            }
        };

        match resource_catalog::read_resource(&published.agent) {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(e) => {
                error!("resources/read: failed to render coach markdown: {e}");
                JsonRpcResponse::error(id, ERROR_INTERNAL_ERROR, "Failed to render resource")
            }
        }
    }

    /// Serve `prompts/get` — assemble a curated prompt template's messages.
    fn handle_prompts_get(id: Option<Value>, params: Option<&Value>) -> JsonRpcResponse {
        debug!("Handling prompts/get request");

        let Some(name) = params.and_then(|p| p.get("name")).and_then(Value::as_str) else {
            return JsonRpcResponse::error(id, ERROR_INVALID_PARAMS, "Missing 'name' parameter");
        };

        let arguments = extract_prompt_arguments(params);

        match prompt_templates::get_prompt(name, &arguments) {
            PromptGetOutcome::Found(result) => JsonRpcResponse::success(id, result),
            PromptGetOutcome::UnknownPrompt(prompt) => JsonRpcResponse::error(
                id,
                ERROR_INVALID_PARAMS,
                format!("Unknown prompt: {prompt}"),
            ),
            PromptGetOutcome::MissingArgument { prompt, argument } => JsonRpcResponse::error(
                id,
                ERROR_INVALID_PARAMS,
                format!("Prompt '{prompt}' requires argument '{argument}'"),
            ),
        }
    }
}

/// Extract the `arguments` object from a prompts/get request into a string map.
///
/// Non-string argument values are stringified so templates always receive a
/// usable value; missing or malformed `arguments` yields an empty map.
fn extract_prompt_arguments(params: Option<&Value>) -> HashMap<String, String> {
    params
        .and_then(|p| p.get("arguments"))
        .and_then(Value::as_object)
        .map_or_default(|map| {
            map.iter()
                .map(|(key, value)| {
                    let text = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned);
                    (key.clone(), text)
                })
                .collect()
        })
}
