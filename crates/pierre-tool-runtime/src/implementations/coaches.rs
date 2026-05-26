// ABOUTME: AI coach management tools with direct database access.
// ABOUTME: Implements list_coaches, create_coach, get_coach, etc. using CoachesManager.
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

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::context::ToolExecutionContext;
use crate::traits::{McpTool, ToolCapabilities};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{
    CoachCategory, CreateCoachRequest, ListCoachesFilter, UpdateCoachRequest,
};
use pierre_core::models::TenantId;
use pierre_formatters::{format_output, OutputFormat};
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// Extract output format ("json" or "toon") from tool arguments.
fn extract_format(args: &Value) -> OutputFormat {
    args.get("format")
        .and_then(Value::as_str)
        .map(OutputFormat::from_str_param)
        .unwrap_or_default()
}

/// Apply TOON formatting to a result payload, mirroring `apply_format_to_response`.
///
/// For JSON format, returns `value` unchanged. For TOON format, returns
/// `{ "<data_key>_toon": <encoded>, "format": "toon" }` on success, or falls
/// back to `{ "<data_key>": <value>, "format": "json", "format_fallback": true,
/// "format_error": "<msg>" }` if encoding fails.
fn finalize_payload(value: Value, data_key: &str, format: OutputFormat) -> Value {
    match format {
        OutputFormat::Json => value,
        OutputFormat::Toon => match format_output(&value, OutputFormat::Toon) {
            Ok(formatted) => {
                let toon_key = format!("{data_key}_toon");
                json!({
                    toon_key: formatted.data,
                    "format": "toon",
                })
            }
            Err(e) => json!({
                data_key: value,
                "format": "json",
                "format_fallback": true,
                "format_error": e.to_string(),
            }),
        },
    }
}

/// Annotations for idempotent write operations (create, update)
fn write_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for destructive operations (delete)
fn destructive_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for read-only coach retrieval operations
fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

// ============================================================================
// ListCoachesTool
// ============================================================================

/// Tool for listing available AI coaches.
pub struct ListCoachesTool;

#[async_trait]
impl McpTool for ListCoachesTool {
    fn name(&self) -> &'static str {
        "list_coaches"
    }

    fn description(&self) -> &'static str {
        "List available AI coaches for personalized training guidance"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let format = extract_format(&args);
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

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

        let coach_summaries: Vec<Value> = coaches
            .iter()
            .map(|item| {
                json!({
                    "id": item.coach.id.to_string(),
                    "title": item.coach.title,
                    "description": item.coach.description,
                    "category": item.coach.category.as_str(),
                    "tags": item.coach.tags,
                    "token_count": item.coach.token_count,
                    "is_favorite": item.is_favorite,
                    "is_system": item.coach.is_system,
                    "is_assigned": item.is_assigned,
                    "use_count": item.use_count,
                    "last_used_at": item.last_used_at.map(|dt| dt.to_rfc3339()),
                    "updated_at": item.coach.updated_at.to_rfc3339(),
                })
            })
            .collect();

        let returned_count = coach_summaries.len();
        #[allow(clippy::cast_possible_truncation)]
        let has_more = limit.is_some_and(|l| returned_count == l as usize);

        let payload = json!({
            "coaches": coach_summaries,
            "count": returned_count,
            "total": total,
            "offset": offset.unwrap_or(0),
            "limit": limit.unwrap_or(50),
            "has_more": has_more,
        });

        Ok(ToolResult::ok(finalize_payload(payload, "coaches", format)))
    }
}

// ============================================================================
// CreateCoachTool
// ============================================================================

/// Tool for creating a custom AI coach.
pub struct CreateCoachTool;

#[async_trait]
impl McpTool for CreateCoachTool {
    fn name(&self) -> &'static str {
        "create_coach"
    }

    fn description(&self) -> &'static str {
        "Create a custom AI coach with personalized training guidance"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["title".to_owned(), "system_prompt".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

        let title = args
            .get("title")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| AppError::invalid_input("Missing required parameter: title"))?;
        let system_prompt = args
            .get("system_prompt")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| AppError::invalid_input("Missing required parameter: system_prompt"))?;
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
        };

        let manager = ctx.resources.coaches_manager();
        let coach = manager
            .create(user_id, tenant_id, &create_request)
            .await
            .map_err(|e| AppError::internal(format!("Failed to create coach: {e}")))?;

        Ok(ToolResult::ok(json!({
            "id": coach.id.to_string(),
            "title": coach.title,
            "description": coach.description,
            "category": coach.category.as_str(),
            "tags": coach.tags,
            "token_count": coach.token_count,
            "created_at": coach.created_at.to_rfc3339(),
        })))
    }
}

// ============================================================================
// GetCoachTool
// ============================================================================

/// Tool for getting coach details.
pub struct GetCoachTool;

