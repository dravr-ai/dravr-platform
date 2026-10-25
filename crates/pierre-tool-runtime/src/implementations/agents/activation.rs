// ABOUTME: Agent activation tools — activate_agent, deactivate_agent, get_active_agent.
// ABOUTME: The active-agent family, beside the CRUD and listing tools in `agents`.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent Activation Tools
//!
//! - `ActivateAgentTool` - Activate an agent
//! - `DeactivateAgentTool` - Deactivate the active agent
//! - `GetActiveAgentTool` - Get currently active agent

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, apply_format, format_property, object_schema, ok_typed, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::agents_audience::activate_visible_agent;
use crate::implementations::agents_output::{
    activate_agent_payload, active_agent_payload, ActivateAgentResult, DeactivateAgentResult,
    GetActiveAgentResult,
};
use crate::implementations::agents_tool_shape::{
    extract_format, read_only_annotations, write_annotations,
};
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

// ============================================================================
// ActivateAgentTool
// ============================================================================

/// Tool for activating an agent.
#[derive(Debug)]
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            let agent_id = args
                .get("agent_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_input("Missing required parameter: agent_id"))?;

            let agent = activate_visible_agent(&ctx, agent_id, tenant_id)
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
#[derive(Debug)]
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
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
#[derive(Debug)]
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
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
