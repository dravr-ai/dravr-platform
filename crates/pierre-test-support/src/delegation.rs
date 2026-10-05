// ABOUTME: Coaching-group fixtures a delegated link rests on: the group's agent, a coached group, an active member
// ABOUTME: Shared by the tests that seed a coach's confirmed link, so each one builds the same rows a deployment holds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{AgentCategory, CreateAgentRequest, TenantId};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Create the agent persona a group runs, owned by `owner`, and return its id.
///
/// # Errors
///
/// Returns an error if the agent cannot be stored.
pub async fn create_group_agent(
    repos: &RepositoryRegistry,
    owner: Uuid,
    owner_tenant: TenantId,
) -> AppResult<String> {
    let agent = repos
        .agents
        .create(
            owner,
            owner_tenant,
            &CreateAgentRequest {
                title: "Group Agent".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
                category: AgentCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await?;
    Ok(agent.id.to_string())
}

/// Create an active group `owner` runs on `agent_id`, coached by `coach`, and
/// return its id.
///
/// # Errors
///
/// Returns an error if the group cannot be stored or its coach not set.
pub async fn create_coached_group(
    repos: &RepositoryRegistry,
    owner: Uuid,
    owner_tenant: TenantId,
    agent_id: &str,
    name: &str,
    coach: Uuid,
) -> AppResult<Uuid> {
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner_tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner_tenant.to_string(),
                name: name.to_owned(),
                description: None,
                agent_id: agent_id.to_owned(),
                owner_id: owner,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
    let coach_set = repos
        .groups
        .set_group_coach_user(&group.id.to_string(), Some(coach), owner_tenant)
        .await?;
    if !coach_set {
        return Err(AppError::internal(format!(
            "test fixture: group {} was created but its coach was not set",
            group.id
        )));
    }
    Ok(group.id)
}

/// Add `user`, in their own `tenant`, as an active member of `group`, sharing
/// with the group's coach as joining grants (ADR-002) and not with peers.
///
/// # Errors
///
/// Returns an error if the membership cannot be stored.
pub async fn add_group_member(
    repos: &RepositoryRegistry,
    group: Uuid,
    user: Uuid,
    tenant: TenantId,
) -> AppResult<()> {
    let now = Utc::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group,
            user_id: user,
            tenant_id: tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await?;
    Ok(())
}
