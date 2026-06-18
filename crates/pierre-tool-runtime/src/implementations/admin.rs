// ABOUTME: Admin-only tools for system coach management with direct database access.
// ABOUTME: Implements admin coach operations using CoachesRepository directly.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Admin Tools
//!
//! This module provides admin-only tools for system coach management with direct
//! `CoachesRepository` access (no `dispatch_handler` bridging).
//!
//! - `AdminListSystemCoachesTool` - List all system coaches
//! - `AdminCreateSystemCoachTool` - Create a system-wide coach
//! - `AdminGetSystemCoachTool` - Get system coach details
//! - `AdminUpdateSystemCoachTool` - Update a system coach
//! - `AdminDeleteSystemCoachTool` - Delete a system coach
//! - `AdminAssignCoachTool` - Assign coach to a user
//! - `AdminUnassignCoachTool` - Remove coach assignment
//! - `AdminListCoachAssignmentsTool` - List coach assignments
//!
//! Each tool below calls `require_admin_access(ctx)` to enforce the
//! admin role and `ctx.require_tenant()` to obtain the active tenant
//! before forwarding to the repository.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{
    CoachCategory, CoachVisibility, CreateSystemCoachRequest, UpdateCoachRequest,
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

/// Enforce admin role for the current request.
///
/// `UniversalToolExecutor::execute_tool` calls `McpTool::execute` directly
/// and does not run the `ADMIN_ONLY` gate that `ToolRegistry::execute`
/// applies, so admin tools must check the role inline. The error is
/// emitted as `InvalidInput` so the executor maps it to
/// `ProtocolError::InvalidRequest` via `app_error_to_protocol_error`.
async fn require_admin_access(ctx: &ToolExecutionContext) -> AppResult<()> {
    if ctx.is_admin().await? {
        Ok(())
    } else {
        Err(AppError::invalid_input(
            "Permission denied: Admin access required",
        ))
    }
}

/// Apply TOON formatting to a result payload, mirroring `apply_format_to_response`.
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

