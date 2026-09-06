// ABOUTME: AI coach management tools with direct database access.
// ABOUTME: Implements list_coaches, create_coach, get_coach, etc. through CoachesRepository.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # AI Coach Management Tools
//!
//! This module provides tools for AI coach management with direct business logic:
//! - `ListCoachesTool` - List available coaches
//! - `CreateCoachTool` - Create a custom coach
//! - `GetCoachTool` - Get coach details
//! - `UpdateCoachTool` - Update coach settings
//! - `DeleteCoachTool` - Delete a coach
//! - `ToggleCoachFavoriteTool` - Toggle favorite status
//! - `SearchCoachesTool` - Search coaches
//! - `ActivateCoachTool` - Activate a coach
//! - `DeactivateCoachTool` - Deactivate the active coach
//! - `GetActiveCoachTool` - Get currently active coach
//! - `HideCoachTool` - Hide a coach from listings
//! - `ShowCoachTool` - Show a hidden coach
//! - `ListHiddenCoachesTool` - List hidden coaches

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::coaches_output::{
    activate_coach_payload, active_coach_payload, create_coach_payload, get_coach_payload,
    list_coaches_payload, list_hidden_coaches_payload, search_coaches_payload,
    update_coach_payload, ActivateCoachResult, CreateCoachResult, DeactivateCoachResult,
    DeleteCoachResult, GetActiveCoachResult, GetCoachResult, HideCoachResult, ListCoachesResult,
    ListHiddenCoachesResult, SearchCoachesResult, ShowCoachResult, ToggleCoachFavoriteResult,
    UpdateCoachResult,
};
use super::coaches_tool_shape::{
    destructive_annotations, extract_format, read_only_annotations, write_annotations,
};
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, apply_format, capabilities_to_tronc, object_schema, ok_typed, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::field_update::FieldUpdate;
use pierre_core::models::coaches::{
    CoachCategory, CreateCoachRequest, ListCoachesFilter, UpdateCoachRequest,
};
use pierre_core::models::TenantId;
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

// ============================================================================
// ListCoachesTool
// ============================================================================

/// Tool for listing available AI coaches.
pub struct ListCoachesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListCoachesTool {
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
                description: Some("Include system coaches. Default: true".to_owned()),
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
        let schema = object_schema(properties, None);

