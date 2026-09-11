// ABOUTME: The shapes the agent tools answer with, and the schemas derived from them
// ABOUTME: Separate from agents.rs because that file is at its size ceiling and frozen
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Result types for the thirteen agent tools.
//!
//! These live beside `agents.rs` rather than inside it because that file is
//! past the 1200-line ceiling and frozen at its current size, exactly as
//! `goals_output` sits beside `goals.rs`. The answer shapes are a coherent
//! unit — the tests and the derived schemas both name them — and none of them
//! needs the tool plumbing next door.

use pierre_core::models::agents::{Agent, AgentListItem};
use serde::Serialize;

/// One agent as `list_agents` reports it.
///
/// A projection of the enriched list row, not the stored agent: it carries the
/// usage signals (`is_favorite`, `use_count`, `last_used_at`) that only the
/// list query joins in, and leaves out `system_prompt`, which is long and is
/// what `get_agent` is for.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AgentListEntry {
    /// Identifier the other agent tools take.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What the agent is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on: training, nutrition, recovery, recipes,
    /// mobility, analysis or custom.
    pub category: String,
    /// Free-form labels for filtering and search.
    pub tags: Vec<String>,
    /// Estimated size of the agent's system prompt, in tokens.
    pub token_count: u32,
    /// Whether the athlete has starred it.
    pub is_favorite: bool,
    /// Whether it ships with the platform rather than being athlete-authored.
    pub is_system: bool,
    /// Whether it is assigned to this athlete.
    pub is_assigned: bool,
    /// How many times it has been used.
    pub use_count: u32,
    /// RFC 3339 timestamp of the last use; absent if never used.
    pub last_used_at: Option<String>,
    /// RFC 3339 timestamp of the last edit.
    pub updated_at: String,
}

/// What `list_agents` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListAgentsResult {
    /// The agents on this page.
    pub agents: Vec<AgentListEntry>,
    /// How many came back on this page.
    pub count: usize,
    /// How many the athlete has in total, ignoring paging.
    pub total: u32,
    /// The paging offset these start at.
    pub offset: u32,
    /// The page size in force, 50 when the caller named none.
    pub limit: u32,
    /// Whether another page follows. Always false when the caller set no
    /// limit, because there is then nothing to compare the count against.
    pub has_more: bool,
}

/// What `create_agent` answers with.
///
/// Deliberately does not echo `system_prompt` or the `sample_prompts` the tool
/// accepts: the caller just sent them, and the prompt is long.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CreateAgentResult {
    /// Identifier of the new agent.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// What `get_agent` answers with.
///
/// Carries `system_prompt`, which the list projection omits — reading one
/// agent in full is what this tool is for.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetAgentResult {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// The instructions the agent runs on.
    pub system_prompt: String,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp of the last edit.
    pub updated_at: String,
}

/// What `update_agent` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct UpdateAgentResult {
    /// Identifier.
    pub id: String,
    /// Display name after the edit.
    pub title: String,
    /// What it is for; absent when none is set.
    pub description: Option<String>,
    /// The instructions after the edit.
    pub system_prompt: String,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt after the edit, in tokens.
    pub token_count: u32,
    /// RFC 3339 timestamp of this edit.
    pub updated_at: String,
}

/// What `delete_agent` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeleteAgentResult {
    /// Always true: the tool errors rather than reporting a failed delete.
    pub deleted: bool,
    /// The agent that was removed, echoed back.
    pub agent_id: String,
}

/// What `toggle_agent_favorite` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ToggleAgentFavoriteResult {
    /// The agent whose star was flipped.
    pub agent_id: String,
    /// Its state AFTER the flip, so a caller need not track the previous one.
    pub is_favorite: bool,
}

/// One agent as `search_agents` reports it.
///
/// Narrower than the list entry: a search result is for picking, so it omits
/// the usage signals and timestamps.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AgentSearchEntry {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
}

/// What `search_agents` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SearchAgentsResult {
    /// The query, echoed back.
    pub query: String,
    /// The matches on this page.
    pub results: Vec<AgentSearchEntry>,
    /// How many came back. Named `returned_count` rather than `count` here,
    /// unlike `list_agents` — kept as-is because renaming it would change a
    /// wire shape for no gain.
    pub returned_count: usize,
    /// The paging offset these start at.
    pub offset: u32,
    /// The page size in force, 20 when the caller named none.
    pub limit: u32,
    /// Whether another page follows.
    pub has_more: bool,
}