/// Annotations for idempotent write operations (create, update, assign).
fn write_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for destructive operations (delete, unassign).
fn destructive_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for read-only retrieval operations.
fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Verify that a target user belongs to a given tenant.
///
/// Prevents cross-tenant operations by checking tenant membership.
async fn verify_user_tenant_membership(
    ctx: &ToolExecutionContext,
    target_user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<()> {
    let user_tenants = ctx
        .resources
        .repos()
        .tenants
        .list_for_user(target_user_id)
        .await
        .map_err(|e| {
            AppError::internal(format!(
                "Failed to verify tenant membership for user {target_user_id}: {e}"
            ))
        })?;

    if !user_tenants.iter().any(|t| t.id == tenant_id) {
        return Err(AppError::invalid_input(format!(
            "User {target_user_id} does not belong to this tenant"
        )));
    }

    Ok(())
}

// ============================================================================
// AdminListSystemCoachesTool
// ============================================================================

/// Tool for listing system coaches (admin only).
pub struct AdminListSystemCoachesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminListSystemCoachesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum number of coaches to return. Default: 50".to_owned()),
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };
        tool_definition(
            "admin_list_system_coaches",
            "List all system coaches in the tenant (admin only)",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::ADMIN_ONLY,
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
            require_admin_access(&ctx).await?;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            let manager = ctx.resources.coaches_manager();
            let coaches = manager
                .list_system_coaches(tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to list system coaches: {e}")))?;

            let coach_summaries: Vec<Value> = coaches
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id.to_string(),
                        "title": c.title,
                        "description": c.description,
                        "category": c.category.as_str(),
                        "tags": c.tags,
                        "token_count": c.token_count,
                        "visibility": c.visibility.as_str(),
                        "created_at": c.created_at.to_rfc3339(),
                        "updated_at": c.updated_at.to_rfc3339(),
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
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminCreateSystemCoachTool
// ============================================================================

/// Input parameters for creating a system coach.
#[derive(Debug, Deserialize)]
struct CreateSystemCoachParams {
    title: String,
    description: Option<String>,
    system_prompt: String,
    category: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    sample_prompts: Vec<String>,
    visibility: Option<String>,
}

/// Tool for creating system coaches (admin only).
pub struct AdminCreateSystemCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminCreateSystemCoachTool {
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
                description: Some("Description explaining the coach's purpose".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Category: 'training', 'nutrition', 'recovery', 'recipes', 'custom'".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "tags".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Tags for filtering and organization".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Tag label".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "visibility".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Visibility: 'tenant' (default) or 'global'".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["title".to_owned(), "system_prompt".to_owned()]),
        };
        tool_definition(
            "admin_create_system_coach",
            "Create a new system coach visible to all tenant users (admin only)",
            schema,
            Some(write_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::ADMIN_ONLY,
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
            require_admin_access(&ctx).await?;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            let params: CreateSystemCoachParams = serde_json::from_value(args).map_err(|e| {
                AppError::invalid_input(format!("Invalid system coach parameters: {e}"))
            })?;

            let visibility = params
                .visibility
                .as_deref()
                .map_or(CoachVisibility::Tenant, CoachVisibility::parse);

            let create_request = CreateSystemCoachRequest {
                title: params.title.clone(),
                description: params.description,
                system_prompt: params.system_prompt,
                category: params
                    .category
                    .as_deref()
                    .map(CoachCategory::parse)
                    .unwrap_or_default(),
                tags: params.tags,
                sample_prompts: params.sample_prompts,
                visibility,
            };

            let manager = ctx.resources.coaches_manager();
            let coach = manager
                .create_system_coach(user_id, tenant_id, &create_request)
                .await
                .map_err(|e| AppError::internal(format!("Failed to create system coach: {e}")))?;

            Ok(ToolResult::ok(json!({
                "id": coach.id.to_string(),
                "title": coach.title,
                "description": coach.description,
                "category": coach.category.as_str(),
                "tags": coach.tags,
                "token_count": coach.token_count,
                "visibility": coach.visibility.as_str(),
                "is_system": coach.is_system,
                "created_at": coach.created_at.to_rfc3339(),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminGetSystemCoachTool
// ============================================================================

/// Tool for getting system coach details (admin only).
pub struct AdminGetSystemCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminGetSystemCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system coach to retrieve".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        };
        tool_definition(
            "admin_get_system_coach",
            "Get detailed information about a system coach (admin only)",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::ADMIN_ONLY,
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
            require_admin_access(&ctx).await?;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: coach_id".to_owned())
                })?;

            let manager = ctx.resources.coaches_manager();
            let coach = manager
                .get_system_coach(coach_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to get system coach: {e}")))?;

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
                        "visibility": c.visibility.as_str(),
                        "is_system": c.is_system,
                        "created_at": c.created_at.to_rfc3339(),
                        "updated_at": c.updated_at.to_rfc3339(),
                    });
                    Ok(ToolResult::ok(finalize_payload(payload, "coach", format)))
                }
                None => Ok(ToolResult::error(json!({
                    "error": format!("System coach not found: {coach_id}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminUpdateSystemCoachTool
// ============================================================================

/// Tool for updating system coaches (admin only).
pub struct AdminUpdateSystemCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminUpdateSystemCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system coach to update".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "title".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New display title".to_owned()),
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        };
        tool_definition(
            "admin_update_system_coach",
            "Update an existing system coach (admin only)",
            schema,
            Some(write_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::ADMIN_ONLY,
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
            require_admin_access(&ctx).await?;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: coach_id".to_owned())
                })?;

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
                .update_system_coach(coach_id, tenant_id, &update_request)
                .await
                .map_err(|e| AppError::internal(format!("Failed to update system coach: {e}")))?;

            match coach {
                Some(c) => Ok(ToolResult::ok(json!({
                    "id": c.id.to_string(),
                    "title": c.title,
                    "description": c.description,
                    "system_prompt": c.system_prompt,
                    "category": c.category.as_str(),
                    "tags": c.tags,
                    "token_count": c.token_count,
                    "visibility": c.visibility.as_str(),
                    "is_system": c.is_system,
                    "updated_at": c.updated_at.to_rfc3339(),
                }))),
                None => Ok(ToolResult::error(json!({
                    "error": format!("System coach not found: {coach_id}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminDeleteSystemCoachTool
// ============================================================================

/// Tool for deleting system coaches (admin only).
pub struct AdminDeleteSystemCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminDeleteSystemCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system coach to delete".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        };
        tool_definition(
            "admin_delete_system_coach",
            "Delete a system coach and remove all assignments (admin only)",
            schema,
            Some(destructive_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::ADMIN_ONLY,
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
            require_admin_access(&ctx).await?;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: coach_id".to_owned())
                })?;

            let manager = ctx.resources.coaches_manager();
            let deleted = manager
                .delete_system_coach(coach_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to delete system coach: {e}")))?;

            if deleted {
                Ok(ToolResult::ok(json!({
                    "deleted": true,
                    "coach_id": coach_id,
                })))
            } else {
                Ok(ToolResult::error(json!({
                    "error": format!("System coach not found: {coach_id}"),
                })))
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminAssignCoachTool
// ============================================================================

/// Tool for assigning coaches to users (admin only).
pub struct AdminAssignCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminAssignCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system coach to assign".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "user_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the user to assign the coach to".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned(), "user_id".to_owned()]),
        };
        tool_definition(
            "admin_assign_coach",
            "Assign a system coach to a specific user (admin only)",
            schema,
            Some(write_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::ADMIN_ONLY,
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
            let admin_user_id = ctx.user_id;
            require_admin_access(&ctx).await?;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: coach_id".to_owned())
                })?;

            let target_user_id_str =
                args.get("user_id").and_then(Value::as_str).ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: user_id".to_owned())
                })?;

            let target_user_id = Uuid::parse_str(target_user_id_str).map_err(|_| {
                AppError::invalid_input(format!("Invalid user_id: {target_user_id_str}"))
            })?;

            let manager = ctx.resources.coaches_manager();

            // Verify the coach exists and is a system coach in this tenant
            let coach = manager
                .get_system_coach(coach_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to get coach: {e}")))?
                .ok_or_else(|| {
                    AppError::invalid_input(format!("System coach not found: {coach_id}"))
                })?;

            // Verify target user belongs to the same tenant as the admin
            verify_user_tenant_membership(&ctx, target_user_id, tenant_id).await?;

            manager
                .assign_coach(coach_id, target_user_id, admin_user_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to assign coach: {e}")))?;

            Ok(ToolResult::ok(json!({
                "assigned": true,
                "coach_id": coach_id,
                "coach_title": coach.title,
                "user_id": target_user_id.to_string(),
                "assigned_by": admin_user_id.to_string(),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminUnassignCoachTool
// ============================================================================

/// Tool for removing coach assignments (admin only).
pub struct AdminUnassignCoachTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminUnassignCoachTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system coach to unassign".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "user_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the user to remove the assignment from".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned(), "user_id".to_owned()]),
        };
        tool_definition(
            "admin_unassign_coach",
            "Remove a coach assignment from a user (admin only)",
            schema,
            Some(destructive_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::ADMIN_ONLY,
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
            require_admin_access(&ctx).await?;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: coach_id".to_owned())
                })?;

            let target_user_id_str =
                args.get("user_id").and_then(Value::as_str).ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: user_id".to_owned())
                })?;

            let target_user_id = Uuid::parse_str(target_user_id_str).map_err(|_| {
                AppError::invalid_input(format!("Invalid user_id: {target_user_id_str}"))
            })?;

            // Verify target user belongs to the same tenant as the admin
            verify_user_tenant_membership(&ctx, target_user_id, tenant_id).await?;

            let manager = ctx.resources.coaches_manager();
            let unassigned = manager
                .unassign_coach(coach_id, target_user_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to unassign coach: {e}")))?;

            if unassigned {
                Ok(ToolResult::ok(json!({
                    "unassigned": true,
                    "coach_id": coach_id,
                    "user_id": target_user_id.to_string(),
                })))
            } else {
                Ok(ToolResult::error(json!({
                    "error": format!(
                        "Assignment not found for coach {coach_id} and user {target_user_id}"
                    ),
                })))
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminListCoachAssignmentsTool
// ============================================================================

/// Tool for listing coach assignments (admin only).
pub struct AdminListCoachAssignmentsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminListCoachAssignmentsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to list assignments for".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        };
        tool_definition(
            "admin_list_coach_assignments",
            "List all assignments for a system coach (admin only)",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::ADMIN_ONLY,
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
            require_admin_access(&ctx).await?;
            let tenant_id = TenantId::from(ctx.require_tenant()?);

            let coach_id = args
                .get("coach_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("coach_id is required to list assignments".to_owned())
                })?;

            let manager = ctx.resources.coaches_manager();

            // Verify the coach belongs to the admin's tenant
            manager
                .get_system_coach(coach_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to verify coach tenant: {e}")))?
                .ok_or_else(|| {
                    AppError::invalid_input(format!("System coach {coach_id} not found"))
                })?;

            // List assignments scoped to the admin's tenant
            let assignments = manager
                .list_assignments_for_tenant(coach_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to list assignments: {e}")))?;

            let assignment_list: Vec<Value> = assignments
                .iter()
                .map(|a| {
                    json!({
                        "user_id": a.user_id,
                        "user_email": a.user_email,
                        "assigned_at": a.assigned_at,
                        "assigned_by": a.assigned_by,
                    })
                })
                .collect();

            Ok(ToolResult::ok(json!({
                "coach_id": coach_id,
                "assignments": assignment_list,
                "count": assignment_list.len(),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all admin tools for registration.
#[must_use]
pub fn create_admin_tools() -> Vec<Box<dyn McpTool<dyn ToolRuntime>>> {
    vec![
        Box::new(AdminListSystemCoachesTool),
        Box::new(AdminCreateSystemCoachTool),
        Box::new(AdminGetSystemCoachTool),
        Box::new(AdminUpdateSystemCoachTool),
        Box::new(AdminDeleteSystemCoachTool),
        Box::new(AdminAssignCoachTool),
        Box::new(AdminUnassignCoachTool),
        Box::new(AdminListCoachAssignmentsTool),
    ]
}
