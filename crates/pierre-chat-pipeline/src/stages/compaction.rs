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

use std::collections::HashMap;

use pierre_core::errors::AppResult;
use pierre_database::database::MessageRecord;
use pierre_llm::stage::LlmStage;
use pierre_llm::{ChatMessage, ChatProvider};
use pierre_services::conversation_compaction::{
    CompactionContext, CompactionOutcome, ConversationCompactor,
};

use crate::recorders::turn_call_recorder;
use crate::stages::prompt_builder::replayable_summary;
use crate::{ChatPipelineContext, TurnInput};

/// Run conversation compaction in place. Failures log and continue — a
/// failed compaction never blocks a turn.
///
/// The summary call is billed to `input`'s turn as a
/// [`LlmStage::CompactionSummary`] row.
pub async fn apply_tier1_compaction(
    ctx: &ChatPipelineContext,
    provider: &ChatProvider,
    input: &TurnInput,
    history: &[MessageRecord],
    source_ids: &[Option<String>],
    llm_messages: &mut Vec<ChatMessage>,
) {
    let source_consents = consents_by_row(history, source_ids);
    // Read the active compaction tunables from the harness config registry
    // so admin updates via `PUT /admin/settings/harness` apply on the next
    // turn without a server restart.
    let compactor = ConversationCompactor::new(ctx.harness_config_registry.current_compaction());
    let summary_prompt = ctx.prompt_registry.conversation_summary_prompt();
    let recorder = turn_call_recorder(ctx, input, LlmStage::CompactionSummary.call_type());
    let compaction = CompactionContext {
        repo: ctx.repos.memory.as_ref(),
        provider,
        summary_prompt: summary_prompt.trim(),
        recorder: Some(&recorder),
        tenant_id: input.conversation_tenant_id,
        conversation_id: &input.conversation_id,
        source_ids,
        source_consents: &source_consents,
        llm_messages,
        summary_for_prompt: replayable_summary,
    };
    log_outcome(compactor.compact_if_needed(compaction).await);
}

/// Log what the compactor did; a failure is logged and the turn continues.
fn log_outcome(result: AppResult<CompactionOutcome>) {
    match result {
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

/// The consent keys each emitted row names, aligned with `source_ids`
/// (carnet#1157).
fn consents_by_row(history: &[MessageRecord], source_ids: &[Option<String>]) -> Vec<Vec<String>> {
    let consents: HashMap<&str, &[String]> = history
        .iter()
        .map(|row| (row.id.as_str(), row.ai_consent_sources.as_slice()))
        .collect();
    source_ids
        .iter()
        .map(|id| {
            id.as_deref()
                .and_then(|id| consents.get(id))
                .map_or_else(Vec::new, |sources| sources.to_vec())
        })
        .collect()
}