/// What `activate_agent` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActivateAgentResult {
    /// Identifier of the agent now active.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// The instructions it runs on.
    pub system_prompt: String,
    /// Which shelf it sits on.
    pub category: String,
    /// Always true: the tool errors rather than reporting a failed activation.
    pub is_active: bool,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
}

/// What `deactivate_agent` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeactivateAgentResult {
    /// Whether an agent was actually deactivated. False when none was active,
    /// which is a success rather than an error.
    pub deactivated: bool,
}

/// The active agent in full, as `get_active_agent` reports it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ActiveAgentDetail {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// The instructions it runs on.
    pub system_prompt: String,
    /// Which shelf it sits on.
    pub category: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Estimated size of the system prompt, in tokens.
    pub token_count: u32,
}

/// What `get_active_agent` answers with.
///
/// One shape for both answers rather than two: the tool sends the same key set
/// whether an agent is active or not, so `active` false pairs with `agent`
/// absent. A client reads one field to branch instead of probing for a key.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetActiveAgentResult {
    /// Whether any agent is active for this athlete.
    pub active: bool,
    /// The active agent; absent when `active` is false.
    pub agent: Option<ActiveAgentDetail>,
}

/// What `hide_agent` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HideAgentResult {
    /// The agent that was hidden.
    pub agent_id: String,
    /// Always true here; `show_agent` sends the same field as false.
    pub is_hidden: bool,
}

/// What `show_agent` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ShowAgentResult {
    /// The agent that was un-hidden.
    pub agent_id: String,
    /// Always false here; `hide_agent` sends the same field as true.
    pub is_hidden: bool,
    /// Whether a stored hide preference was actually removed. False when the
    /// agent was not hidden to begin with, which is a success.
    pub removed_preference: bool,
}

/// One agent as `list_hidden_agents` reports it.
///
/// The narrowest projection of the four: enough to recognise an agent and
/// un-hide it, and nothing else.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HiddenAgentEntry {
    /// Identifier `show_agent` takes.
    pub id: String,
    /// Display name.
    pub title: String,
    /// What it is for; absent when none was given.
    pub description: Option<String>,
    /// Which shelf it sits on.
    pub category: String,
    /// Whether it ships with the platform rather than being athlete-authored.
    pub is_system: bool,
}

/// What `list_hidden_agents` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ListHiddenAgentsResult {
    /// The hidden agents.
    pub agents: Vec<HiddenAgentEntry>,
    /// How many there are.
    pub count: usize,
}

// ============================================================================
// Payload builders
// ============================================================================
//
// The projections themselves, kept here beside the types they build rather
// than in `agents.rs`: each one is the answer to "what does this tool put on
// the wire", which is what this module is about.

/// Project the enriched list rows into the `list_agents` answer.
///
/// `has_more` can only be decided when the caller set a limit — a full page is
/// the only evidence of a next one — so an unbounded call reports false.
#[must_use]
pub fn list_agents_payload(
    agents: &[AgentListItem],
    total: u32,
    offset: Option<u32>,
    limit: Option<u32>,
) -> ListAgentsResult {
    let entries: Vec<AgentListEntry> = agents
        .iter()
        .map(|item| AgentListEntry {
            id: item.agent.id.to_string(),
            title: item.agent.title.clone(),
            description: item.agent.description.clone(),
            category: item.agent.category.as_str().to_owned(),
            tags: item.agent.tags.clone(),
            token_count: item.agent.token_count,
            is_favorite: item.is_favorite,
            is_system: item.agent.is_system,
            is_assigned: item.is_assigned,
            use_count: item.use_count,
            last_used_at: item.last_used_at.map(|dt| dt.to_rfc3339()),
            updated_at: item.agent.updated_at.to_rfc3339(),
        })
        .collect();
    let count = entries.len();
    ListAgentsResult {
        agents: entries,
        count,
        total,
        offset: offset.unwrap_or(0),
        limit: limit.unwrap_or(DEFAULT_LIST_LIMIT),
        has_more: limit.is_some_and(|l| count as u64 == u64::from(l)),
    }
}

/// The page size `list_agents` reports when the caller named none.
const DEFAULT_LIST_LIMIT: u32 = 50;

