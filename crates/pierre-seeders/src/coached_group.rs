// ABOUTME: Seeds a coaching group with its agent, owned by one account and coached by a human coach who holds no membership row
// ABOUTME: The one group shape the coach-facing seeders share, and the rerun check that makes each of them a no-op the second time
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{AgentCategory, CreateAgentRequest};
use pierre_database::RepositoryRegistry;
use pierre_middleware::mask_email;
use tracing::info;
use uuid::Uuid;

use crate::accounts::Account;

/// What a seeded group is called and what its agent is told.
pub(crate) struct GroupSpec<'a> {
    /// The group's name, which its threads are titled after.
    pub name: &'a str,
    /// The group's description.
    pub description: &'a str,
    /// The description of the group's agent.
    pub agent_description: &'a str,
    /// The system prompt of the group's agent.
    pub agent_prompt: &'a str,
}

/// Whether `coach` already coaches a group named `name`, which makes a
/// seeder's rerun a no-op.
///
/// # Errors
///
/// Returns the repository error from the coached-groups read.
pub(crate) async fn coaches_group_named(
    repos: &RepositoryRegistry,
    coach: &Account,
    name: &str,
) -> AppResult<bool> {
    let coached = repos.groups.list_groups_coached_by(coach.id).await?;
    let already = coached.iter().any(|group| group.name == name);
    if already {
        info!(
            "{} already coaches {name}; nothing to seed",
            mask_email(&coach.email)
        );
    }
    Ok(already)
}

/// The group `spec` names: its agent made by `owner`, the group owned by
/// `owner` and coached by `coach`, and `members` in it with their roles, each
/// sharing their data with the coach.
///
/// The coach is attached as a coach invite attaches one, through the group's
/// `coach_user_id`, and holds no membership row unless `members` names them.
///
/// # Errors
///
/// Returns the repository error from the agent, group, coach or member writes.
pub(crate) async fn seed_coached_group(
    repos: &RepositoryRegistry,
    spec: &GroupSpec<'_>,
    owner: &Account,
    coach: &Account,
    members: &[(&Account, GroupRole)],
) -> AppResult<CoachingGroup> {
    let agent_id = repos
        .agents
        .create(
            owner.id,
            owner.tenant,
            &CreateAgentRequest {
                title: format!("{} Agent", spec.name),
                description: Some(spec.agent_description.to_owned()),
                system_prompt: spec.agent_prompt.to_owned(),
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
        .await?
        .id
        .to_string();

    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: spec.name.to_owned(),
                description: Some(spec.description.to_owned()),
                agent_id,
                owner_id: owner.id,
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
    repos
        .groups
        .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
        .await?;
    for (account, role) in members {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: account.id,
                tenant_id: account.tenant.to_string(),
                role: *role,
                peer_sharing_consent: false,
                coach_sharing_consent: true,
                coach_sharing_consent_at: None,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await?;
    }
    Ok(CoachingGroup {
        coach_user_id: Some(coach.id),
        ..group
    })
}
