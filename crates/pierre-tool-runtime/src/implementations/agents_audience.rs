// ABOUTME: The coach-tool audience rule as the agent tools apply it to their caller
// ABOUTME: An athlete never lists, finds or activates an agent written for a coach

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use pierre_core::models::agents::Agent;
use pierre_core::models::TenantId;
use pierre_services::agents::{is_hidden_coach_tool, retain_visible_agents};

use crate::context::ToolExecutionContext;

/// `items` without the coach-facing agents the caller may not see — the same
/// rule as `GET /api/agents`; `agent_of` reads each item's agent.
pub(super) async fn visible_to_caller<T, F>(
    ctx: &ToolExecutionContext,
    mut items: Vec<T>,
    agent_of: F,
) -> Vec<T>
where
    T: Send,
    F: Fn(&T) -> &Agent + Send,
{
    let users = ctx.resources.data().repos().users.clone();
    retain_visible_agents(users.as_ref(), ctx.user_id, &mut items, agent_of).await;
    items
}

/// Activate `agent_id` for the caller. A coach-facing agent the caller may
/// not see reads as one that does not exist, as on every list.
///
/// # Errors
///
/// Returns the repository error if the lookup or the activation fails.
pub(super) async fn activate_visible_agent(
    ctx: &ToolExecutionContext,
    agent_id: &str,
    tenant_id: TenantId,
) -> AppResult<Option<Agent>> {
    let agents = ctx.resources.agents_manager();
    let users = ctx.resources.data().repos().users.clone();
    if is_hidden_coach_tool(agents, users.as_ref(), agent_id, ctx.user_id, tenant_id).await? {
        return Ok(None);
    }
    agents
        .activate_agent(agent_id, ctx.user_id, tenant_id)
        .await
}