#[async_trait]
impl McpTool for GetCoachTool {
    fn name(&self) -> &'static str {
        "get_coach"
    }

    fn description(&self) -> &'static str {
        "Get detailed information about a specific coach"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to retrieve".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let format = extract_format(&args);
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

        let coach_id = args
            .get("coach_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

        let manager = ctx.resources.coaches_manager();
        let coach = manager
            .get_by_id(coach_id, user_id, tenant_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to get coach: {e}")))?;

        match coach {
            Some(c) => {
                let payload = json!({
                    "id": c.id.to_string(),
                    "title": c.title,
                    "description": c.description,
                    "system_prompt": c.system_prompt,
                    "category": c.category.as_str(),
                    "tags": c.tags,
                    "token_count": c.token_count,
                    "is_favorite": false,
                    "use_count": 0,
                    "last_used_at": Option::<String>::None,
                    "created_at": c.created_at.to_rfc3339(),
                    "updated_at": c.updated_at.to_rfc3339(),
                });
                Ok(ToolResult::ok(finalize_payload(payload, "coach", format)))
            }
            None => Ok(ToolResult::error(json!({
                "error": format!("Coach not found: {coach_id}"),
            }))),
        }
    }
}

// ============================================================================
// UpdateCoachTool
// ============================================================================

/// Tool for updating coach settings.
pub struct UpdateCoachTool;

#[async_trait]
impl McpTool for UpdateCoachTool {
    fn name(&self) -> &'static str {
        "update_coach"
    }

    fn description(&self) -> &'static str {
        "Update an existing coach's settings"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

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
        };

        let manager = ctx.resources.coaches_manager();
        let coach = manager
            .update(coach_id, user_id, tenant_id, &update_request)
            .await
            .map_err(|e| AppError::internal(format!("Failed to update coach: {e}")))?;

        match coach {
            Some(c) => Ok(ToolResult::ok(json!({
                "id": c.id.to_string(),
                "title": c.title,
                "description": c.description,
                "system_prompt": c.system_prompt,
                "category": c.category.as_str(),
                "tags": c.tags,
                "token_count": c.token_count,
                "updated_at": c.updated_at.to_rfc3339(),
            }))),
            None => Ok(ToolResult::error(json!({
                "error": format!("Coach not found: {coach_id}"),
            }))),
        }
    }
}

// ============================================================================
// DeleteCoachTool
// ============================================================================

/// Tool for deleting a coach.
pub struct DeleteCoachTool;

#[async_trait]
impl McpTool for DeleteCoachTool {
    fn name(&self) -> &'static str {
        "delete_coach"
    }

    fn description(&self) -> &'static str {
        "Delete a coach"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to delete".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(destructive_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

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
            Ok(ToolResult::ok(json!({
                "deleted": true,
                "coach_id": coach_id,
            })))
        } else {
            Ok(ToolResult::error(json!({
                "error": format!("Coach not found: {coach_id}"),
            })))
        }
    }
}

// ============================================================================
// ToggleCoachFavoriteTool
// ============================================================================

/// Tool for toggling coach favorite status.
pub struct ToggleCoachFavoriteTool;

#[async_trait]
impl McpTool for ToggleCoachFavoriteTool {
    fn name(&self) -> &'static str {
        "toggle_coach_favorite"
    }

    fn description(&self) -> &'static str {
        "Toggle the favorite status of a coach"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

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
                Ok(ToolResult::ok(json!({
                    "coach_id": coach_id,
                    "is_favorite": fav,
                })))
            },
        )
    }
}

// ============================================================================
// SearchCoachesTool
// ============================================================================

/// Tool for searching coaches.
pub struct SearchCoachesTool;

#[async_trait]
impl McpTool for SearchCoachesTool {
    fn name(&self) -> &'static str {
        "search_coaches"
    }

    fn description(&self) -> &'static str {
        "Search for coaches by query. Returns up to 20 results by default. Check the `has_more` field before requesting additional results with offset."
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["query".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let format = extract_format(&args);
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

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

        let results: Vec<Value> = coaches
            .iter()
            .map(|c| {
                json!({
                    "id": c.id.to_string(),
                    "title": c.title,
                    "description": c.description,
                    "category": c.category.as_str(),
                    "tags": c.tags,
                    "token_count": c.token_count,
                })
            })
            .collect();

        let returned_count = results.len();
        let limit_val = limit.unwrap_or(20);
        #[allow(clippy::cast_possible_truncation)]
        let has_more = returned_count == limit_val as usize;

        let payload = json!({
            "query": query,
            "results": results,
            "returned_count": returned_count,
            "offset": offset.unwrap_or(0),
            "limit": limit_val,
            "has_more": has_more,
        });

        Ok(ToolResult::ok(finalize_payload(payload, "results", format)))
    }
}

// ============================================================================
// ActivateCoachTool
// ============================================================================

/// Tool for activating a coach.
pub struct ActivateCoachTool;

#[async_trait]
impl McpTool for ActivateCoachTool {
    fn name(&self) -> &'static str {
        "activate_coach"
    }

    fn description(&self) -> &'static str {
        "Activate a coach for personalized training guidance"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to activate".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

        let coach_id = args
            .get("coach_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

        let manager = ctx.resources.coaches_manager();
        let coach = manager
            .activate_coach(coach_id, user_id, tenant_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to activate coach: {e}")))?;

        match coach {
            Some(c) => Ok(ToolResult::ok(json!({
                "id": c.id.to_string(),
                "title": c.title,
                "description": c.description,
                "system_prompt": c.system_prompt,
                "category": c.category.as_str(),
                "is_active": true,
                "token_count": c.token_count,
            }))),
            None => Ok(ToolResult::error(json!({
                "error": format!("Coach not found: {coach_id}"),
            }))),
        }
    }
}

