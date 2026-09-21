// ABOUTME: Tier 1 context-window compaction stage — summarizes long conversations in place
// ABOUTME: Provides apply_tier1_compaction — runs before LLM dispatch when token budget tips over
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 1 context-window compaction.
//!
//! When the assembled LLM message list approaches the active model's
//! context window, this stage summarizes the oldest portion of the
//! conversation in place so subsequent turns do not exceed the window.
//! Delegates to [`crate::services::conversation_compaction`] for the
//! actual summarization logic; this stage is the harness that decides
//! when to invoke it.

use pierre_core::models::TenantId;
use pierre_llm::{ChatMessage, ChatProvider};
use pierre_services::conversation_compaction::{
    CompactionContext, CompactionOutcome, ConversationCompactor,
};

use crate::ChatPipelineContext;

/// Run conversation compaction in place. Failures log and continue — a
/// failed compaction never blocks a turn.
pub async fn apply_tier1_compaction(
    ctx: &ChatPipelineContext,
    provider: &ChatProvider,
    tenant_id: TenantId,
    conversation_id: &str,
    source_ids: &[Option<String>],
    llm_messages: &mut Vec<ChatMessage>,
) {
    // Read the active compaction tunables from the harness config registry
    // so admin updates via `PUT /admin/settings/harness` apply on the next
    // turn without a server restart.
    let compactor = ConversationCompactor::new(ctx.harness_config_registry.current_compaction());
    let summary_prompt = ctx.prompt_registry.conversation_summary_prompt();
    let compaction = CompactionContext {
        repo: ctx.repos.memory.as_ref(),
        provider,
        summary_prompt: summary_prompt.trim(),
        tenant_id,
        conversation_id,
        source_ids,
        llm_messages,
    };
    match compactor.compact_if_needed(compaction).await {
        Ok(outcome) => {
            if !matches!(outcome, CompactionOutcome::NoOp { .. }) {
                tracing::debug!(?outcome, "compaction applied before dispatch");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "compaction failed; continuing with uncompacted prompt");
        }
    }
}
