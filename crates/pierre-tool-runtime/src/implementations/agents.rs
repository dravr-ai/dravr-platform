// ABOUTME: AI agent management tools with direct database access.
// ABOUTME: Implements list_agents, create_agent, get_agent, etc. through AgentsRepository.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # AI Agent Management Tools
//!
//! This module provides tools for AI agent management with direct business logic:
//! - `ListAgentsTool` - List available agents
//! - `CreateAgentTool` - Create a custom agent
//! - `GetAgentTool` - Get agent details
//! - `UpdateAgentTool` - Update agent settings
//! - `DeleteAgentTool` - Delete an agent
//! - `ToggleAgentFavoriteTool` - Toggle favorite status
//! - `SearchAgentsTool` - Search agents
//! - `ActivateAgentTool` - Activate an agent
//! - `DeactivateAgentTool` - Deactivate the active agent
//! - `GetActiveAgentTool` - Get currently active agent
//! - `HideAgentTool` - Hide an agent from listings
//! - `ShowAgentTool` - Show a hidden agent
//! - `ListHiddenAgentsTool` - List hidden agents

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::agents_output::{
    activate_agent_payload, active_agent_payload, create_agent_payload, get_agent_payload,
    list_agents_payload, list_hidden_agents_payload, search_agents_payload, update_agent_payload,
    ActivateAgentResult, CreateAgentResult, DeactivateAgentResult, DeleteAgentResult,
    GetActiveAgentResult, GetAgentResult, HideAgentResult, ListAgentsResult,
    ListHiddenAgentsResult, SearchAgentsResult, ShowAgentResult, ToggleAgentFavoriteResult,
    UpdateAgentResult,
};
use super::agents_tool_shape::{
    destructive_annotations, extract_format, read_only_annotations, write_annotations,
};
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, apply_format, capabilities_to_tronc, format_property, object_schema,
    object_schema_with_format, ok_typed, tool_definition, tool_result_to_response, Formatted,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::field_update::FieldUpdate;
