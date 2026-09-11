// ABOUTME: Admin-only tools for system agent management with direct database access.
// ABOUTME: Implements admin agent operations using AgentsRepository directly.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Admin Tools
//!
//! This module provides admin-only tools for system agent management with direct
//! `AgentsRepository` access (no `dispatch_handler` bridging).
//!
//! - `AdminListSystemAgentsTool` - List all system agents
//! - `AdminCreateSystemAgentTool` - Create a system-wide agent
//! - `AdminGetSystemAgentTool` - Get system agent details
//! - `AdminUpdateSystemAgentTool` - Update a system agent
//! - `AdminDeleteSystemAgentTool` - Delete a system agent
//! - `AdminAssignAgentTool` - Assign agent to a user
//! - `AdminUnassignAgentTool` - Remove agent assignment
//! - `AdminListAgentAssignmentsTool` - List agent assignments
//!
//! Each tool below calls `ctx.require_admin()` to enforce the admin role
//! inline — `UniversalToolExecutor::execute_tool` refuses non-admins at the
//! dispatch chokepoint, and the body's own check holds when a tool is run
//! without it — and `ctx.require_tenant()` to obtain the active tenant
//! before forwarding to the repository. Both refuse with `PermissionDenied`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::admin_output::{
    AdminAssignAgentResult, AdminCreateSystemAgentResult, AdminDeleteSystemAgentResult,
    AdminGetSystemAgentResult, AdminListAgentAssignmentsResult, AdminListSystemAgentsResult,
    AdminUnassignAgentResult, AdminUpdateSystemAgentResult, AgentAssignmentEntry, SystemAgentEntry,
};
use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, apply_format, capabilities_to_tronc, object_schema, object_schema_with_format,
    ok_typed, tool_definition, tool_result_to_response, Formatted,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::field_update::FieldUpdate;
