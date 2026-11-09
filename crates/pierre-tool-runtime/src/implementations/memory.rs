// ABOUTME: Tier 3 agent-authored memory tools — remember_fact, recall_user_memory
// ABOUTME: Pure McpTool impls; persistence goes through HarnessMemoryRepository wired in Tier 0
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent-Authored Memory Tools
//!
//! These tools let the agent persona itself write to and read from the
//! harness memory layer via the standard MCP tool-call surface. They are
//! the Letta/MemGPT-style "active memory" complement to the background
//! fact extractor in `services/memory_extraction.rs`.

use crate::annotations::{annotations, Effect, Reach};
use pierre_providers::ai_scope;
use pierre_services::memory_facts::SentenceRenderer;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use pierre_database::repositories::UpsertUserFactParams;
use pierre_memory::{
    FactKind, FactProposal, FactSource, MemoryScope, PredicateCode, ProposedClaim,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

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

/// What `remember_fact` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RememberFactResult {
    /// Identifier of the stored fact.
    pub fact_id: String,
    /// Which category it was filed under.
    pub kind: String,
    /// Extractor confidence, 0.0 to 1.0.
    pub confidence: f32,
    /// `approved` when the athlete's latest message says it in these words,
    /// and the agent reads it from now on. `draft` otherwise: it waits for the
    /// athlete to keep it on their Memory screen, so say it was noted for them
    /// to confirm, not that it is remembered.
    pub status: String,
}

/// One fact as `recall_user_memory` reports it.
///
/// A projection of the stored row, not the row: tenant, user, agent and scope
/// stay behind. `sentence` is the fact rendered in the athlete's own locale by
/// the same renderer the memory screen uses, so the agent reads what they read.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RecalledFact {
    /// Stable identifier.
    pub id: String,
    /// Semantic category.
    pub kind: String,
    /// The closed predicate code behind the sentence.
    pub predicate_code: String,
    /// The fact as a sentence, in the athlete's language.
    pub sentence: String,
    /// The athlete's own words for the value asserted.
    pub object: String,
    /// Extractor confidence, 0.0 to 1.0.
    pub confidence: f32,
    /// Message the fact was extracted from, when it came from a conversation.
    pub source_msg_id: Option<String>,
    /// RFC 3339 timestamp of the last update.
    pub updated_at: String,
}

/// What `recall_user_memory` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RecallUserMemoryResult {
    /// The facts recalled, most relevant first.
    pub facts: Vec<RecalledFact>,
    /// How many were returned.
    pub count: usize,
}

// ============================================================================
// RememberFactTool — let the agent assert a structured fact
// ============================================================================

