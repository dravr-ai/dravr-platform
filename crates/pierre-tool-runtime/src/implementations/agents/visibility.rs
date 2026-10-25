// ABOUTME: Agent visibility tools — hide_agent, show_agent, list_hidden_agents.
// ABOUTME: The per-user hide/show family, beside the CRUD and listing tools in `agents`.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent Visibility Tools
//!
//! - `HideAgentTool` - Hide an agent from listings
//! - `ShowAgentTool` - Show a hidden agent
//! - `ListHiddenAgentsTool` - List hidden agents

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, apply_format, format_property, object_schema, ok_typed, tool_definition,
    tool_result_to_response, Formatted,
};
use crate::implementations::agents_output::{
    list_hidden_agents_payload, HideAgentResult, ListHiddenAgentsResult, ShowAgentResult,
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
// HideAgentTool
// ============================================================================

/// Tool for hiding an agent from listings.
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
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
