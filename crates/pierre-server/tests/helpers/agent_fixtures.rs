// ABOUTME: Catalogue-agent fixtures — publish an agent so it owns a @handle, then install it for an athlete
// ABOUTME: Shared by the @handle mention and /agent add @handle tests so "installed" means one thing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]

use pierre_core::models::agents::{
    Agent, AgentCategory, AgentVisibility, CreateSystemAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Create a system agent carrying `system_prompt` and take it through review
/// to a published listing — the moment it is assigned its catalogue `@handle`,
/// derived from `title` ("Recovery Coach" → `recovery-agent`).
pub async fn publish_catalogue_agent(
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
pub async fn publish_catalogue_agent_in(
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
pub async fn publish_catalogue_agent_tagged(
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

/// Install a published agent on `user_id`'s agent list and return the copy.
///
/// The copy carries its origin's handle, which is what
/// `AgentsRepository::find_installed_by_handle` resolves for this athlete.
pub async fn install_catalogue_agent(
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
