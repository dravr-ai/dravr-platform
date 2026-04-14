// ABOUTME: Tier 3 coach-authored memory tools — coach_note_add, coach_followup_schedule, remember_fact, recall_user_memory
// ABOUTME: Pure McpTool impls; persistence goes through HarnessMemoryRepository wired in Tier 0
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach-Authored Memory Tools
//!
//! These tools let the coach persona itself write to and read from the
//! harness memory layer via the standard MCP tool-call surface. They are
//! the Letta/MemGPT-style "active memory" complement to the background
//! fact extractor in `services/memory_extraction.rs`.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::DateTime;
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    InsertCoachFollowupParams, InsertCoachNoteParams, UpsertUserFactParams,
};
use pierre_memory::{FactKind, MemoryScope};
use serde_json::{json, Value};

use crate::errors::{AppError, AppResult};
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

/// Annotation set for tools that mutate memory state.
fn write_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

/// Annotation set for read-only memory recall.
fn read_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

fn require_string_field(args: &Value, key: &str) -> AppResult<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| AppError::invalid_input(format!("missing required '{key}' field")))
}

fn optional_string_field(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn ctx_user_id(context: &ToolExecutionContext) -> String {
    context.user_id.to_string()
}

// ============================================================================
// CoachNoteAddTool — write a private coach note about a user
// ============================================================================

/// Coach-authored note write tool.
///
/// Lets the coach persist a note it intentionally wants to remember about
/// the user (e.g., "user prefers no scientific jargon, dislikes percentage
/// targets"). Notes are surfaced to admins via a coach-notes audit log and
/// to the coach itself via recall on the next session.
pub struct CoachNoteAddTool;

#[async_trait]
impl McpTool for CoachNoteAddTool {
    fn name(&self) -> &'static str {
        "coach_note_add"
    }

    fn description(&self) -> &'static str {
        "Persist a private coach note about the user for the harness memory layer. Use this when you decide that something the user said should be remembered across sessions."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "content".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Free-form note content (plain text, no markup, ≤ 2000 characters).".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "ID of the coach authoring the note. Must match the coach attached to the active conversation.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "conversation_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional originating conversation ID for provenance.".to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["content".to_owned(), "coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = TenantId::from(context.require_tenant()?);
        let body = require_string_field(&args, "content")?;
        if body.trim().is_empty() {
            return Err(AppError::invalid_input("note content must not be empty"));
        }
        if body.len() > 2000 {
            return Err(AppError::invalid_input(
                "note content exceeds 2000 character limit",
            ));
        }
        let coach_id = require_string_field(&args, "coach_id")?;
        let conv_ref = optional_string_field(&args, "conversation_id");
        let user_id = ctx_user_id(context);

        let params = InsertCoachNoteParams {
            tenant_id,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: conv_ref.as_deref(),
            scope: MemoryScope::User,
            content: &body,
            embedding: None,
        };
        let note = context
            .resources
            .repos
            .memory
            .insert_coach_note(&params)
            .await?;

        Ok(ToolResult::ok(json!({
            "note_id": note.id,
            "created_at": note.created_at.to_rfc3339(),
        })))
    }
}

// ============================================================================
// CoachFollowupScheduleTool — schedule a future check-in
// ============================================================================

/// Coach followup scheduling tool.
///
/// Lets the coach record a promised future check-in. Pending followups are
/// rendered into the next conversation's system prompt so the coach
/// remembers its commitment.
pub struct CoachFollowupScheduleTool;

#[async_trait]
impl McpTool for CoachFollowupScheduleTool {
    fn name(&self) -> &'static str {
        "coach_followup_schedule"
    }

    fn description(&self) -> &'static str {
        "Schedule a future check-in the coach should remember. The reminder is injected into the system prompt of the next coaching conversation. Use when you tell the user 'I'll check back on X tomorrow.'"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "content".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Reminder text (≤ 500 characters), e.g., 'check on Achilles pain after long run'.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Coach making the promise.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "due_at".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional RFC3339 timestamp for when to surface the reminder.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "conversation_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional originating conversation ID for provenance.".to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["content".to_owned(), "coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = TenantId::from(context.require_tenant()?);
        let body = require_string_field(&args, "content")?;
        if body.trim().is_empty() {
            return Err(AppError::invalid_input(
                "followup content must not be empty",
            ));
        }
        if body.len() > 500 {
            return Err(AppError::invalid_input(
                "followup content exceeds 500 character limit",
            ));
        }
        let coach_id = require_string_field(&args, "coach_id")?;
        let conv_ref = optional_string_field(&args, "conversation_id");
        let due_at = optional_string_field(&args, "due_at")
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| AppError::invalid_input(format!("due_at must be RFC3339: {e}")))
            })
            .transpose()?;
        let user_id = ctx_user_id(context);

        let params = InsertCoachFollowupParams {
            tenant_id,
            user_id: &user_id,
            coach_id: &coach_id,
            conversation_id: conv_ref.as_deref(),
            content: &body,
            due_at,
        };
        let followup = context
            .resources
            .repos
            .memory
            .insert_coach_followup(&params)
            .await?;

        Ok(ToolResult::ok(json!({
            "followup_id": followup.id,
            "status": "pending",
            "due_at": followup.due_at.map(|d| d.to_rfc3339()),
        })))
    }
}

