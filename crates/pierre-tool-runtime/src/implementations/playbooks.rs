// ABOUTME: GDPR/transparency MCP tools for coaching playbook memory — list_coaching_playbooks + forget_playbook
// ABOUTME: list is chat-callable (coach can surface what it learned); forget is auth-gated, not chat-callable
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coaching playbook memory tools (P7 of coaching playbook memory)
//!
//! Two MCP tools over the learned-playbook store:
//!
//! - [`ListCoachingPlaybooksTool`] — read-only; lets the coach (or an MCP
//!   client) surface "what works for this athlete". Chat-callable, so the coach
//!   can answer the athlete conversationally.
//! - [`ForgetPlaybookTool`] — deletes one of the athlete's playbooks by id (GDPR
//!   "forget this"). Tenant + user scoped, and registered under a non-chat
//!   category so the LLM can never delete a playbook on its own.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, capabilities_to_tronc, object_schema, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// Annotation set for the read-only listing tool.
fn read_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotation set for the destructive forget tool.
fn forget_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// One learned playbook, as `list_coaching_playbooks` reports it.
///
/// Distinct from `pierre_memory::playbooks::Playbook`: that is the stored row,
/// carrying tenant, user and coach identifiers the athlete's own client has no
/// business reading. This is the projection the tool actually sends.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PlaybookEntry {
    /// Stable identifier, and what `forget_playbook` takes.
    pub id: String,
    /// The situation the playbook responds to.
    pub trigger: TriggerEntry,
    /// The action it prescribes.
    pub intervention: InterventionEntry,
    /// Times the intervention worked.
    pub success_count: u32,
    /// Times it did not.
    pub failure_count: u32,
    /// Times the outcome carried no clear signal.
    pub neutral_count: u32,
    /// Wilson lower bound on the success rate, so a 1/1 never outranks an 18/20.
    pub confidence: f32,
    /// RFC 3339 timestamp of the most recent outcome; absent if never labeled.
    pub last_outcome_at: Option<String>,
}

/// The trigger half of a [`PlaybookEntry`].
#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerEntry {
    /// Which pattern fired.
    pub kind: String,
    /// Sport it is scoped to, or absent when it applies across sports.
    pub sport: Option<String>,
    /// How pronounced the trigger was, as a band rather than a raw figure.
    pub magnitude: String,
}

/// The intervention half of a [`PlaybookEntry`].
#[derive(Debug, Serialize, JsonSchema)]
pub struct InterventionEntry {
    /// Which action the playbook prescribes.
    pub kind: String,
    /// Its size where one applies; absent for interventions that carry none.
    pub magnitude: Option<i32>,
}

/// What `list_coaching_playbooks` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListCoachingPlaybooksResult {
    /// The playbooks learned for this athlete, ranked by confidence.
    pub playbooks: Vec<PlaybookEntry>,
    /// How many were returned.
    pub count: usize,
}

/// What `forget_playbook` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ForgetPlaybookResult {
    /// Rows removed. A COUNT, not a flag: it is 0 when the id was not this
    /// athlete's, which is the same answer as "no such playbook" on purpose —
    /// a caller must not be able to probe another athlete's ids by the shape
    /// of the reply.
    pub deleted: u64,
    /// The id that was asked for, echoed back.
    pub playbook_id: String,
}

// ============================================================================
// ListCoachingPlaybooksTool — read what the coach has learned about the athlete
// ============================================================================

/// Read-only tool surfacing the athlete's learned coaching playbooks.
pub struct ListCoachingPlaybooksTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListCoachingPlaybooksTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Max playbooks to return (1-50, default 12).".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);
        answers_with::<ListCoachingPlaybooksResult>(tool_definition(
            "list_coaching_playbooks",
            "List the coaching playbooks the harness has learned for this athlete — the trigger→intervention patterns and how well each has worked (success/failure counts + confidence). Use this to tell the athlete what you have learned about what works for them.",
            schema,
            Some(read_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::READS_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = TenantId::from_uuid(context.require_tenant()?).to_string();
            let user_id = context.user_id.to_string();
            let limit = args
                .get("limit")
                .and_then(Value::as_i64)
                .unwrap_or(12)
                .clamp(1, 50);

            let playbooks = context
                .resources
                .repos()
                .playbooks
                .list_all_user_playbooks(&tenant_id, &user_id, limit)
                .await?;

            let entries: Vec<PlaybookEntry> = playbooks
                .into_iter()
                .map(|p| PlaybookEntry {
                    id: p.id,
                    trigger: TriggerEntry {
                        kind: p.trigger.kind.as_str().to_owned(),
                        sport: p.trigger.sport,
                        magnitude: p.trigger.magnitude.as_str().to_owned(),
                    },
                    intervention: InterventionEntry {
                        kind: p.intervention.kind.as_str().to_owned(),
                        magnitude: p.intervention.magnitude,
                    },
                    success_count: p.success_count,
                    failure_count: p.failure_count,
                    neutral_count: p.neutral_count,
                    confidence: p.confidence,
                    last_outcome_at: p.last_outcome_at.map(|t| t.to_rfc3339()),
                })
                .collect();

            let payload = ListCoachingPlaybooksResult {
                count: entries.len(),
                playbooks: entries,
            };
            Ok(ToolResult::ok(serde_json::to_value(payload).map_err(
                |e| {
                    AppError::internal(format!(
                        "list_coaching_playbooks result did not serialize: {e}"
                    ))
                },
            )?))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ForgetPlaybookTool — GDPR "forget this learned pattern"
// ============================================================================

/// Auth-gated tool deleting one of the athlete's learned playbooks by id.
pub struct ForgetPlaybookTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ForgetPlaybookTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "playbook_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "The id of the playbook to forget (from list_coaching_playbooks).".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["playbook_id".to_owned()]));
        answers_with::<ForgetPlaybookResult>(tool_definition(
            "forget_playbook",
            "Delete one learned coaching playbook by id (GDPR forget). The athlete can only forget their own playbooks.",
            schema,
            Some(forget_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = TenantId::from_uuid(context.require_tenant()?).to_string();
            let user_id = context.user_id.to_string();
            let playbook_id = args
                .get("playbook_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| AppError::invalid_input("missing required 'playbook_id' field"))?;

            let removed = context
                .resources
                .repos()
                .playbooks
                .delete_playbook(&tenant_id, &user_id, &playbook_id)
                .await?;

            let payload = ForgetPlaybookResult {
                deleted: removed,
                playbook_id,
            };
            Ok(ToolResult::ok(serde_json::to_value(payload).map_err(
                |e| AppError::internal(format!("forget_playbook result did not serialize: {e}")),
            )?))
        }
        .await;
        tool_result_to_response(result)
    }
}

// Guardian security classification (the `RuntimeTool` supertrait forces every
// registered tool to declare one — omitting these is a compile error).
// `forget_playbook` deletes a coaching playbook: irreversible. Listing is a
// read of the user's own playbooks — trusted, internal, no egress axis.
crate::declare_security!(ListCoachingPlaybooksTool => empty);
crate::declare_security!(ForgetPlaybookTool => IRREVERSIBLE);
