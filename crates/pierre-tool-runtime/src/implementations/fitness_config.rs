// ABOUTME: Fitness configuration tools for user training preferences.
// ABOUTME: Implements get_fitness_config, set_fitness_config, list_fitness_configs, delete_fitness_config.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Fitness Configuration Tools
//!
//! This module provides tools for managing fitness configurations with direct database access:
//! - `GetFitnessConfigTool` - Get user's fitness configuration
//! - `SetFitnessConfigTool` - Save or update fitness configuration
//! - `ListFitnessConfigsTool` - List available configuration names
//! - `DeleteFitnessConfigTool` - Remove a configuration
//!
//! All tools use direct database access via `FitnessConfigurationManager`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::config::fitness::FitnessConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

/// Require the caller's tenant. Errors if the tool was invoked without
/// one — never fabricates from the user uuid (see
/// `pierre_runtime_context::tenant` for the policy).
fn require_tenant_id(ctx: &ToolExecutionContext) -> AppResult<TenantId> {
    ctx.tenant_id.map(TenantId::from).ok_or_else(|| {
        AppError::auth_invalid("fitness_config tools require an authenticated tenant context")
    })
}

// ============================================================================
// GetFitnessConfigTool
// ============================================================================

/// Tool for retrieving fitness configuration.
///
/// Retrieves user-specific configuration if available, otherwise falls back to tenant default.
pub struct GetFitnessConfigTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetFitnessConfigTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "configuration_name".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Name of the configuration to retrieve (default: 'default')".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec![]),
        };
        tool_definition(
            "get_fitness_config",
            "Get fitness configuration for the current user (falls back to tenant default)",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let configuration_name = args
                .get("configuration_name")
                .and_then(Value::as_str)
                .unwrap_or("default");

            tracing::debug!(
                user_id = %ctx.user_id,
                config_name = %configuration_name,
                "Getting fitness configuration"
            );

            let repo = ctx.resources.repos().fitness_config.as_ref();
            let user_id_str = ctx.user_id.to_string();
            let tenant_id = require_tenant_id(&ctx)?;

            let config = repo
                .get_user_config(tenant_id, &user_id_str, configuration_name)
                .await?;

            config.map_or_else(
            || {
                Ok(ToolResult::ok(json!({
                    "configuration_name": configuration_name,
                    "config": null,
                    "source": "not_found",
                    "message": format!("No configuration found with name '{configuration_name}'"),
                    "retrieved_at": Utc::now().to_rfc3339(),
                })))
            },
            |fitness_config| {
                Ok(ToolResult::ok(json!({
                    "configuration_name": configuration_name,
                    "config": fitness_config,
                    "source": "database",
                    "retrieved_at": Utc::now().to_rfc3339(),
                })))
            },
        )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SetFitnessConfigTool
// ============================================================================

/// Tool for saving or updating fitness configuration.
pub struct SetFitnessConfigTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SetFitnessConfigTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "configuration_name".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Name for this configuration (default: 'default')".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "config".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some(
                    "Fitness configuration object with sport_types, intelligence settings"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "user_level".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some(
                    "If true, save as user-specific config. If false, save as tenant default (requires admin)".to_owned()
                ),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["config".to_owned()]),
        };
        tool_definition(
            "set_fitness_config",
            "Save or update fitness configuration for the current user or tenant",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let configuration_name = args
                .get("configuration_name")
                .and_then(Value::as_str)
                .unwrap_or("default");

            let config_json = args
                .get("config")
                .ok_or_else(|| AppError::invalid_input("config object is required"))?;

            let user_level = args
                .get("user_level")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            tracing::debug!(
                user_id = %ctx.user_id,
                config_name = %configuration_name,
                user_level = %user_level,
                "Setting fitness configuration"
            );

            // Parse the config to validate it
            let fitness_config: FitnessConfig = serde_json::from_value(config_json.clone())
                .map_err(|e| {
                    AppError::invalid_input(format!("Invalid fitness config format: {e}"))
                })?;

            let repo = ctx.resources.repos().fitness_config.as_ref();
            let user_id_str = ctx.user_id.to_string();
            let tenant_id = require_tenant_id(&ctx)?;

            let config_id: String = if user_level {
                repo.save_user_config(tenant_id, &user_id_str, configuration_name, &fitness_config)
                    .await?
            } else {
                // Tenant-level config requires admin privileges
                ctx.require_admin().await?;
                repo.save_tenant_config(tenant_id, configuration_name, &fitness_config)
                    .await?
            };

            Ok(ToolResult::ok(json!({
                "success": true,
                "config_id": config_id,
                "configuration_name": configuration_name,
                "user_level": user_level,
                "message": format!("Configuration '{}' saved successfully", configuration_name),
                "saved_at": Utc::now().to_rfc3339(),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ListFitnessConfigsTool
// ============================================================================

/// Tool for listing available fitness configuration names.
pub struct ListFitnessConfigsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListFitnessConfigsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "include_tenant".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some("Include tenant-level configurations (default: true)".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec![]),
        };
        tool_definition(
            "list_fitness_configs",
            "List all available fitness configuration names for the user and tenant",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let include_tenant = args
                .get("include_tenant")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            tracing::debug!(
                user_id = %ctx.user_id,
                include_tenant = %include_tenant,
                "Listing fitness configurations"
            );

            let repo = ctx.resources.repos().fitness_config.as_ref();
            let user_id_str = ctx.user_id.to_string();
            let tenant_id = require_tenant_id(&ctx)?;

            // Get user-specific configurations
            let user_configs: Vec<String> = repo
                .list_user_configurations(tenant_id, &user_id_str)
                .await?;

            // Get tenant-level configurations if requested
            let tenant_configs: Vec<String> = if include_tenant {
                repo.list_tenant_configurations(tenant_id).await?
            } else {
                Vec::new()
            };

            // Combine and deduplicate
            let mut all_configs: Vec<String> = user_configs.clone();
            for tc in &tenant_configs {
                if !all_configs.contains(tc) {
                    all_configs.push(tc.clone());
                }
            }
            all_configs.sort();

            Ok(ToolResult::ok(json!({
                "configurations": all_configs,
                "user_specific": user_configs,
                "tenant_level": tenant_configs,
                "total_count": all_configs.len(),
                "retrieved_at": Utc::now().to_rfc3339(),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DeleteFitnessConfigTool
// ============================================================================

/// Tool for deleting a fitness configuration.
pub struct DeleteFitnessConfigTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DeleteFitnessConfigTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "configuration_name".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Name of the configuration to delete".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "user_level".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some(
                    "If true, delete user-specific config. If false, delete tenant config (requires admin)".to_owned()
                ),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["configuration_name".to_owned()]),
        };
        tool_definition(
            "delete_fitness_config",
            "Delete a fitness configuration by name",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
        let configuration_name = args
            .get("configuration_name")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("configuration_name is required"))?;

        let user_level = args
            .get("user_level")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        tracing::debug!(
            user_id = %ctx.user_id,
            config_name = %configuration_name,
            user_level = %user_level,
            "Deleting fitness configuration"
        );

        let repo = ctx.resources.repos().fitness_config.as_ref();
        let user_id_str = ctx.user_id.to_string();
        let tenant_id = require_tenant_id(&ctx)?;

        let user_id_option = if user_level {
            Some(user_id_str.as_str())
        } else {
            // Tenant-level config deletion requires admin privileges
            ctx.require_admin().await?;
            None
        };

        let deleted = repo
            .delete_config(tenant_id, user_id_option, configuration_name)
            .await?;

        if deleted {
            Ok(ToolResult::ok(json!({
                "success": true,
                "configuration_name": configuration_name,
                "user_level": user_level,
                "message": format!("Configuration '{}' deleted successfully", configuration_name),
                "deleted_at": Utc::now().to_rfc3339(),
            })))
        } else {
            Ok(ToolResult::ok(json!({
                "success": false,
                "configuration_name": configuration_name,
                "user_level": user_level,
                "message": format!("Configuration '{}' not found", configuration_name),
            })))
        }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all fitness config tools for registration
#[must_use]
pub fn create_fitness_config_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(GetFitnessConfigTool),
        Box::new(SetFitnessConfigTool),
        Box::new(ListFitnessConfigsTool),
        Box::new(DeleteFitnessConfigTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(DeleteFitnessConfigTool => IRREVERSIBLE);
crate::declare_security!(GetFitnessConfigTool => empty);
crate::declare_security!(ListFitnessConfigsTool => empty);
crate::declare_security!(SetFitnessConfigTool => empty);
