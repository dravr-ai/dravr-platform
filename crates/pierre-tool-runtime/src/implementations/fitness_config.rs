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
use serde::Serialize;
use serde_json::Value;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, capabilities_to_tronc, object_schema, ok_typed, tool_definition,
    tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::config::fitness::FitnessConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

/// What `get_fitness_config` answers with.
///
/// One shape for both answers: a missing configuration is a fact about the
/// tenant, not a fault, so it reports `config: null` with `source` saying
/// where the answer came from rather than erroring.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetFitnessConfigResult {
    /// The configuration asked for, echoed back.
    pub configuration_name: String,
    /// The configuration; null when none is stored under that name.
    pub config: Option<FitnessConfig>,
    /// Where the answer came from: `database`, or `not_found`.
    pub source: String,
    /// Why there is no configuration. Absent when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// RFC 3339 timestamp of the read, so a cached answer is recognisable.
    pub retrieved_at: String,
}

/// What `set_fitness_config` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SetFitnessConfigResult {
    /// Always true: the tool errors rather than reporting a failed save.
    pub success: bool,
    /// The stored row's identifier.
    pub config_id: String,
    /// The name it was saved under.
    pub configuration_name: String,
    /// Whether it was saved for one athlete or for the whole tenant.
    pub user_level: bool,
    /// What to tell the operator, already written.
    pub message: String,
    /// RFC 3339 timestamp of the write.
    pub saved_at: String,
}

/// What `list_fitness_configs` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListFitnessConfigsResult {
    /// Every name visible to this athlete, user-level and tenant-level
    /// merged and sorted, with no duplicates.
    pub configurations: Vec<String>,
    /// The names stored for this athlete specifically.
    pub user_specific: Vec<String>,
    /// The names stored for the whole tenant. A name in both lists is
    /// overridden per athlete, which is why both are reported.
    pub tenant_level: Vec<String>,
    /// How many distinct names `configurations` holds.
    pub total_count: usize,
    /// RFC 3339 timestamp of the read.
    pub retrieved_at: String,
}

/// What `delete_fitness_config` answers with.
///
/// Deleting something that was not there answers `success: false` rather than
/// erroring — the configuration is gone either way — so `deleted_at` is what
/// separates a delete that happened from one that had nothing to do.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeleteFitnessConfigResult {
    /// Whether a configuration was actually removed.
    pub success: bool,
    /// The name asked about, echoed back.
    pub configuration_name: String,
    /// Whether the delete was scoped to one athlete or the whole tenant.
    pub user_level: bool,
    /// What to tell the operator, already written.
    pub message: String,
    /// RFC 3339 timestamp of the delete. Absent when nothing was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

/// Require the caller's tenant. Errors if the tool was invoked without
/// one — never fabricates from the user uuid (see
/// `pierre_runtime_context::tenant` for the policy).
fn require_tenant_id(ctx: &ToolExecutionContext) -> AppResult<TenantId> {
    ctx.tenant_id.map(TenantId::from_uuid).ok_or_else(|| {
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
        let schema = object_schema(properties, Some(vec![]));
        answers_with::<GetFitnessConfigResult>(tool_definition(
            "get_fitness_config",
            "Get fitness configuration for the current user (falls back to tenant default)",
            schema,
            None,
        ))
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
                    ok_typed(
                        "get_fitness_config",
                        GetFitnessConfigResult {
                            configuration_name: configuration_name.to_owned(),
                            config: None,
                            source: "not_found".to_owned(),
                            message: Some(format!(
                                "No configuration found with name '{configuration_name}'"
                            )),
                            retrieved_at: Utc::now().to_rfc3339(),
                        },
                    )
                },
                |fitness_config| {
                    ok_typed(
                        "get_fitness_config",
                        GetFitnessConfigResult {
                            configuration_name: configuration_name.to_owned(),
                            config: Some(fitness_config),
                            source: "database".to_owned(),
                            message: None,
                            retrieved_at: Utc::now().to_rfc3339(),
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
        let schema = object_schema(properties, Some(vec!["config".to_owned()]));
        answers_with::<SetFitnessConfigResult>(tool_definition(
            "set_fitness_config",
            "Save or update fitness configuration for the current user or tenant",
            schema,
            None,
        ))
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

            ok_typed(
                "set_fitness_config",
                SetFitnessConfigResult {
                    success: true,
                    config_id,
                    configuration_name: configuration_name.to_owned(),
                    user_level,
                    message: format!("Configuration '{configuration_name}' saved successfully"),
                    saved_at: Utc::now().to_rfc3339(),
                },
            )
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
        let schema = object_schema(properties, Some(vec![]));
        answers_with::<ListFitnessConfigsResult>(tool_definition(
            "list_fitness_configs",
            "List all available fitness configuration names for the user and tenant",
            schema,
            None,
        ))
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

            ok_typed(
                "list_fitness_configs",
                ListFitnessConfigsResult {
                    total_count: all_configs.len(),
                    configurations: all_configs,
                    user_specific: user_configs,
                    tenant_level: tenant_configs,
                    retrieved_at: Utc::now().to_rfc3339(),
                },
            )
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
        let schema = object_schema(properties, Some(vec!["configuration_name".to_owned()]));
        answers_with::<DeleteFitnessConfigResult>(tool_definition(
            "delete_fitness_config",
            "Delete a fitness configuration by name",
            schema,
            None,
        ))
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

            ok_typed(
                "delete_fitness_config",
                if deleted {
                    DeleteFitnessConfigResult {
                        success: true,
                        configuration_name: configuration_name.to_owned(),
                        user_level,
                        message: format!(
                            "Configuration '{configuration_name}' deleted successfully"
                        ),
                        deleted_at: Some(Utc::now().to_rfc3339()),
                    }
                } else {
                    DeleteFitnessConfigResult {
                        success: false,
                        configuration_name: configuration_name.to_owned(),
                        user_level,
                        message: format!("Configuration '{configuration_name}' not found"),
                        deleted_at: None,
                    }
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
