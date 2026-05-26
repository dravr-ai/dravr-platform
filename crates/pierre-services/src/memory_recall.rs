// ABOUTME: Tier 2 semantic user memory — recall top-k facts and render them into a prompt block
// ABOUTME: Complements memory_extraction.rs (write path) with the read path used during prompt assembly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Memory Recall
//!
//! Pulls the most relevant stored [`pierre_memory::UserFact`] records for a
//! user + coach pair and formats them into a short block that the chat
//! orchestrator injects into the system prompt. Today the recall query is
//! a simple `updated_at DESC LIMIT k` scan — later tiers will add embedding
//! similarity on top of the same query.

use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::tokens::estimate_prompt_tokens;
use pierre_database::repositories::HarnessMemoryRepository;
use pierre_memory::{FactKind, UserFact};

/// How many facts we fetch at most before rendering.
const DEFAULT_RECALL_LIMIT: i64 = 12;

/// Target budget for the rendered memory block (soft cap).
const DEFAULT_TOKEN_BUDGET: u32 = 500;

/// Parameters for a memory-context build.
pub struct RecallRequest<'a> {
    /// Tenant owning the conversation.
    pub tenant_id: TenantId,
    /// User whose facts we want.
    pub user_id: &'a str,
    /// Optional coach filter. When `Some`, we prioritize facts recorded
    /// against this coach; when `None`, we pull user-wide facts.
    pub coach_id: Option<&'a str>,
    /// Maximum facts to fetch. Defaults to [`DEFAULT_RECALL_LIMIT`] if unset.
    pub limit: Option<i64>,
    /// Soft token budget for the rendered block. Defaults to
    /// [`DEFAULT_TOKEN_BUDGET`] if unset.
    pub token_budget: Option<u32>,
}

/// Fetch stored facts and return a prompt-ready block, plus the underlying
/// records for tests / observability.
///
/// Returns `Ok(None)` when the user has no facts yet — callers skip prompt
/// injection in that case.
///
/// # Errors
///
/// Returns the repository error if the fact list query fails.
pub async fn build_user_memory_context<R>(
    repo: &R,
    req: &RecallRequest<'_>,
) -> AppResult<Option<MemoryContext>>
where
    R: HarnessMemoryRepository + ?Sized,
{
    let limit = req.limit.unwrap_or(DEFAULT_RECALL_LIMIT);
    let budget = req.token_budget.unwrap_or(DEFAULT_TOKEN_BUDGET);

    let facts = repo
        .list_user_facts(req.tenant_id, req.user_id, req.coach_id, None, limit)
        .await?;

    if facts.is_empty() {
        return Ok(None);
    }

    let rendered = render_memory_block(&facts, budget);
    Ok(Some(MemoryContext {
        block: rendered,
        facts,
    }))
}

/// A recalled memory context ready to be appended to the system prompt.
#[derive(Debug, Clone)]
pub struct MemoryContext {
    /// Prompt-ready text block. Empty-free: only returned when non-trivial.
    pub block: String,
    /// Underlying facts, preserved for telemetry and tests.
    pub facts: Vec<UserFact>,
}

/// Render a fact list into a short, deterministic prompt block.
fn render_memory_block(facts: &[UserFact], token_budget: u32) -> String {
    let mut out = String::from("\n\n## What you remember about this user\n\n");
    let header_tokens = estimate_prompt_tokens(&out);
    let mut used = header_tokens;

    for fact in facts {
        let line = format_fact_line(fact);
        let line_tokens = estimate_prompt_tokens(&line);
        if used + line_tokens > token_budget {
            out.push_str("- …additional facts truncated for budget\n");
            break;
        }
        out.push_str(&line);
        used += line_tokens;
    }

    out.push_str(
        "\nUse these facts to personalize guidance. Do not contradict them; \
         ask the user to confirm if something has changed.",
    );
    out
}

fn format_fact_line(fact: &UserFact) -> String {
    let kind = kind_label(fact.kind);
    format!(
        "- [{kind}] {subject} {predicate} {object} (confidence {confidence:.2})\n",
        subject = fact.subject,
        predicate = fact.predicate,
        object = fact.object,
        confidence = fact.confidence,
    )
}

const fn kind_label(kind: FactKind) -> &'static str {
    match kind {
        FactKind::Preference => "preference",
        FactKind::Physiology => "physiology",
        FactKind::Injury => "injury",
        FactKind::Goal => "goal",
        FactKind::Schedule => "schedule",
        FactKind::Equipment => "equipment",
        FactKind::Other => "other",
    }
}
