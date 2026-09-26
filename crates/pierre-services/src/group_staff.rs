// ABOUTME: Names a coaching group's AI agent and human coach for the person reading the group
// ABOUTME: One resolution behind the group REST view and the /group members roster
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Group staff
//!
//! A coaching group has two roles no membership row describes: the AI agent
//! that answers in its chat (`coaching_groups.agent_id`) and the human coach
//! attached to oversee it (`coaching_groups.coach_user_id`). The group row
//! holds only their ids, so every surface that tells a member who is who
//! resolves them here, once, the same way.
//!
//! The agent is read in the group's tenant, where its chat runs it — its own
//! agents and every system agent — and never in the reader's: members join
//! across tenants, and one who joined from elsewhere must read the same agent
//! its chat answers with. Its title is the `agent_translations` overlay for
//! the reader's locale, the title the store and the chat's introduction show
//! them. The coach is named by [`person_name`].

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use std::slice;

use crate::delegated_connections::person_name;

/// Who runs a coaching group besides its members, named for one reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupStaff {
    /// The AI agent's title in the reader's locale; `None` when the agent
    /// cannot be read in the group's tenant (it was deleted).
    pub agent_title: Option<String>,
    /// The AI agent's `@handle`, without the `@`; `None` when it has none or
    /// cannot be read.
    pub agent_handle: Option<String>,
    /// The human coach's display name, else their email; `None` when no coach
    /// is attached or the coach's account cannot be read.
    pub coach_display_name: Option<String>,
}

/// Name `group`'s AI agent and human coach for a reader of `locale`.
///
/// # Errors
///
/// Returns an error when the group's tenant id is malformed, or when the
/// agent, its translation overlay or the coach's account cannot be read.
pub async fn resolve_group_staff(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    locale: &str,
) -> AppResult<GroupStaff> {
    let tenant_id = TenantId::parse_str(&group.tenant_id)
        .map_err(|e| AppError::internal(format!("Invalid group tenant: {e}")))?;

    let mut agent = repos
        .agents
        .get_in_tenant(&group.agent_id, tenant_id)
        .await?;
    if let Some(agent) = agent.as_mut() {
        repos
            .agents
            .translate_agents(slice::from_mut(agent), locale)
            .await?;
    }

    let coach = match group.coach_user_id {
        Some(coach_id) => repos.users.get_global(coach_id).await?,
        None => None,
    };

    Ok(GroupStaff {
        agent_title: agent.as_ref().map(|agent| agent.title.clone()),
        agent_handle: agent.and_then(|agent| agent.handle),
        coach_display_name: coach.as_ref().map(person_name),
    })
}
