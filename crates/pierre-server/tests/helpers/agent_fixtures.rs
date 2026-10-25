// ABOUTME: Catalogue-agent fixtures — publish an agent so it owns a @handle, then install it for an athlete
// ABOUTME: Shared by the @handle mention and /agent add @handle tests so "installed" means one thing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]

use pierre_core::models::agents::{
    Agent, AgentCategory, AgentVisibility, CreateAgentRequest, CreateSystemAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Create a system agent carrying `system_prompt` and take it through review
/// to a published listing — the moment it is assigned its catalogue `@handle`,
/// derived from `title` ("Recovery Coach" → `recovery-agent`).
pub(crate) async fn publish_catalogue_agent(
    repos: &RepositoryRegistry,
    author_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    system_prompt: &str,
) -> Uuid {
    publish_catalogue_agent_in(
        repos,
        author_id,
        tenant_id,
        title,
        system_prompt,
        AgentCategory::Training,
    )
    .await
}

/// [`publish_catalogue_agent`] filed under a specific Store category, for
/// tests that browse one shelf of the catalogue.
pub(crate) async fn publish_catalogue_agent_in(
    repos: &RepositoryRegistry,
    author_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    system_prompt: &str,
    category: AgentCategory,
) -> Uuid {
    publish_catalogue_agent_tagged(
        repos,
        author_id,
        tenant_id,
        title,
        system_prompt,
        category,
        vec!["test".to_owned()],
    )
    .await
}

/// [`publish_catalogue_agent_in`] carrying its own canonical tag list, for
/// tests about what the Store's chips say and what its search matches.
pub(crate) async fn publish_catalogue_agent_tagged(
    repos: &RepositoryRegistry,
    author_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    system_prompt: &str,
    category: AgentCategory,
    tags: Vec<String>,
) -> Uuid {
    let agent = repos
        .agents
        .create_system_agent(
            author_id,
            tenant_id,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: Some(format!("Description for {title}")),
                system_prompt: system_prompt.to_owned(),
                category,
                tags,
                visibility: AgentVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap();
    let id = agent.id.to_string();
    repos
        .store_listings
        .submit_for_review(&id, author_id, tenant_id)
        .await
        .unwrap();
    repos
        .store_listings
        .approve_agent(&id, tenant_id, Some(author_id))
        .await
        .unwrap();
    agent.id
}

/// Install a published agent on `user_id`'s team and return what joined it.
///
/// A catalogue (system) agent — every agent [`publish_catalogue_agent`]
/// makes — joins as itself: the returned agent is the origin, its own id and
/// handle, which `AgentsRepository::find_installed_by_handle` now resolves
/// for this athlete. Only another tenant's user-published agent comes back as
/// a copy (`forked_from` = the origin).
pub(crate) async fn install_catalogue_agent(
    repos: &RepositoryRegistry,
    origin: Uuid,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Agent {
    repos
        .store_listings
        .install_from_store(&origin.to_string(), user_id, tenant_id)
        .await
        .unwrap()
}

/// Give `user_id` an agent of their own and select it, so the threads they
/// open bind that agent rather than the catalogue default.
///
/// Signup leaves every account holding a selection; without one, a thread
/// falls back to the newest athlete-facing catalogue agent and records usage
/// on it. Usage no longer puts an agent on the team, but tests about
/// installing a catalogue agent still pin the thread here, so the agent under
/// test is touched by the install alone.
pub(crate) async fn select_own_agent(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    let request: CreateAgentRequest = serde_json::from_value(serde_json::json!({
        "title": "Own Agent",
        "system_prompt": "You are the athlete's own agent.",
        "category": "training",
        "tags": ["own"],
    }))
    .unwrap();
    let agent = repos
        .agents
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    repos
        .tenants
        .set_selected_agent(tenant_id, user_id, Some(&agent.id.to_string()))
        .await
        .unwrap();
}