/// The page size `search_agents` reports when the caller named none.
const DEFAULT_SEARCH_LIMIT: u32 = 20;

/// Project a freshly created agent into the `create_agent` answer.
#[must_use]
pub fn create_agent_payload(agent: &Agent) -> CreateAgentResult {
    CreateAgentResult {
        id: agent.id.to_string(),
        title: agent.title.clone(),
        description: agent.description.clone(),
        category: agent.category.as_str().to_owned(),
        tags: agent.tags.clone(),
        token_count: agent.token_count,
        created_at: agent.created_at.to_rfc3339(),
    }
}

/// Project an agent into the `get_agent` answer.
#[must_use]
pub fn get_agent_payload(agent: &Agent) -> GetAgentResult {
    GetAgentResult {
        id: agent.id.to_string(),
        title: agent.title.clone(),
        description: agent.description.clone(),
        system_prompt: agent.system_prompt.clone(),
        category: agent.category.as_str().to_owned(),
        tags: agent.tags.clone(),
        token_count: agent.token_count,
        created_at: agent.created_at.to_rfc3339(),
        updated_at: agent.updated_at.to_rfc3339(),
    }
}

/// Project an edited agent into the `update_agent` answer.
#[must_use]
pub fn update_agent_payload(agent: &Agent) -> UpdateAgentResult {
    UpdateAgentResult {
        id: agent.id.to_string(),
        title: agent.title.clone(),
        description: agent.description.clone(),
        system_prompt: agent.system_prompt.clone(),
        category: agent.category.as_str().to_owned(),
        tags: agent.tags.clone(),
        token_count: agent.token_count,
        updated_at: agent.updated_at.to_rfc3339(),
    }
}

/// Project search hits into the `search_agents` answer.
#[must_use]
pub fn search_agents_payload(
    query: &str,
    agents: &[Agent],
    offset: Option<u32>,
    limit: Option<u32>,
) -> SearchAgentsResult {
    let results: Vec<AgentSearchEntry> = agents
        .iter()
        .map(|c| AgentSearchEntry {
            id: c.id.to_string(),
            title: c.title.clone(),
            description: c.description.clone(),
            category: c.category.as_str().to_owned(),
            tags: c.tags.clone(),
            token_count: c.token_count,
        })
        .collect();
    let returned_count = results.len();
    let limit = limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
    SearchAgentsResult {
        query: query.to_owned(),
        results,
        returned_count,
        offset: offset.unwrap_or(0),
        limit,
        has_more: returned_count as u64 == u64::from(limit),
    }
}

/// Project the now-active agent into the `activate_agent` answer.
#[must_use]
pub fn activate_agent_payload(agent: &Agent) -> ActivateAgentResult {
    ActivateAgentResult {
        id: agent.id.to_string(),
        title: agent.title.clone(),
        description: agent.description.clone(),
        system_prompt: agent.system_prompt.clone(),
        category: agent.category.as_str().to_owned(),
        is_active: true,
        token_count: agent.token_count,
    }
}

/// Project the active agent, or its absence, into the `get_active_agent`
/// answer.
///
/// Takes the `Option` rather than being called only on the `Some` arm so that
/// both answers are built in one place and cannot drift apart.
#[must_use]
pub fn active_agent_payload(agent: Option<&Agent>) -> GetActiveAgentResult {
    GetActiveAgentResult {
        active: agent.is_some(),
        agent: agent.map(|c| ActiveAgentDetail {
            id: c.id.to_string(),
            title: c.title.clone(),
            description: c.description.clone(),
            system_prompt: c.system_prompt.clone(),
            category: c.category.as_str().to_owned(),
            tags: c.tags.clone(),
            token_count: c.token_count,
        }),
    }
}

/// Project the hidden agents into the `list_hidden_agents` answer.
#[must_use]
pub fn list_hidden_agents_payload(agents: &[Agent]) -> ListHiddenAgentsResult {
    let entries: Vec<HiddenAgentEntry> = agents
        .iter()
        .map(|c| HiddenAgentEntry {
            id: c.id.to_string(),
            title: c.title.clone(),
            description: c.description.clone(),
            category: c.category.as_str().to_owned(),
            is_system: c.is_system,
        })
        .collect();
    let count = entries.len();
    ListHiddenAgentsResult {
        agents: entries,
        count,
    }
}