// ============================================================================
// RememberFactTool — let the coach assert a structured fact
// ============================================================================

/// Active-memory fact write tool.
///
/// Lets the coach persona explicitly assert a durable fact about the user
/// (e.g., a goal commitment confirmed in this turn) without waiting for
/// the background extractor to infer it.
pub struct RememberFactTool;

#[async_trait]
impl McpTool for RememberFactTool {
    fn name(&self) -> &'static str {
        "remember_fact"
    }

    fn description(&self) -> &'static str {
        "Persist a structured durable fact about the user (preference, physiology, injury, goal, schedule, equipment, other). Use this when the user explicitly confirms something the coach should remember next time."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "kind".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "One of: preference | physiology | injury | goal | schedule | equipment | other.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "subject".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Short subject phrase, usually 'you'.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "predicate".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Short verb phrase (prefers, has, runs, targets, avoids, ...).".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "object".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Asserted value or detail.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "confidence".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Confidence in [0.0, 1.0]. Direct user confirmation → 0.9+.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Coach attaching the fact (optional; defaults to user-wide).".to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec![
                "kind".to_owned(),
                "subject".to_owned(),
                "predicate".to_owned(),
                "object".to_owned(),
                "confidence".to_owned(),
            ]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = TenantId::from(context.require_tenant()?);
        let kind_str = require_string_field(&args, "kind")?;
        let subject = require_string_field(&args, "subject")?;
        let predicate = require_string_field(&args, "predicate")?;
        let object = require_string_field(&args, "object")?;
        #[allow(clippy::cast_possible_truncation)]
        let confidence_f64 = args
            .get("confidence")
            .and_then(Value::as_f64)
            .ok_or_else(|| AppError::invalid_input("confidence must be a number"))?;
        let confidence = (confidence_f64 as f32).clamp(0.0, 1.0);
        let coach_id = optional_string_field(&args, "coach_id");
        let user_id = ctx_user_id(context);

        let params = UpsertUserFactParams {
            tenant_id,
            user_id: &user_id,
            coach_id: coach_id.as_deref(),
            scope: MemoryScope::User,
            kind: FactKind::parse_lenient(&kind_str),
            subject: &subject,
            predicate: &predicate,
            object: &object,
            confidence,
            source_msg_id: None,
            embedding: None,
        };
        let fact = context
            .resources
            .repos
            .memory
            .upsert_user_fact(&params)
            .await?;

        Ok(ToolResult::ok(json!({
            "fact_id": fact.id,
            "kind": fact.kind.as_str(),
            "confidence": fact.confidence,
        })))
    }
}

// ============================================================================
// RecallUserMemoryTool — read top-k stored facts
// ============================================================================

/// Memory recall read tool.
///
/// Returns the most recently updated stored facts for the user, optionally
/// scoped to a coach or fact kind. Mirrors the `services/memory_recall.rs`
/// retrieval the orchestrator uses to inject facts into the system prompt,
/// exposed as a tool so the coach can also query it explicitly during a turn.
pub struct RecallUserMemoryTool;

#[async_trait]
impl McpTool for RecallUserMemoryTool {
    fn name(&self) -> &'static str {
        "recall_user_memory"
    }

    fn description(&self) -> &'static str {
        "Retrieve stored facts the harness has remembered about the user. Returns recent facts ordered by last-update timestamp. Use this when you need to confirm what you already know before answering."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional coach scope. When set, only facts attached to this coach are returned.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "kind".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional fact kind filter (preference | physiology | injury | goal | schedule | equipment | other).".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum facts to return (1–50, default 12).".to_owned()),
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
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = TenantId::from(context.require_tenant()?);
        let coach_id = optional_string_field(&args, "coach_id");
        let kind = optional_string_field(&args, "kind").map(|s| FactKind::parse_lenient(&s));
        let limit = args
            .get("limit")
            .and_then(Value::as_i64)
            .unwrap_or(12)
            .clamp(1, 50);
        let user_id = ctx_user_id(context);

        let facts = context
            .resources
            .repos
            .memory
            .list_user_facts(tenant_id, &user_id, coach_id.as_deref(), kind, limit)
            .await?;

        let payload: Vec<_> = facts
            .into_iter()
            .map(|f| {
                json!({
                    "id": f.id,
                    "kind": f.kind.as_str(),
                    "subject": f.subject,
                    "predicate": f.predicate,
                    "object": f.object,
                    "confidence": f.confidence,
                    "source_msg_id": f.source_msg_id,
                    "updated_at": f.updated_at.to_rfc3339(),
                })
            })
            .collect();

        Ok(ToolResult::ok(json!({
            "facts": payload,
            "count": payload.len(),
        })))
    }
}

/// Build the full set of Tier 3 memory tools for registration.
#[must_use]
pub fn create_memory_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(CoachNoteAddTool),
        Box::new(CoachFollowupScheduleTool),
        Box::new(RememberFactTool),
        Box::new(RecallUserMemoryTool),
    ]
}