/// Active-memory fact write tool.
///
/// Lets the agent persona explicitly assert a durable fact about the user
/// (e.g., a goal commitment confirmed in this turn) without waiting for
/// the background extractor to infer it.
#[derive(Debug)]
pub struct RememberFactTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for RememberFactTool {
    fn definition(&self) -> Tool {
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
            "predicate_code".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "What the fact says, as a code for the kind: goal → training_for | working_toward | target_race; preference → prefer | avoid | primarily_train; physiology → have_baseline; injury → have | recovering_from; schedule → can_train_on | cannot_train_on | need_session_on | unavailable; equipment → own | train_on; other → states. `states` fits every kind (the object stands alone). Unknown codes are rejected.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "object".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "The athlete's own words for the value, verbatim, in the athlete's language — never translated or paraphrased.".to_owned(),
                ),
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
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Agent attaching the fact; omit for user-wide.".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec![
                "kind".to_owned(),
                "predicate_code".to_owned(),
                "object".to_owned(),
                "confidence".to_owned(),
            ]),
        );
        answers_with::<RememberFactResult>(tool_definition(
            "remember_fact",
            "Persist a structured durable fact about the user. Use it when the user explicitly confirms something the agent should remember next time.",
            schema,
            annotations("Remember fact about user", Effect::AdditiveWrite, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let kind_str = require_string_field(&args, "kind")?;
            let kind = FactKind::parse_lenient(&kind_str);
            let code_str = require_string_field(&args, "predicate_code")?;
            let predicate_code = PredicateCode::parse(&code_str)
                .filter(|code| code.allowed_for(kind))
                .ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "predicate_code '{code_str}' is not a known code for kind '{}'",
                        kind.as_str()
                    ))
                })?;
            let object = require_string_field(&args, "object")?;
            #[allow(clippy::cast_possible_truncation)]
            let confidence_f64 = args
                .get("confidence")
                .and_then(Value::as_f64)
                .ok_or_else(|| AppError::invalid_input("confidence must be a number"))?;
            let confidence = (confidence_f64 as f32).clamp(0.0, 1.0);
            let agent_id = optional_string_field(&args, "agent_id");
            let user_id = ctx_user_id(&context);

            let params = UpsertUserFactParams {
                tenant_id,
                user_id: &user_id,
                agent_id: agent_id.as_deref(),
                scope: MemoryScope::User,
                kind,
                pillar: None,
                predicate_code,
                object: &object,
                confidence,
                source: FactSource::Coach,
                valid_until: None,
                source_msg_id: None,
                transport_policy: ai_scope::derived_policy(),
            };
            let athlete_message = latest_athlete_message(&context, &user_id, tenant_id).await;
            let proposal = FactProposal::from_tool_call(
                ProposedClaim {
                    user_id: &user_id,
                    kind,
                    predicate_code,
                    object: &object,
                },
                athlete_message.as_deref(),
            );
            let fact = context
                .resources
                .repos()
                .memory
                .propose_user_fact(&params, &proposal)
                .await?;

            ok_typed(
                "remember_fact",
                RememberFactResult {
                    fact_id: fact.id,
                    kind: fact.kind.as_str().to_owned(),
                    confidence: fact.confidence,
                    status: fact.status.as_str().to_owned(),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// How many recent messages are read to find the athlete's latest one.
const ATHLETE_MESSAGE_PEEK: i64 = 8;

/// The athlete's most recent message in this conversation: what a fact the
/// agent remembers is checked against. `None` outside a conversation, or when
/// it cannot be read — the fact is then a draft, never silently approved.
async fn latest_athlete_message(
    context: &ToolExecutionContext,
    user_id: &str,
    tenant_id: TenantId,
) -> Option<String> {
    let conversation_id = context.conversation_id.as_deref()?;
    context
        .resources
        .repos()
        .chat
        .get_recent_messages(
            conversation_id,
            user_id,
            tenant_id,
            ATHLETE_MESSAGE_PEEK,
            ai_scope::readable_policy(),
        )
        .await
        .ok()?
        .into_iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.content)
}

// ============================================================================
// RecallUserMemoryTool — read top-k stored facts
// ============================================================================

/// Memory recall read tool.
///
/// Returns the most recently updated stored facts for the user, optionally
/// scoped to an agent or fact kind. Mirrors the `services/memory_recall.rs`
/// retrieval the orchestrator uses to inject facts into the system prompt,
/// exposed as a tool so the agent can also query it explicitly during a turn.
#[derive(Debug)]
pub struct RecallUserMemoryTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for RecallUserMemoryTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Return only facts attached to this agent.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "kind".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fact kind filter (preference | physiology | injury | goal | schedule | equipment | other).".to_owned(),
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
        let schema = object_schema(properties, None);
        answers_with::<RecallUserMemoryResult>(tool_definition(
            "recall_user_memory",
            "Retrieve the facts remembered about the user, most recently updated first. Use it to confirm what you already know before answering.",
            schema,
            annotations("Recall user memory", Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let agent_id = optional_string_field(&args, "agent_id");
            let kind = optional_string_field(&args, "kind").map(|s| FactKind::parse_lenient(&s));
            let limit = args
                .get("limit")
                .and_then(Value::as_i64)
                .unwrap_or(12)
                .clamp(1, 50);
            let user_id = ctx_user_id(&context);

            let mut facts = context
                .resources
                .repos()
                .memory
                .list_user_facts(
                    tenant_id,
                    &user_id,
                    agent_id.as_deref(),
                    kind,
                    limit,
                    ai_scope::readable_policy(),
                )
                .await?;
            // A fact derived from first-party-only data is withheld from an
            // external caller; the rest are served (carnet#769).
            ai_scope::retain_admitted(&mut facts, |fact| fact.transport_policy);
            // The agent reads each fact as a sentence in the athlete's own
            // locale, rendered by the same function the memory screen uses.
            let locale = context.render_locale(context.user_id).await;
            let sentences =
                SentenceRenderer::new(context.resources.messaging_strings_registry(), &locale);

            let recalled: Vec<RecalledFact> = facts
                .into_iter()
                .map(|f| RecalledFact {
                    id: f.id,
                    kind: f.kind.as_str().to_owned(),
                    predicate_code: f.predicate_code.as_str().to_owned(),
                    sentence: sentences.render(f.predicate_code, &f.object),
                    object: f.object,
                    confidence: f.confidence,
                    source_msg_id: f.source_msg_id,
                    updated_at: f.updated_at.to_rfc3339(),
                })
                .collect();

            ok_typed(
                "recall_user_memory",
                RecallUserMemoryResult {
                    count: recalled.len(),
                    facts: recalled,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Build the full set of Tier 3 memory tools for registration.
#[must_use]
pub fn create_memory_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(RememberFactTool), Box::new(RecallUserMemoryTool)]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(RecallUserMemoryTool => empty);
crate::declare_security!(RememberFactTool => empty; first_party_only);