use pierre_core::models::agents::{
    AgentCategory, AgentVisibility, CreateSystemAgentRequest, UpdateAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_core::pagination::parse_limit_offset;
use pierre_formatters::OutputFormat;
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// Extract output format ("json" or "toon") from tool arguments.
fn extract_format(args: &Value) -> OutputFormat {
    args.get("format")
        .and_then(Value::as_str)
        .map(OutputFormat::from_str_param)
        .unwrap_or_default()
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
// AdminListSystemAgentsTool
// ============================================================================

/// Tool for listing system agents (admin only).
pub struct AdminListSystemAgentsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminListSystemAgentsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum number of agents to return. Default: 50".to_owned()),
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
        answers_with::<Formatted<AdminListSystemAgentsResult>>(tool_definition(
            "admin_list_system_agents",
            "List all system agents in the tenant (admin only)",
            schema,
            Some(read_only_annotations()),
        ))
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
            ctx.require_admin().await?;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            // The schema has always advertised limit/offset; execute ignored
            // both, so a client paging through agents silently re-read the
            // full set every call. Clamped per the pagination rule.
            let (limit, offset) = parse_limit_offset(&args, 50, 100);

            let manager = ctx.resources.agents_manager();
            let agents = manager
                .list_system_agents(tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to list system agents: {e}")))?;

            let total = agents.len();
            let agent_summaries: Vec<SystemAgentEntry> = agents
                .iter()
                .skip(offset)
                .take(limit)
                .map(|c| SystemAgentEntry {
                    id: c.id.to_string(),
                    title: c.title.clone(),
                    description: c.description.clone(),
                    category: c.category.as_str().to_owned(),
                    tags: c.tags.clone(),
                    token_count: c.token_count,
                    visibility: c.visibility.as_str().to_owned(),
                    created_at: c.created_at.to_rfc3339(),
                    updated_at: c.updated_at.to_rfc3339(),
                })
                .collect();

            let payload = AdminListSystemAgentsResult {
                count: agent_summaries.len(),
                agents: agent_summaries,
                total,
                offset,
            };

            ok_typed("admin_list_system_agents", apply_format(payload, format))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminCreateSystemAgentTool
// ============================================================================

/// Input parameters for creating a system agent.
#[derive(Debug, Deserialize)]
struct CreateSystemAgentParams {
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

/// Tool for creating system agents (admin only).
pub struct AdminCreateSystemAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminCreateSystemAgentTool {
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
                description: Some("Description explaining the agent's purpose".to_owned()),
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
        let schema = object_schema(
            properties,
            Some(vec!["title".to_owned(), "system_prompt".to_owned()]),
        );
        answers_with::<AdminCreateSystemAgentResult>(tool_definition(
            "admin_create_system_agent",
            "Create a new system agent visible to all tenant users (admin only)",
            schema,
            Some(write_annotations()),
        ))
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
            ctx.require_admin().await?;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let params: CreateSystemAgentParams = serde_json::from_value(args).map_err(|e| {
                AppError::invalid_input(format!("Invalid system agent parameters: {e}"))
            })?;

            let visibility = params
                .visibility
                .as_deref()
                .map_or(AgentVisibility::Tenant, AgentVisibility::parse);

            let create_request = CreateSystemAgentRequest {
                title: params.title.clone(),
                description: params.description,
                system_prompt: params.system_prompt,
                category: params
                    .category
                    .as_deref()
                    .map(AgentCategory::parse)
                    .unwrap_or_default(),
                tags: params.tags,
                sample_prompts: params.sample_prompts,
                visibility,
            };

            let manager = ctx.resources.agents_manager();
            let agent = manager
                .create_system_agent(user_id, tenant_id, &create_request)
                .await
                .map_err(|e| AppError::internal(format!("Failed to create system agent: {e}")))?;

            ok_typed(
                "admin_create_system_agent",
                AdminCreateSystemAgentResult {
                    id: agent.id.to_string(),
                    title: agent.title,
                    description: agent.description,
                    category: agent.category.as_str().to_owned(),
                    tags: agent.tags,
                    token_count: agent.token_count,
                    visibility: agent.visibility.as_str().to_owned(),
                    is_system: agent.is_system,
                    created_at: agent.created_at.to_rfc3339(),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminGetSystemAgentTool
// ============================================================================

/// Tool for getting system agent details (admin only).
pub struct AdminGetSystemAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminGetSystemAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system agent to retrieve".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema_with_format(properties, Some(vec!["agent_id".to_owned()]));
        answers_with::<Formatted<AdminGetSystemAgentResult>>(tool_definition(
            "admin_get_system_agent",
            "Get detailed information about a system agent (admin only)",
            schema,
            Some(read_only_annotations()),
        ))
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
            ctx.require_admin().await?;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: agent_id".to_owned())
                })?;

            let manager = ctx.resources.agents_manager();
            let agent = manager
                .get_system_agent(agent_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to get system agent: {e}")))?;

            match agent {
                Some(c) => {
                    let payload = AdminGetSystemAgentResult {
                        id: c.id.to_string(),
                        title: c.title,
                        description: c.description,
                        system_prompt: c.system_prompt,
                        category: c.category.as_str().to_owned(),
                        tags: c.tags,
                        token_count: c.token_count,
                        visibility: c.visibility.as_str().to_owned(),
                        is_system: c.is_system,
                        created_at: c.created_at.to_rfc3339(),
                        updated_at: c.updated_at.to_rfc3339(),
                    };
                    ok_typed("admin_get_system_agent", apply_format(payload, format))
                }
                None => Ok(ToolResult::error(json!({
                    "error": format!("System agent not found: {agent_id}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminUpdateSystemAgentTool
// ============================================================================

/// Tool for updating system agents (admin only).
pub struct AdminUpdateSystemAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminUpdateSystemAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system agent to update".to_owned()),
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
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));
        answers_with::<AdminUpdateSystemAgentResult>(tool_definition(
            "admin_update_system_agent",
            "Update an existing system agent (admin only)",
            schema,
            Some(write_annotations()),
        ))
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
            ctx.require_admin().await?;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: agent_id".to_owned())
                })?;

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
            let agent = manager
                .update_system_agent(agent_id, tenant_id, &update_request)
                .await
                .map_err(|e| AppError::internal(format!("Failed to update system agent: {e}")))?;

            match agent {
                Some(c) => ok_typed(
                    "admin_update_system_agent",
                    AdminUpdateSystemAgentResult {
                        id: c.id.to_string(),
                        title: c.title,
                        description: c.description,
                        system_prompt: c.system_prompt,
                        category: c.category.as_str().to_owned(),
                        tags: c.tags,
                        token_count: c.token_count,
                        visibility: c.visibility.as_str().to_owned(),
                        is_system: c.is_system,
                        updated_at: c.updated_at.to_rfc3339(),
                    },
                ),
                None => Ok(ToolResult::error(json!({
                    "error": format!("System agent not found: {agent_id}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminDeleteSystemAgentTool
// ============================================================================

/// Tool for deleting system agents (admin only).
pub struct AdminDeleteSystemAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminDeleteSystemAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system agent to delete".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));
        answers_with::<AdminDeleteSystemAgentResult>(tool_definition(
            "admin_delete_system_agent",
            "Delete a system agent and remove all assignments (admin only)",
            schema,
            Some(destructive_annotations()),
        ))
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
            ctx.require_admin().await?;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: agent_id".to_owned())
                })?;

            let manager = ctx.resources.agents_manager();
            let deleted = manager
                .delete_system_agent(agent_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to delete system agent: {e}")))?;

            if deleted {
                ok_typed(
                    "admin_delete_system_agent",
                    AdminDeleteSystemAgentResult {
                        deleted: true,
                        agent_id: agent_id.to_owned(),
                    },
                )
            } else {
                Ok(ToolResult::error(json!({
                    "error": format!("System agent not found: {agent_id}"),
                })))
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminAssignAgentTool
// ============================================================================

/// Tool for assigning agents to users (admin only).
pub struct AdminAssignAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminAssignAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system agent to assign".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "user_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the user to assign the agent to".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec!["agent_id".to_owned(), "user_id".to_owned()]),
        );
        answers_with::<AdminAssignAgentResult>(tool_definition(
            "admin_assign_agent",
            "Assign a system agent to a specific user (admin only)",
            schema,
            Some(write_annotations()),
        ))
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
            ctx.require_admin().await?;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: agent_id".to_owned())
                })?;

            let target_user_id_str =
                args.get("user_id").and_then(Value::as_str).ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: user_id".to_owned())
                })?;

            let target_user_id = Uuid::parse_str(target_user_id_str).map_err(|_| {
                AppError::invalid_input(format!("Invalid user_id: {target_user_id_str}"))
            })?;

            let manager = ctx.resources.agents_manager();

            // Verify the agent exists and is a system agent in this tenant
            let agent = manager
                .get_system_agent(agent_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to get coach: {e}")))?
                .ok_or_else(|| {
                    AppError::invalid_input(format!("System agent not found: {agent_id}"))
                })?;

            // Verify target user belongs to the same tenant as the admin
            verify_user_tenant_membership(&ctx, target_user_id, tenant_id).await?;

            manager
                .assign_agent(agent_id, target_user_id, admin_user_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to assign coach: {e}")))?;

            ok_typed(
                "admin_assign_agent",
                AdminAssignAgentResult {
                    assigned: true,
                    agent_id: agent_id.to_owned(),
                    agent_title: agent.title,
                    user_id: target_user_id.to_string(),
                    assigned_by: admin_user_id.to_string(),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminUnassignAgentTool
// ============================================================================

/// Tool for removing agent assignments (admin only).
pub struct AdminUnassignAgentTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminUnassignAgentTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the system agent to unassign".to_owned()),
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
        let schema = object_schema(
            properties,
            Some(vec!["agent_id".to_owned(), "user_id".to_owned()]),
        );
        answers_with::<AdminUnassignAgentResult>(tool_definition(
            "admin_unassign_agent",
            "Remove an agent assignment from a user (admin only)",
            schema,
            Some(destructive_annotations()),
        ))
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
            ctx.require_admin().await?;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing required parameter: agent_id".to_owned())
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

            let manager = ctx.resources.agents_manager();
            let unassigned = manager
                .unassign_agent(agent_id, target_user_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to unassign coach: {e}")))?;

            if unassigned {
                ok_typed(
                    "admin_unassign_agent",
                    AdminUnassignAgentResult {
                        unassigned: true,
                        agent_id: agent_id.to_owned(),
                        user_id: target_user_id.to_string(),
                    },
                )
            } else {
                Ok(ToolResult::error(json!({
                    "error": format!(
                        "Assignment not found for agent {agent_id} and user {target_user_id}"
                    ),
                })))
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AdminListAgentAssignmentsTool
// ============================================================================

/// Cap on assignment rows one listing returns; `total`/`truncated` in the
/// payload say when the agent has more.
const MAX_ASSIGNMENT_ROWS: usize = 200;

/// Tool for listing agent assignments (admin only).
pub struct AdminListAgentAssignmentsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminListAgentAssignmentsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the agent to list assignments for".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["agent_id".to_owned()]));
        answers_with::<AdminListAgentAssignmentsResult>(tool_definition(
            "admin_list_agent_assignments",
            "List all assignments for a system agent (admin only)",
            schema,
            Some(read_only_annotations()),
        ))
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
            ctx.require_admin().await?;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input("coach_id is required to list assignments".to_owned())
                })?;

            let manager = ctx.resources.agents_manager();

            // Verify the agent belongs to the admin's tenant
            manager
                .get_system_agent(agent_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to verify coach tenant: {e}")))?
                .ok_or_else(|| {
                    AppError::invalid_input(format!("System agent {agent_id} not found"))
                })?;

            // List assignments scoped to the admin's tenant
            let assignments = manager
                .list_assignments_for_tenant(agent_id, tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to list assignments: {e}")))?;

            // Bounded output: a popular system agent in a large tenant can
            // carry an assignment per athlete, and this listing had no cap.
            // The truncation is stated in the payload rather than hidden.
            let total = assignments.len();
            let assignment_list: Vec<AgentAssignmentEntry> = assignments
                .iter()
                .take(MAX_ASSIGNMENT_ROWS)
                .map(|a| AgentAssignmentEntry {
                    user_id: a.user_id.clone(),
                    user_email: a.user_email.clone(),
                    assigned_at: a.assigned_at.clone(),
                    assigned_by: a.assigned_by.clone(),
                })
                .collect();

            ok_typed(
                "admin_list_agent_assignments",
                AdminListAgentAssignmentsResult {
                    agent_id: agent_id.to_owned(),
                    count: assignment_list.len(),
                    assignments: assignment_list,
                    total,
                    truncated: total > MAX_ASSIGNMENT_ROWS,
                },
            )
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
pub fn create_admin_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(AdminListSystemAgentsTool),
        Box::new(AdminCreateSystemAgentTool),
        Box::new(AdminGetSystemAgentTool),
        Box::new(AdminUpdateSystemAgentTool),
        Box::new(AdminDeleteSystemAgentTool),
        Box::new(AdminAssignAgentTool),
        Box::new(AdminUnassignAgentTool),
        Box::new(AdminListAgentAssignmentsTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(AdminDeleteSystemAgentTool => IRREVERSIBLE);
crate::declare_security!(AdminAssignAgentTool => empty);
crate::declare_security!(AdminCreateSystemAgentTool => empty);
// Return agent persona / system-prompt content (agent-authored free text) —
// the same source class as agents.rs GetCoach/ListCoaches (UNTRUSTED_OUTPUT).
// ADMIN_ONLY keeps them off the chat loop today, but the label must be right so
// the compile-time "must classify" net doesn't hide a present-but-wrong label.
crate::declare_security!(AdminGetSystemAgentTool => UNTRUSTED_OUTPUT);
crate::declare_security!(AdminListAgentAssignmentsTool => empty);
crate::declare_security!(AdminListSystemAgentsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(AdminUnassignAgentTool => empty);
crate::declare_security!(AdminUpdateSystemAgentTool => empty);