        answers_with::<Formatted<ListCoachesResult>>(tool_definition(
            "list_coaches",
            "List available AI coaches for personalized training guidance",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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
                .map(CoachCategory::parse);
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

            let filter = ListCoachesFilter {
                category,
                favorites_only,
                limit,
                offset,
                include_system,
                include_hidden,
            };

            let manager = ctx.resources.coaches_manager();
            let coaches = manager
                .list(user_id, tenant_id, &filter)
                .await
                .map_err(|e| AppError::internal(format!("Failed to list coaches: {e}")))?;
            let total = manager
                .count(user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to count coaches: {e}")))?;

            let payload = list_coaches_payload(&coaches, total, offset, limit);
            ok_typed("list_coaches", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// CreateCoachTool
// ============================================================================

/// Tool for creating a custom AI coach.
pub struct CreateCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CreateCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "title".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Display title for the coach".to_owned()),
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
                description: Some("Description of the coach".to_owned()),
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

        answers_with::<CreateCoachResult>(tool_definition(
            "create_coach",
            "Create a custom AI coach with personalized training guidance",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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
                .map(CoachCategory::parse)
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

            let create_request = CreateCoachRequest {
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

            let manager = ctx.resources.coaches_manager();
            let coach = manager
                .create(user_id, tenant_id, &create_request)
                .await
                .map_err(|e| AppError::internal(format!("Failed to create coach: {e}")))?;

            ok_typed("create_coach", create_coach_payload(&coach))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetCoachTool
// ============================================================================

/// Tool for getting coach details.
pub struct GetCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to retrieve".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["coach_id".to_owned()]));

        answers_with::<Formatted<GetCoachResult>>(tool_definition(
            "get_coach",
            "Get detailed information about a specific coach",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

            let manager = ctx.resources.coaches_manager();
            let coach = manager
                .get_by_id(coach_id, user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to get coach: {e}")))?;

            coach.map_or_else(
                || {
                    Ok(ToolResult::error(json!({
                        "error": format!("Coach not found: {coach_id}"),
                    })))
                },
                |c| ok_typed("get_coach", apply_format(get_coach_payload(&c), format)),
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// UpdateCoachTool
// ============================================================================

/// Tool for updating coach settings.
pub struct UpdateCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for UpdateCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to update".to_owned()),
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
        let schema = object_schema(properties, Some(vec!["coach_id".to_owned()]));

        answers_with::<UpdateCoachResult>(tool_definition(
            "update_coach",
            "Update an existing coach's settings",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

            let update_request = UpdateCoachRequest {
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
                    .map(CoachCategory::parse),
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
            let manager = ctx.resources.coaches_manager();
            // The tool schema carries no change-summary argument.
            let coach = manager
                .update(coach_id, user_id, tenant_id, &update_request, None)
                .await
                .map_err(|e| AppError::internal(format!("Failed to update coach: {e}")))?;

            coach.map_or_else(
                || {
                    Ok(ToolResult::error(json!({
                        "error": format!("Coach not found: {coach_id}"),
                    })))
                },
                |c| ok_typed("update_coach", update_coach_payload(&c)),
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DeleteCoachTool
// ============================================================================

/// Tool for deleting a coach.
pub struct DeleteCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DeleteCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to delete".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["coach_id".to_owned()]));

        answers_with::<DeleteCoachResult>(tool_definition(
            "delete_coach",
            "Delete a coach",
            schema,
            Some(destructive_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

            let manager = ctx.resources.coaches_manager();
            let deleted = manager
                .delete(coach_id, user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to delete coach: {e}")))?;

            if deleted {
                ok_typed(
                    "delete_coach",
                    DeleteCoachResult {
                        deleted: true,
                        coach_id: coach_id.to_owned(),
                    },
                )
            } else {
                Ok(ToolResult::error(json!({
                    "error": format!("Coach not found: {coach_id}"),
                })))
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ToggleCoachFavoriteTool
// ============================================================================

/// Tool for toggling coach favorite status.
pub struct ToggleCoachFavoriteTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ToggleCoachFavoriteTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["coach_id".to_owned()]));

        answers_with::<ToggleCoachFavoriteResult>(tool_definition(
            "toggle_coach_favorite",
            "Toggle the favorite status of a coach",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

            let manager = ctx.resources.coaches_manager();
            let is_favorite = manager
                .toggle_favorite(coach_id, user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to toggle favorite: {e}")))?;

            is_favorite.map_or_else(
                || {
                    Ok(ToolResult::error(json!({
                        "error": format!("Coach not found: {coach_id}"),
                    })))
                },
                |fav| {
                    ok_typed(
                        "toggle_coach_favorite",
                        ToggleCoachFavoriteResult {
                            coach_id: coach_id.to_owned(),
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
// SearchCoachesTool
// ============================================================================

/// Tool for searching coaches.
pub struct SearchCoachesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SearchCoachesTool {
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
        let schema = object_schema(properties, Some(vec!["query".to_owned()]));

        answers_with::<Formatted<SearchCoachesResult>>(tool_definition(
            "search_coaches",
            "Search for coaches by query. Returns up to 20 results by default. Check the `has_more` field before requesting additional results with offset.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let manager = ctx.resources.coaches_manager();
            let coaches = manager
                .search(user_id, tenant_id, query, limit, offset)
                .await
                .map_err(|e| AppError::internal(format!("Failed to search coaches: {e}")))?;

            let payload = search_coaches_payload(query, &coaches, offset, limit);
            ok_typed("search_coaches", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ActivateCoachTool
// ============================================================================

/// Tool for activating a coach.
pub struct ActivateCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ActivateCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to activate".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["coach_id".to_owned()]));

        answers_with::<ActivateCoachResult>(tool_definition(
            "activate_coach",
            "Activate a coach for personalized training guidance",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

            let manager = ctx.resources.coaches_manager();
            let coach = manager
                .activate_coach(coach_id, user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to activate coach: {e}")))?;

            coach.map_or_else(
                || {
                    Ok(ToolResult::error(json!({
                        "error": format!("Coach not found: {coach_id}"),
                    })))
                },
                |c| ok_typed("activate_coach", activate_coach_payload(&c)),
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DeactivateCoachTool
// ============================================================================

/// Tool for deactivating the current coach.
pub struct DeactivateCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DeactivateCoachTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: None,
            ..Default::default()
        };

        answers_with::<DeactivateCoachResult>(tool_definition(
            "deactivate_coach",
            "Deactivate the current coach and return to default AI guidance",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let manager = ctx.resources.coaches_manager();
            let deactivated = manager
                .deactivate_coach(user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to deactivate coach: {e}")))?;

            ok_typed("deactivate_coach", DeactivateCoachResult { deactivated })
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetActiveCoachTool
// ============================================================================

/// Tool for getting the currently active coach.
pub struct GetActiveCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetActiveCoachTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: None,
            ..Default::default()
        };

        answers_with::<Formatted<GetActiveCoachResult>>(tool_definition(
            "get_active_coach",
            "Get the currently active coach",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let manager = ctx.resources.coaches_manager();
            let coach = manager
                .get_active_coach(user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to get active coach: {e}")))?;

            let payload = active_coach_payload(coach.as_ref());
            ok_typed("get_active_coach", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// HideCoachTool
// ============================================================================

/// Tool for hiding a coach from listings.
pub struct HideCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for HideCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to hide".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["coach_id".to_owned()]));

        answers_with::<HideCoachResult>(tool_definition(
            "hide_coach",
            "Hide a coach from listings",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

        let coach_id = args
            .get("coach_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

        let manager = ctx.resources.coaches_manager();
        let success = manager
            .hide_coach(coach_id, user_id, TenantId::from_uuid(ctx.require_tenant()?))
            .await
            .map_err(|e| AppError::internal(format!("Failed to hide coach: {e}")))?;

        if success {
            ok_typed(
                "hide_coach",
                HideCoachResult {
                    coach_id: coach_id.to_owned(),
                    is_hidden: true,
                },
            )
        } else {
            Ok(ToolResult::error(json!({
                "error": "Agent cannot be hidden (only system or assigned agents can be hidden)",
                "coach_id": coach_id,
                "is_hidden": false,
            })))
        }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ShowCoachTool
// ============================================================================

/// Tool for showing a hidden coach.
pub struct ShowCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ShowCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to show".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["coach_id".to_owned()]));

        answers_with::<ShowCoachResult>(tool_definition(
            "show_coach",
            "Show a previously hidden coach",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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
            ctx.require_tenant()?; // A gate, not a key — see `CoachesRepository::show_coach`.

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

            let manager = ctx.resources.coaches_manager();
            let success = manager
                .show_coach(coach_id, ctx.user_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to show coach: {e}")))?;

            ok_typed(
                "show_coach",
                ShowCoachResult {
                    coach_id: coach_id.to_owned(),
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
// ListHiddenCoachesTool
// ============================================================================

/// Tool for listing hidden coaches.
pub struct ListHiddenCoachesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListHiddenCoachesTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: None,
            ..Default::default()
        };

        answers_with::<Formatted<ListHiddenCoachesResult>>(tool_definition(
            "list_hidden_coaches",
            "List all hidden coaches",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::COACHES
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

            let manager = ctx.resources.coaches_manager();
            let coaches = manager
                .list_hidden_coaches(user_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to list hidden coaches: {e}")))?;

            let payload = list_hidden_coaches_payload(&coaches);
            ok_typed("list_hidden_coaches", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all coach tools for registration
#[must_use]
pub fn create_coach_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ListCoachesTool),
        Box::new(CreateCoachTool),
        Box::new(GetCoachTool),
        Box::new(UpdateCoachTool),
        Box::new(DeleteCoachTool),
        Box::new(ToggleCoachFavoriteTool),
        Box::new(SearchCoachesTool),
        Box::new(ActivateCoachTool),
        Box::new(DeactivateCoachTool),
        Box::new(GetActiveCoachTool),
        Box::new(HideCoachTool),
        Box::new(ShowCoachTool),
        Box::new(ListHiddenCoachesTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(DeleteCoachTool => IRREVERSIBLE);
crate::declare_security!(ActivateCoachTool => empty);
crate::declare_security!(CreateCoachTool => empty);
crate::declare_security!(DeactivateCoachTool => empty);
crate::declare_security!(GetActiveCoachTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetCoachTool => UNTRUSTED_OUTPUT);
crate::declare_security!(HideCoachTool => empty);
crate::declare_security!(ListCoachesTool => UNTRUSTED_OUTPUT);
crate::declare_security!(ListHiddenCoachesTool => UNTRUSTED_OUTPUT);
crate::declare_security!(SearchCoachesTool => UNTRUSTED_OUTPUT);
crate::declare_security!(ShowCoachTool => empty);
crate::declare_security!(ToggleCoachFavoriteTool => empty);
crate::declare_security!(UpdateCoachTool => empty);
