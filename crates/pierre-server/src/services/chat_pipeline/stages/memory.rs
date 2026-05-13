// ABOUTME: Tier 2 memory recall stage — injects stored user facts into the system prompt
// ABOUTME: Extracted from services/chat_orchestration.rs::inject_memory_recall (2026-04-16)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 2 memory recall.
//!
//! Retrieves previously-extracted user facts for the current (user, coach)
//! pair and formats them as a short system-prompt section, so the coach can
//! reference long-term context it has accumulated across prior turns
//! (preferences, constraints, stated goals) without needing to re-derive
//! them from the conversation history.
//!
//! Complementary to [`crate::services::memory_extraction`], which runs
//! after the turn completes and distills new facts from the completed
//! exchange.

use pierre_database::repositories::HarnessMemoryRepository;

use crate::models::TenantId;
use crate::services::memory_recall::{build_user_memory_context, RecallRequest};

/// Append a recalled-memory block to the system prompt.
///
/// Targets the given (tenant, user, coach) triple. Errors and missing
/// memories both pass through silently — recall is a best-effort
/// enhancement, not a hard dependency of the dispatch path.
pub async fn inject_memory_recall(
    memory_repo: &dyn HarnessMemoryRepository,
    tenant_id: TenantId,
    user_id: &str,
    coach_id: Option<&str>,
    base_prompt: String,
) -> String {
    let request = RecallRequest {
        tenant_id,
        user_id,
        coach_id,
        limit: None,
        token_budget: None,
    };
    match build_user_memory_context(memory_repo, &request).await {
        Ok(Some(ctx)) => format!("{base_prompt}{}", ctx.block),
        Ok(None) => base_prompt,
        Err(e) => {
            tracing::warn!(error = %e, "memory recall failed; continuing without user facts");
            base_prompt
        }
    }
}
