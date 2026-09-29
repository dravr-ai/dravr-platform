// ABOUTME: Who writes to the athlete's one season plan — the turn's agent — and who wrote what is already there
// ABOUTME: Resolves the writing agent, names a season's authors for get_training_plan, and words the refusal over another agent's outline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan authorship
//!
//! The athlete has one active season plan per tenant, and every agent they
//! use reads it and adds weeks to it. What differs between agents is
//! authorship: the outline records the agent that laid the season, each week
//! the agent that wrote it. This module answers the three questions that
//! follow from that:
//!
//! - **Who is writing.** The agent the turn answers as ([`resolve_turn_agent`]):
//!   the mentioned agent on a `@handle` turn, the room's agent in a room,
//!   otherwise the conversation's; the `agent_id` argument only on a direct
//!   MCP call that carries no conversation.
//! - **Who wrote what is there.** [`plan_authors`] names the outline's author
//!   and every week author in the reply of `get_training_plan`, and marks
//!   which one is the reader.
//! - **Why an outline was refused.** A season another agent laid is not
//!   re-laid by accident: [`season_held_refusal`] says who laid it and how to
//!   proceed — weeks without an outline, or `replace_season` when the athlete
//!   asked for the change.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use uuid::Uuid;

use super::plan_scope::PlanWriter;
use super::training_plans_output::PlanAuthorView;
use crate::context::ToolExecutionContext;

/// The originating conversation, read under the requester's own tenant.
///
/// `None` when the call carries no conversation (a direct MCP call) and when
/// the requester's tenant does not own the row — a shared room files its
/// conversation under the bot tenant, which is how the plan scope tells a
/// room from a direct chat.
///
/// # Errors
///
/// Returns the repository error from the conversation lookup.
pub(super) async fn load_conversation(
    repos: &RepositoryRegistry,
    conversation_id: Option<&str>,
    tenant: TenantId,
    user_id: &str,
) -> AppResult<Option<ConversationRecord>> {
    match conversation_id {
        Some(conv_id) => repos.chat.get_conversation(conv_id, user_id, tenant).await,
        None => Ok(None),
    }
}

/// The agent a plan write is made by.
///
/// The turn's agent wins: the chat pipeline binds the agent the turn answers
/// as — the mentioned agent on a `@handle` turn, the room's agent in a room —
/// so a week is authored by the agent the athlete was talking to. Without a
/// bound turn agent, a conversation that loaded decides, including when it
/// binds no agent (`None`, never the argument). The LLM-supplied `agent_id`
/// counts only when there is no conversation at all: a direct MCP call.
#[must_use]
pub(super) fn resolve_turn_agent(
    context: &ToolExecutionContext,
    conversation: Option<&ConversationRecord>,
    arg_agent: Option<String>,
) -> Option<String> {
    if let Some(agent_id) = context.turn_agent_id.clone() {
        return Some(agent_id);
    }
    conversation.map_or(arg_agent, |conv| conv.agent_id.clone())
}

/// The title of agent `agent_id` as `user_id` in `tenant` may see it — their
/// own agent, one installed for them, or a system agent — or `None` when that
/// reader cannot see it (another tenant's custom agent, or a retired one).
///
/// # Errors
///
/// Returns the repository error from the agent lookup.
pub(super) async fn agent_title(
    repos: &RepositoryRegistry,
    agent_id: &str,
    tenant: TenantId,
    user_id: Uuid,
) -> AppResult<Option<String>> {
    Ok(repos
        .agents
        .get_by_id(agent_id, user_id, tenant)
        .await?
        .map(|agent| agent.title))
}

/// The refusal for an outline save over a season another agent laid, or over
/// an agent's season by a writer with no agent, when the caller did not set
/// `replace_season`. Nothing was written when it is returned.
///
/// `author_title` is the season author's title as the writer sees it;
/// `acting_for` the athlete's roster name when a coach saves for them.
#[must_use]
pub(super) fn season_held_refusal(
    author_title: Option<&str>,
    acting_for: Option<&str>,
) -> AppError {
    let author = author_title.unwrap_or("Another agent");
    let whose = acting_for.map_or_else(|| "the athlete's".to_owned(), |name| format!("{name}'s"));
    AppError::invalid_input(format!(
        "{author} laid out {whose} season, so this save changed nothing. Save the weeks you \
         change without an outline — they attach to that season. Send an outline only when the \
         athlete asked you to re-lay or change that season, and set replace_season to true."
    ))
}

/// Every agent that wrote the season as returned: the outline's author first,
/// then each distinct week author in calendar order, marked with whether it is
/// the reader and whether it laid the season. An outline or week no agent
/// wrote names no one.
///
/// Titles are resolved as the reader (`writer`), so an agent the reader cannot
/// see is named by id with a null `name`.
///
/// # Errors
///
/// Returns the repository error from an agent lookup.
pub(super) async fn plan_authors(
    repos: &RepositoryRegistry,
    plan: &TrainingPlan,
    weeks: &[PlanWeek],
    writer: &PlanWriter,
) -> AppResult<Vec<PlanAuthorView>> {
    let mut ids: Vec<&str> = Vec::new();
    for agent_id in plan.author_agent_id.as_deref().into_iter().chain(
        weeks
            .iter()
            .filter_map(|week| week.author_agent_id.as_deref()),
    ) {
        if !agent_id.is_empty() && !ids.contains(&agent_id) {
            ids.push(agent_id);
        }
    }
    let mut authors = Vec::with_capacity(ids.len());
    for agent_id in ids {
        authors.push(PlanAuthorView {
            agent_id: agent_id.to_owned(),
            name: agent_title(repos, agent_id, writer.tenant, writer.user_id).await?,
            is_you: writer.agent_id.as_deref() == Some(agent_id),
            laid_the_season: plan.author_agent_id.as_deref() == Some(agent_id),
        });
    }
    Ok(authors)
}