use pierre_core::models::agents::{
    AgentCategory, CreateAgentRequest, ListAgentsFilter, UpdateAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

// ============================================================================
// ListAgentsTool
// ============================================================================

/// Tool for listing available AI agents.
pub struct ListAgentsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListAgentsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Filter by category".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "include_system".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some("Include system agents. Default: true".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "favorites_only".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some("Only show favorites. Default: false".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Max results. Default: 50".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "offset".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Pagination offset. Default: 0".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, None);

        answers_with::<Formatted<ListAgentsResult>>(tool_definition(
            "list_agents",
            "List available AI agents for personalized training guidance",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::READS_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = extract_format(&args);
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let category = args
                .get("category")
                .and_then(Value::as_str)
                .map(AgentCategory::parse);
            let favorites_only = args
                .get("favorites_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            #[allow(clippy::cast_possible_truncation)]
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|v| v.min(100) as u32);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let offset = args.get("offset").and_then(|v| {
                v.as_u64()
                    .map(|n| n.min(u64::from(u32::MAX)) as u32)
                    .or_else(|| v.as_f64().map(|f| f as u32))
            });
            let include_system = args
                .get("include_system")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let include_hidden = args
                .get("include_hidden")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let filter = ListAgentsFilter {
                category,
                favorites_only,
                limit,
                offset,
                include_system,
                include_hidden,
            };

            let manager = ctx.resources.agents_manager();
            let agents = manager
                .list(user_id, tenant_id, &filter)
                .await
                .map_err(|e| AppError::internal(format!("Failed to list coaches: {e}")))?;
            let total = manager
                .count(user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to count coaches: {e}")))?;

            let payload = list_agents_payload(&agents, total, offset, limit);
            ok_typed("list_agents", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// CreateAgentTool
// ============================================================================

/// Tool for creating a custom AI agent.
pub struct CreateAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CreateAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "title".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Display title for the agent".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "system_prompt".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("System prompt that shapes AI responses".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "description".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Description of the agent".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Category: training, nutrition, recovery, recipes, custom".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "tags".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Tags for organization".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Tag label".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "sample_prompts".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Example prompts to show users".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Sample prompt text".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec!["title".to_owned(), "system_prompt".to_owned()]),
        );

        answers_with::<CreateAgentResult>(tool_definition(
            "create_agent",
            "Create a custom AI agent with personalized training guidance",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let title = args
                .get("title")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: title"))?;
            let system_prompt = args
                .get("system_prompt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: system_prompt")
                })?;
            let description = args
                .get("description")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let category = args
                .get("category")
                .and_then(Value::as_str)
                .map(AgentCategory::parse)
                .unwrap_or_default();
            let tags: Vec<String> = args
                .get("tags")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let sample_prompts: Vec<String> = args
                .get("sample_prompts")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default();

            let create_request = CreateAgentRequest {
                title,
                description,
                system_prompt,
                category,
                tags,
                sample_prompts,
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            };

            let manager = ctx.resources.agents_manager();
            let agent = manager
                .create(user_id, tenant_id, &create_request)
                .await
                .map_err(|e| AppError::internal(format!("Failed to create coach: {e}")))?;

            ok_typed("create_agent", create_agent_payload(&agent))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetAgentTool
// ============================================================================

/// Tool for getting agent details.
pub struct GetAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the agent to retrieve".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, Some(vec!["agent_id".to_owned()]));

        answers_with::<Formatted<GetAgentResult>>(tool_definition(
            "get_agent",
            "Get detailed information about a specific agent",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::READS_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = extract_format(&args);
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: agent_id"))?;

            let manager = ctx.resources.agents_manager();
            let agent = manager
                .get_by_id(agent_id, user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to get coach: {e}")))?;

            agent.map_or_else(
                || {
                    Ok(ToolResult::error(json!({
                        "error": format!("Agent not found: {agent_id}"),
                    })))
                },
                |c| ok_typed("get_agent", apply_format(get_agent_payload(&c), format)),
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// UpdateAgentTool
// ============================================================================

/// Tool for updating agent settings.
pub struct UpdateAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for UpdateAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the agent to update".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "title".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New title".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "system_prompt".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New system prompt".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "description".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New description".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New category".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "tags".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("New tags".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Tag label".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));

        answers_with::<UpdateAgentResult>(tool_definition(
            "update_agent",
            "Update an existing agent's settings",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: agent_id"))?;

            let update_request = UpdateAgentRequest {
                title: args
                    .get("title")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                description: args
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                system_prompt: args
                    .get("system_prompt")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                category: args
                    .get("category")
                    .and_then(Value::as_str)
                    .map(AgentCategory::parse),
                tags: args.get("tags").and_then(Value::as_array).map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                }),
                sample_prompts: args
                    .get("sample_prompts")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(ToOwned::to_owned)
                            .collect()
                    }),
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: FieldUpdate::Keep,
            };
            let manager = ctx.resources.agents_manager();
            // The tool schema carries no change-summary argument.
            let agent = manager
                .update(agent_id, user_id, tenant_id, &update_request, None)
                .await
                .map_err(|e| AppError::internal(format!("Failed to update coach: {e}")))?;

            agent.map_or_else(
                || {
                    Ok(ToolResult::error(json!({
                        "error": format!("Agent not found: {agent_id}"),
                    })))
                },
                |c| ok_typed("update_agent", update_agent_payload(&c)),
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DeleteAgentTool
// ============================================================================

/// Tool for deleting an agent.
pub struct DeleteAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DeleteAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the agent to delete".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));

        answers_with::<DeleteAgentResult>(tool_definition(
            "delete_agent",
            "Delete an agent",
            schema,
            Some(destructive_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: agent_id"))?;

            let manager = ctx.resources.agents_manager();
            let deleted = manager
                .delete(agent_id, user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to delete coach: {e}")))?;

            if deleted {
                ok_typed(
                    "delete_agent",
                    DeleteAgentResult {
                        deleted: true,
                        agent_id: agent_id.to_owned(),
                    },
                )
            } else {
                Ok(ToolResult::error(json!({
                    "error": format!("Agent not found: {agent_id}"),
                })))
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ToggleAgentFavoriteTool
// ============================================================================

/// Tool for toggling agent favorite status.
pub struct ToggleAgentFavoriteTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ToggleAgentFavoriteTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the agent".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));

        answers_with::<ToggleAgentFavoriteResult>(tool_definition(
            "toggle_agent_favorite",
            "Toggle the favorite status of an agent",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: agent_id"))?;

            let manager = ctx.resources.agents_manager();
            let is_favorite = manager
                .toggle_favorite(agent_id, user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to toggle favorite: {e}")))?;

            is_favorite.map_or_else(
                || {
                    Ok(ToolResult::error(json!({
                        "error": format!("Agent not found: {agent_id}"),
                    })))
                },
                |fav| {
                    ok_typed(
                        "toggle_agent_favorite",
                        ToggleAgentFavoriteResult {
                            agent_id: agent_id.to_owned(),
                            is_favorite: fav,
                        },
                    )
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SearchAgentsTool
// ============================================================================

/// Tool for searching agents.
pub struct SearchAgentsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SearchAgentsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "query".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Search query".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Filter by category".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum results per request. Default: 20, max: 100".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "offset".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Pagination offset. Default: 0. Only use if previous response had has_more=true".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, Some(vec!["query".to_owned()]));

        answers_with::<Formatted<SearchAgentsResult>>(tool_definition(
            "search_agents",
            "Search for agents by query. Returns up to 20 results by default. Check the `has_more` field before requesting additional results with offset.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::READS_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = extract_format(&args);
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let query = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: query"))?;

            #[allow(clippy::cast_possible_truncation)]
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map(|v| v.min(100) as u32);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let offset = args.get("offset").and_then(|v| {
                v.as_u64()
                    .map(|n| n.min(u64::from(u32::MAX)) as u32)
                    .or_else(|| v.as_f64().map(|f| f as u32))
            });

            let manager = ctx.resources.agents_manager();
            let agents = manager
                .search(user_id, tenant_id, query, limit, offset)
                .await
                .map_err(|e| AppError::internal(format!("Failed to search coaches: {e}")))?;

            let payload = search_agents_payload(query, &agents, offset, limit);
            ok_typed("search_agents", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ActivateAgentTool
// ============================================================================

/// Tool for activating an agent.
pub struct ActivateAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ActivateAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the agent to activate".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));

        answers_with::<ActivateAgentResult>(tool_definition(
            "activate_agent",
            "Activate an agent for personalized training guidance",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: agent_id"))?;

            let manager = ctx.resources.agents_manager();
            let agent = manager
                .activate_agent(agent_id, user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to activate coach: {e}")))?;

            agent.map_or_else(
                || {
                    Ok(ToolResult::error(json!({
                        "error": format!("Agent not found: {agent_id}"),
                    })))
                },
                |c| ok_typed("activate_agent", activate_agent_payload(&c)),
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DeactivateAgentTool
// ============================================================================

/// Tool for deactivating the current agent.
pub struct DeactivateAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DeactivateAgentTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: None,
            ..Default::default()
        };

        answers_with::<DeactivateAgentResult>(tool_definition(
            "deactivate_agent",
            "Deactivate the current agent and return to default AI guidance",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let manager = ctx.resources.agents_manager();
            let deactivated = manager
                .deactivate_agent(user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to deactivate coach: {e}")))?;

            ok_typed("deactivate_agent", DeactivateAgentResult { deactivated })
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetActiveAgentTool
// ============================================================================

/// Tool for getting the currently active agent.
pub struct GetActiveAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetActiveAgentTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::from([("format".to_owned(), format_property())])),
            required: None,
            ..Default::default()
        };

        answers_with::<Formatted<GetActiveAgentResult>>(tool_definition(
            "get_active_agent",
            "Get the currently active agent",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::READS_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = extract_format(&args);
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let manager = ctx.resources.agents_manager();
            let agent = manager
                .get_active_agent(user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to get active coach: {e}")))?;

            let payload = active_agent_payload(agent.as_ref());
            ok_typed("get_active_agent", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// HideAgentTool
// ============================================================================

/// Tool for hiding an agent from listings.
pub struct HideAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for HideAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the agent to hide".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));

        answers_with::<HideAgentResult>(tool_definition(
            "hide_agent",
            "Hide an agent from listings",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
        let user_id = ctx.user_id;

        let agent_id = args
            .get("agent_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("Missing required parameter: agent_id"))?;

        let manager = ctx.resources.agents_manager();
        let success = manager
            .hide_agent(agent_id, user_id, TenantId::from_uuid(ctx.require_tenant()?))
            .await
            .map_err(|e| AppError::internal(format!("Failed to hide coach: {e}")))?;

        if success {
            ok_typed(
                "hide_agent",
                HideAgentResult {
                    agent_id: agent_id.to_owned(),
                    is_hidden: true,
                },
            )
        } else {
            Ok(ToolResult::error(json!({
                "error": "Agent cannot be hidden (only system or assigned agents can be hidden)",
                "agent_id": agent_id,
                "is_hidden": false,
            })))
        }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ShowAgentTool
// ============================================================================

/// Tool for showing a hidden agent.
pub struct ShowAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ShowAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the agent to show".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));

        answers_with::<ShowAgentResult>(tool_definition(
            "show_agent",
            "Show a previously hidden agent",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            ctx.require_tenant()?; // A gate, not a key — see `AgentsRepository::show_agent`.

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: agent_id"))?;

            let manager = ctx.resources.agents_manager();
            let success = manager
                .show_agent(agent_id, ctx.user_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to show coach: {e}")))?;

            ok_typed(
                "show_agent",
                ShowAgentResult {
                    agent_id: agent_id.to_owned(),
                    is_hidden: false,
                    removed_preference: success,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ListHiddenAgentsTool
// ============================================================================

/// Tool for listing hidden agents.
pub struct ListHiddenAgentsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListHiddenAgentsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::from([("format".to_owned(), format_property())])),
            required: None,
            ..Default::default()
        };

        answers_with::<Formatted<ListHiddenAgentsResult>>(tool_definition(
            "list_hidden_agents",
            "List all hidden agents",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::AGENTS
                | ToolCapabilities::READS_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = extract_format(&args);
            let user_id = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let manager = ctx.resources.agents_manager();
            let agents = manager
                .list_hidden_agents(user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to list hidden coaches: {e}")))?;

            let payload = list_hidden_agents_payload(&agents);
            ok_typed("list_hidden_agents", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all agent tools for registration
#[must_use]
pub fn create_agent_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ListAgentsTool),
        Box::new(CreateAgentTool),
        Box::new(GetAgentTool),
        Box::new(UpdateAgentTool),
        Box::new(DeleteAgentTool),
        Box::new(ToggleAgentFavoriteTool),
        Box::new(SearchAgentsTool),
        Box::new(ActivateAgentTool),
        Box::new(DeactivateAgentTool),
        Box::new(GetActiveAgentTool),
        Box::new(HideAgentTool),
        Box::new(ShowAgentTool),
        Box::new(ListHiddenAgentsTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(DeleteAgentTool => IRREVERSIBLE);
crate::declare_security!(ActivateAgentTool => empty);
crate::declare_security!(CreateAgentTool => empty);
crate::declare_security!(DeactivateAgentTool => empty);
crate::declare_security!(GetActiveAgentTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetAgentTool => UNTRUSTED_OUTPUT);
crate::declare_security!(HideAgentTool => empty);
crate::declare_security!(ListAgentsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(ListHiddenAgentsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(SearchAgentsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(ShowAgentTool => empty);
crate::declare_security!(ToggleAgentFavoriteTool => empty);
crate::declare_security!(UpdateAgentTool => empty);
