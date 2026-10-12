// ABOUTME: Sinks that persist a turn's tool-dispatch rounds, and the per-turn llm_usage recorder for its LLM calls
// ABOUTME: Each spawns its write so recording never blocks the turn it is observing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Turn recorders.
//!
//! [`ChatRepoToolMessageRecorder`] implements the tool loop's round sink and is
//! handed to the loop by [`crate::stages::tool_dispatch`]; `turn_call_recorder`
//! scopes the platform's one `llm_usage` recorder to a turn, for the loop and
//! for every side call the turn makes. Both are deliberately fire-and-forget:
//! a failed write is logged, never propagated, because losing an accounting
//! row is strictly better than failing the athlete's turn over it.

use pierre_providers::ai_scope;
use std::sync::Arc;

use pierre_core::models::{AddMessageParams, TenantId};
use pierre_database::repositories::ChatRepository;
use pierre_llm::call_record::LlmCallRecorder;
use pierre_services::llm_usage_recorder::UsageRepoCallRecorder;
use pierre_tool_runtime::tool_execution as chat_tool_loop;
use pierre_tool_runtime::tool_loop_io::{ToolMessageRecorder, ToolRoundRecord};
use tracing::warn;

use crate::{ChatPipelineContext, TurnInput};

/// `call_type` of a completion that re-asks a reply which claimed its data
/// access was broken, with the fetched data attached — capability recovery
/// and its subject-routed arm alike, each attempt its own row.
pub(crate) const CAPABILITY_REASK_CALL_TYPE: &str = "reask_capability_recovery";

/// `call_type` of the fact check that lists a reply's unsupported claims
/// about a named group peer.
pub(crate) const PEER_CLAIM_VERIFIER_CALL_TYPE: &str = "peer_claim_verifier";

/// `call_type` of the one re-sample of a reply that broke the persona's
/// identity.
pub(crate) const IDENTITY_LEAK_REASK_CALL_TYPE: &str = "reask_identity_leak";

/// `call_type` of the rewrite that brings a strict persona's reply inside its
/// output-format contract.
pub(crate) const PERSONA_REWRITE_CALL_TYPE: &str = "reask_persona_conformance";

/// `call_type` of the re-ask that repairs visual blocks the schema refused.
pub(crate) const VIZ_REPAIR_CALL_TYPE: &str = "viz_repair";

/// The `llm_usage` recorder for one of `input`'s LLM calls, typed `call_type`.
///
/// Every row a turn writes shares its conversation tenant, athlete,
/// conversation and turn id, so the turn's cost is the sum of its rows and a
/// side call — a re-ask, the claim judge, a summary — is counted apart from the
/// reply by its type alone.
pub(crate) fn turn_call_recorder(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    call_type: &'static str,
) -> Arc<dyn LlmCallRecorder> {
    Arc::new(UsageRepoCallRecorder::new(
        Arc::clone(&ctx.repos.llm_usage),
        input.conversation_tenant_id.to_string(),
        input.user_id.clone(),
        Some(input.conversation_id.clone()),
        input.turn_id,
        call_type,
    ))
}

/// Persists each tool dispatch round as `chat_messages` rows.
pub struct ChatRepoToolMessageRecorder {
    chat: Arc<dyn ChatRepository>,
    conversation_id: String,
    user_id: String,
    tenant_id: TenantId,
}

impl ChatRepoToolMessageRecorder {
    /// Build a recorder scoped to a single conversation.
    #[must_use]
    pub fn new(
        chat: Arc<dyn ChatRepository>,
        conversation_id: String,
        user_id: String,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            chat,
            conversation_id,
            user_id,
            tenant_id,
        }
    }
}

impl ToolMessageRecorder for ChatRepoToolMessageRecorder {
    fn record(&self, record: ToolRoundRecord) {
        let chat = Arc::clone(&self.chat);
        let conversation_id = self.conversation_id.clone();
        let user_id = self.user_id.clone();
        let tenant_id = self.tenant_id;
        // Strip tool-call/tool-result scaffolding before persisting. The raw
        // `<tool_call>`/`<tool_result>` blocks are per-turn LLM plumbing, not
        // durable conversation content; persisting them verbatim lets a thread
        // accrete scaffolding the model later parrots (the read path strips them
        // on replay, so a stored block is dead weight anyway). A real preamble
        // ("Pulling your activities…") survives the strip and is still kept;
        // pure scaffolding reduces to empty and is skipped.
        let assistant_text = chat_tool_loop::strip_simulation_artifacts(&record.assistant_text);
        let tool_result_text = chat_tool_loop::strip_simulation_artifacts(&record.tool_result_text);
        tokio::spawn(async move {
            if !assistant_text.is_empty() {
                let params = AddMessageParams {
                    tenant_id,
                    conversation_id: &conversation_id,
                    user_id: &user_id,
                    role: "tool_call",
                    content: &assistant_text,
                    token_count: None,
                    finish_reason: None,
                    prompt_tokens: None,
                    model: None,
                    content_blocks: None,
                    transport_policy: ai_scope::derived_policy(),
                };
                if let Err(e) = chat.add_message(&params).await {
                    warn!("Failed to persist tool_call message: {e}");
                    return;
                }
            }
            if !tool_result_text.is_empty() {
                let params = AddMessageParams {
                    tenant_id,
                    conversation_id: &conversation_id,
                    user_id: &user_id,
                    role: "tool_result",
                    content: &tool_result_text,
                    token_count: None,
                    finish_reason: None,
                    prompt_tokens: None,
                    model: None,
                    content_blocks: None,
                    transport_policy: ai_scope::derived_policy(),
                };
                if let Err(e) = chat.add_message(&params).await {
                    warn!("Failed to persist tool_result message: {e}");
                }
            }
        });
    }
}
