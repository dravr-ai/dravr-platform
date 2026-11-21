// ABOUTME: The two admin tools that start and end a support session through the admin's own assistant (MCP, A2A)
// ABOUTME: Thin: the rules live in crate::support_session, which the /mcp and A2A doors read to act as the user

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `admin_impersonate_start` and `admin_impersonate_end` (carnet#1511)
//!
//! An admin's assistant acts as a user for up to an hour: from the next
//! call on, the connection that started the session is offered and served
//! exactly what that user's own assistant is, plus
//! [`END_TOOL`](crate::support_session::END_TOOL). Both tools are
//! `ADMIN_ONLY`, so they need an admin whose grant holds `admin` (their API
//! key; no OAuth connection holds it while `admin` is not delegable), and the
//! one access predicate refuses them to anyone else. Neither is offered in
//! Dravr's own chat: the web console starts its own support sessions.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::annotations::{annotations, Effect, Reach};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use crate::support_session::{self, END_TOOL, START_TOOL};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::permissions::impersonation::SESSION_MINUTES;
use pierre_core::transport::Transport;
use pierre_mcp_schema::PropertySchema;
use pierre_providers::ai_scope;
use pierre_tools_core::ToolResult;

/// What `admin_impersonate_start` answers.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ImpersonateStartResult {
    /// The support session's id, as the audit trail names it.
    pub session_id: String,
    /// The account this connection now acts as.
    pub user_id: String,
    /// That account's display name, when it has one.
    pub display_name: Option<String>,
    /// When the session stops acting as the user (RFC 3339): an hour after
    /// it started, never renewed.
    pub expires_at: String,
    /// What to do next: list the tools again, and end the session with
    /// `admin_impersonate_end`.
    pub note: String,
}

/// What `admin_impersonate_end` answers.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ImpersonateEndResult {
    /// Whether a live session was ended; `false` when none was.
    pub ended: bool,
    /// The ended session's id.
    pub session_id: Option<String>,
    /// What to do next.
    pub note: String,
}

/// The external transport the call is served over: a support session is
/// started and ended over MCP or A2A only.
fn external_transport() -> AppResult<Transport> {
    ai_scope::declared_transport()
        .filter(|transport| !transport.is_first_party())
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "A support session through an assistant runs over MCP or A2A only; in Dravr's \
                 own app, use the admin console",
            )
        })
}

/// Start a support session acting as one user.
#[derive(Debug)]
pub struct AdminImpersonateStartTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminImpersonateStartTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "user".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "The email or account id of the user to act as. Never another admin."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "reason".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Why you act as this user (the support request), written to the audit trail."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec!["user".to_owned(), "reason".to_owned()]),
        );
        answers_with::<ImpersonateStartResult>(tool_definition(
            "admin_impersonate_start",
            "Admin support: act as one user, through this connection only, for up to an hour. \
             From the next call on, this connection is offered and served exactly what that \
             user's own assistant is (their data, their tools; for a human coach, the members \
             who allowed it), plus `admin_impersonate_end`. List the tools again after it \
             starts. Nothing that would outlive the session (a token, a key, a connection, a \
             consent) can be created, exports are refused, and every call is on the admin audit \
             trail naming you and the user.",
            schema,
            annotations(
                "Start a support session (admin)",
                Effect::AdditiveWrite,
                Reach::Internal,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::ADMIN_ONLY
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            context.require_admin().await?;
            let transport = external_transport()?;
            let field = |name: &str| args.get(name).and_then(Value::as_str).unwrap_or_default();
            let started = support_session::start(
                context.resources.repos(),
                context.user_id,
                support_session::current_binding().as_deref(),
                field("user"),
                field("reason"),
                transport,
            )
            .await?;
            ok_typed(
                START_TOOL,
                ImpersonateStartResult {
                    session_id: started.session.id.clone(),
                    user_id: started.target.id.to_string(),
                    display_name: started.target.display_name.clone(),
                    expires_at: started.expires_at.to_rfc3339(),
                    note: format!(
                        "This connection now acts as this user for up to {SESSION_MINUTES} \
                         minutes. List the tools again: the list is now the user's. Call \
                         `{END_TOOL}` when you are done."
                    ),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// End this connection's support session.
#[derive(Debug)]
pub struct AdminImpersonateEndTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AdminImpersonateEndTool {
    fn definition(&self) -> Tool {
        answers_with::<ImpersonateEndResult>(tool_definition(
            "admin_impersonate_end",
            "Admin support: end the support session this connection started, so it acts as you \
             again. List the tools again afterwards. The end is written to the admin audit trail.",
            object_schema(HashMap::new(), None),
            annotations(
                "End the support session (admin)",
                Effect::IdempotentWrite,
                Reach::Internal,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::ADMIN_ONLY
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            context.require_admin().await?;
            let transport = external_transport()?;
            let ended = support_session::end(
                context.resources.repos(),
                context.user_id,
                support_session::current_binding().as_deref(),
                transport,
            )
            .await?;
            ok_typed(
                END_TOOL,
                ImpersonateEndResult {
                    ended: ended.is_some(),
                    session_id: ended.map(|session| session.id),
                    note: "This connection acts as you again. List the tools again.".to_owned(),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The two support-session tools, for registration beside the admin tools.
#[must_use]
pub fn create_admin_support_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(AdminImpersonateStartTool),
        Box::new(AdminImpersonateEndTool),
    ]
}

crate::declare_security!(AdminImpersonateStartTool => empty);
crate::declare_security!(AdminImpersonateEndTool => empty);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tool_is_named_as_the_doors_recognise_it() {
        assert_eq!(
            AdminImpersonateStartTool.definition().name,
            START_TOOL,
            "bug: the start tool's declared name is the one the doors recognise"
        );
        assert_eq!(
            AdminImpersonateEndTool.definition().name,
            END_TOOL,
            "bug: the end tool's declared name is the one the doors recognise"
        );
    }
}