// ============================================================================
// DeactivateCoachTool
// ============================================================================

/// Tool for deactivating the current coach.
pub struct DeactivateCoachTool;

#[async_trait]
impl McpTool for DeactivateCoachTool {
    fn name(&self) -> &'static str {
        "deactivate_coach"
    }

    fn description(&self) -> &'static str {
        "Deactivate the current coach and return to default AI guidance"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, _args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

        let manager = ctx.resources.coaches_manager();
        let deactivated = manager
            .deactivate_coach(user_id, tenant_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to deactivate coach: {e}")))?;

        Ok(ToolResult::ok(json!({
            "deactivated": deactivated,
        })))
    }
}

// ============================================================================
// GetActiveCoachTool
// ============================================================================

/// Tool for getting the currently active coach.
pub struct GetActiveCoachTool;

#[async_trait]
impl McpTool for GetActiveCoachTool {
    fn name(&self) -> &'static str {
        "get_active_coach"
    }

    fn description(&self) -> &'static str {
        "Get the currently active coach"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let format = extract_format(&args);
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

        let manager = ctx.resources.coaches_manager();
        let coach = manager
            .get_active_coach(user_id, tenant_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to get active coach: {e}")))?;

        match coach {
            Some(c) => {
                let payload = json!({
                    "active": true,
                    "coach": {
                        "id": c.id.to_string(),
                        "title": c.title,
                        "description": c.description,
                        "system_prompt": c.system_prompt,
                        "category": c.category.as_str(),
                        "tags": c.tags,
                        "token_count": c.token_count,
                    }
                });
                Ok(ToolResult::ok(finalize_payload(payload, "coach", format)))
            }
            None => Ok(ToolResult::ok(json!({
                "active": false,
                "coach": Value::Null,
            }))),
        }
    }
}

// ============================================================================
// HideCoachTool
// ============================================================================

/// Tool for hiding a coach from listings.
pub struct HideCoachTool;

#[async_trait]
impl McpTool for HideCoachTool {
    fn name(&self) -> &'static str {
        "hide_coach"
    }

    fn description(&self) -> &'static str {
        "Hide a coach from listings"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to hide".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_id = ctx.user_id;

        let coach_id = args
            .get("coach_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

        let manager = ctx.resources.coaches_manager();
        let success = manager
            .hide_coach(coach_id, user_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to hide coach: {e}")))?;

        if success {
            Ok(ToolResult::ok(json!({
                "coach_id": coach_id,
                "is_hidden": true,
            })))
        } else {
            Ok(ToolResult::error(json!({
                "error": "Coach cannot be hidden (only system or assigned coaches can be hidden)",
                "coach_id": coach_id,
                "is_hidden": false,
            })))
        }
    }
}

// ============================================================================
// ShowCoachTool
// ============================================================================

/// Tool for showing a hidden coach.
pub struct ShowCoachTool;

#[async_trait]
impl McpTool for ShowCoachTool {
    fn name(&self) -> &'static str {
        "show_coach"
    }

    fn description(&self) -> &'static str {
        "Show a previously hidden coach"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to show".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let user_id = ctx.user_id;

        let coach_id = args
            .get("coach_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("Missing required parameter: coach_id"))?;

        let manager = ctx.resources.coaches_manager();
        let success = manager
            .show_coach(coach_id, user_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to show coach: {e}")))?;

        Ok(ToolResult::ok(json!({
            "coach_id": coach_id,
            "is_hidden": false,
            "removed_preference": success,
        })))
    }
}

// ============================================================================
// ListHiddenCoachesTool
// ============================================================================

/// Tool for listing hidden coaches.
pub struct ListHiddenCoachesTool;

#[async_trait]
impl McpTool for ListHiddenCoachesTool {
    fn name(&self) -> &'static str {
        "list_hidden_coaches"
    }

    fn description(&self) -> &'static str {
        "List all hidden coaches"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        let format = extract_format(&args);
        let user_id = ctx.user_id;
        let tenant_id = TenantId::from(ctx.require_tenant()?);

        let manager = ctx.resources.coaches_manager();
        let coaches = manager
            .list_hidden_coaches(user_id, tenant_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to list hidden coaches: {e}")))?;

        let coach_summaries: Vec<Value> = coaches
            .iter()
            .map(|c| {
                json!({
                    "id": c.id.to_string(),
                    "title": c.title,
                    "description": c.description,
                    "category": c.category.as_str(),
                    "is_system": c.is_system,
                })
            })
            .collect();

        let count = coach_summaries.len();
        let payload = json!({
            "coaches": coach_summaries,
            "count": count,
        });

        Ok(ToolResult::ok(finalize_payload(payload, "coaches", format)))
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all coach tools for registration
#[must_use]
pub fn create_coach_tools() -> Vec<Box<dyn McpTool>> {
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
