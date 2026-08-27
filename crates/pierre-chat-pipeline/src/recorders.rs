// ABOUTME: Sinks that persist LLM-call usage rows and tool-dispatch rounds for one turn
// ABOUTME: Each spawns its write so recording never blocks the turn it is observing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Turn recorders.
//!
//! Both implement a `chat_tool_loop` sink trait and are handed to the tool loop
//! by [`crate::stages::tool_dispatch`]. They are the pipeline's only write path
//! for per-call telemetry, and they are deliberately fire-and-forget: a failed
//! usage row is logged, never propagated, because losing an accounting row is
//! strictly better than failing the athlete's turn over it.

use std::sync::Arc;

use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{AddMessageParams, ConversationTurnId, TenantId};
use pierre_database::database::repositories::LlmUsageRepository;
use pierre_database::repositories::ChatRepository;
use pierre_llm::pricing::{TokenCounts, GLOBAL_PRICING_REGISTRY};
use pierre_tool_runtime::llm_call_record::{LlmCallRecord, LlmCallRecorder};
use pierre_tool_runtime::tool_execution as chat_tool_loop;
use tracing::{info, warn};

/// Per-call sink that persists one `llm_usage` row per LLM invocation.
pub struct UsageRepoCallRecorder {
    llm_usage: Arc<dyn LlmUsageRepository>,
    tenant_id: String,
    user_id: String,
    conversation_id: Option<String>,
    turn_id: ConversationTurnId,
    call_type: &'static str,
}

impl UsageRepoCallRecorder {
    /// Build a recorder scoped to a single turn.
    pub fn new(
        llm_usage: Arc<dyn LlmUsageRepository>,
        tenant_id: String,
        user_id: String,
        conversation_id: Option<String>,
        turn_id: ConversationTurnId,
        call_type: &'static str,
    ) -> Self {
        Self {
            llm_usage,
            tenant_id,
            user_id,
            conversation_id,
            turn_id,
            call_type,
        }
    }
}

impl LlmCallRecorder for UsageRepoCallRecorder {
    fn record(&self, record: LlmCallRecord) {
        let llm_usage = Arc::clone(&self.llm_usage);
        let tenant_id = self.tenant_id.clone();
        let user_id = self.user_id.clone();
        let conversation_id = self.conversation_id.clone();
        let turn_id = self.turn_id;
        let base_call_type = self.call_type;
        let call_sequence = record.call_sequence;
        let tenant_id_for_cost = self.tenant_id.clone();
        tokio::spawn(async move {
            let total_tokens = record.prompt_tokens + record.completion_tokens;
            let counts = TokenCounts::new(record.prompt_tokens, record.completion_tokens)
                .with_cache(record.cached_tokens, record.cached_write_tokens)
                .with_reasoning(record.reasoning_tokens);
            let cost_usd = GLOBAL_PRICING_REGISTRY.calculate_cost(
                Some(tenant_id_for_cost.as_str()),
                &record.provider,
                &record.model,
                &counts,
            );
            info!(
                target: "notify",
                event = "embacle.cost_usd",
                user_id = %user_id,
                tenant_id = %tenant_id,
                model = %record.model,
                cost_usd = cost_usd,
                "llm call cost"
            );
            let call_type_owned = if record.token_counts_estimated {
                format!("{base_call_type}_estimated")
            } else {
                base_call_type.to_owned()
            };
            let tool_calls_count = i64::try_from(record.tools_called.len()).unwrap_or(i64::MAX);
            let tools_called_json =
                serde_json::to_string(&record.tools_called).unwrap_or_else(|_| "[]".to_owned());
            let params = InsertLlmUsage {
                tenant_id: &tenant_id,
                user_id: &user_id,
                conversation_id: conversation_id.as_deref(),
                turn_id,
                provider: &record.provider,
                model: &record.model,
                prompt_tokens: record.prompt_tokens,
                completion_tokens: record.completion_tokens,
                total_tokens,
                cached_tokens: record.cached_tokens,
                cached_write_tokens: record.cached_write_tokens,
                reasoning_tokens: record.reasoning_tokens,
                call_type: &call_type_owned,
                tool_calls_count,
                tools_called: &tools_called_json,
                execution_time_ms: Some(record.latency_ms),
                cost_usd,
                call_sequence,
            };
            if let Err(e) = llm_usage.insert_llm_usage(&params).await {
                warn!("Failed to record per-LLM-call usage: {e}");
            }
        });
    }
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

impl chat_tool_loop::ToolMessageRecorder for ChatRepoToolMessageRecorder {
    fn record(&self, record: chat_tool_loop::ToolRoundRecord) {
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
                };
                if let Err(e) = chat.add_message(&params).await {
                    warn!("Failed to persist tool_result message: {e}");
                }
            }
        });
    }
}
