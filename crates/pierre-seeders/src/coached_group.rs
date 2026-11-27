// ABOUTME: Seeds a coaching group with its agent, owned by one account and coached by a human coach who holds no membership row
// ABOUTME: The one group shape the coach-facing seeders share; ids derived from what the group is for, so a rerun or a racing run reuses them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{AgentCategory, CreateAgentRequest, TenantId};
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
    /// How many members the group holds at most.
    pub max_members: i32,
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

/// The title of the agent [`seed_coached_group`] makes for the group `spec`
/// names.
fn agent_title(spec: &GroupSpec<'_>) -> String {
    format!("{} Agent", spec.name)
}

/// The ids of the group `spec` names that `owner` owns and `coach` coaches,
/// and of its agent.
///
/// Derived from those three alone (UUID v5 under the owner's id), never
/// drawn at random, so every run seeding that group writes the same rows: a
/// rerun after a run that failed part way finds what it wrote, and of two
/// runs racing each other one inserts and the other reads the winner's
/// (carnet#1511). Another coach of the same owner's group of that name gets
/// a group of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SeededGroupIds {
    /// The group's id.
    pub group: Uuid,
    /// Its agent's id.
    pub agent: Uuid,
}

impl SeededGroupIds {
    /// The ids for the group `spec` names, owned by `owner`, coached by
    /// `coach`.
    pub(crate) fn of(spec: &GroupSpec<'_>, owner: &Account, coach: &Account) -> Self {
        let derive = |what: &str| {
            Uuid::new_v5(
                &owner.id,
                format!(
                    "pierre-seeders/coached-group/{}/{}/{what}",
                    coach.id, spec.name
                )
                .as_bytes(),
            )
        };
        Self {
            group: derive("group"),
            agent: derive("agent"),
        }
    }
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
    let ids = SeededGroupIds::of(spec, owner, coach);
    group_agent(repos, spec, owner, ids).await?;
    seed_coached_group_with_agent(repos, spec, ids, owner, coach, members).await
}

/// The agent of the group `spec` names, made by `owner`, titled
/// [`agent_title`] and held under `ids.agent`: created unless an earlier or
/// a racing run created it.
///
/// # Errors
///
/// Returns the repository error from the agent write.
pub(crate) async fn group_agent(
    repos: &RepositoryRegistry,
    spec: &GroupSpec<'_>,
    owner: &Account,
    ids: SeededGroupIds,
) -> AppResult<()> {
    repos
        .agents
        .create_if_absent(
            ids.agent,
            owner.id,
            owner.tenant,
            &CreateAgentRequest {
                title: agent_title(spec),
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
        .await?;
    Ok(())
}

/// The group `spec` names under `ids.group`, run by the agent `ids.agent`,
/// as [`seed_coached_group`] seeds it: created with its coach in one write
/// unless an earlier or a racing run created it, then its coach attached
/// again if it was taken off (no coach), and `members` in it.
///
/// A group under that id whose coach is now another account is refused
/// before any member is added: its members would share their training with
/// that account (carnet#1511 design call 9). The coach is attached again
/// only by a write conditional on the group being active with no coach, so
/// a coach a racing writer (a coach invite redeemed meanwhile) attached
/// first is never overwritten: that write matching nothing is refused.
///
/// # Errors
///
/// Returns a configuration error when the group is archived or coached by
/// another account, or when the conditional attach matched no group, and
/// the repository error from the group, coach or member writes.
pub(crate) async fn seed_coached_group_with_agent(
    repos: &RepositoryRegistry,
    spec: &GroupSpec<'_>,
    ids: SeededGroupIds,
    owner: &Account,
    coach: &Account,
    members: &[(&Account, GroupRole)],
) -> AppResult<CoachingGroup> {
    let now = Timestamp::now();
    repos
        .groups
        .create_group_if_absent(
            owner.tenant,
            &CoachingGroup {
                id: ids.group,
                tenant_id: owner.tenant.to_string(),
                name: spec.name.to_owned(),
                description: Some(spec.description.to_owned()),
                agent_id: ids.agent.to_string(),
                owner_id: owner.id,
                coach_user_id: Some(coach.id),
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: spec.max_members,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                chat_connection: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await?;
    let group_id = ids.group.to_string();
    let group = repos
        .groups
        .get_group(&group_id, owner.tenant)
        .await?
        .ok_or_else(|| AppError::internal(format!("{} not found after creation", spec.name)))?;
    if !group.is_active {
        return Err(AppError::config(format!(
            "{} was archived under the id this seeder gives it, and a seeder never reactivates \
             a group; nothing more was written. Restore the group, or seed with another coach \
             account",
            spec.name
        )));
    }
    if group.coach_user_id.is_some_and(|other| other != coach.id) {
        return Err(AppError::config(format!(
            "{} is coached by another account under the id this seeder gives it, and its \
             members would share their training with that account; nothing more was written. \
             Give the group its coach back, or seed with another coach account",
            spec.name
        )));
    }
    let group = if group.coach_user_id.is_none() {
        if !repos
            .groups
            .attach_coach_to_coachless_group(&group_id, coach.id, owner.tenant)
            .await?
            && !attached_by_a_racing_run(repos, &group_id, owner.tenant, coach.id).await?
        {
            return Err(AppError::config(format!(
                "{}'s coach could not be given back: the group is no longer an active \
                 coachless group in its owner's workspace (archived, moved, or another coach \
                 was attached meanwhile). The group itself was not changed. Restore the \
                 group, or seed with another coach account",
                spec.name
            )));
        }
        CoachingGroup {
            coach_user_id: Some(coach.id),
            ..group
        }
    } else {
        group
    };
    for (account, role) in members {
        add_member(repos, group.id, account, *role, true).await?;
    }
    Ok(group)
}

/// Add `account` to the group with `role`, sharing their data with its coach
/// or not. The membership is dated now, as every join is. A live membership
/// a racing run wrote first is kept as it is: the group holds one row per
/// member.
///
/// # Errors
///
/// Returns the repository error from the member write, unless it failed on a
/// live membership that is there now.
pub(crate) async fn add_member(
    repos: &RepositoryRegistry,
    group_id: Uuid,
    account: &Account,
    role: GroupRole,
    shares_with_coach: bool,
) -> AppResult<()> {
    let now = Timestamp::now();
    let added = repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id: account.id,
            tenant_id: account.tenant.to_string(),
            role,
            peer_sharing_consent: false,
            coach_sharing_consent: shares_with_coach,
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await;
    match added {
        Ok(_) => Ok(()),
        Err(e) => {
            let joined = repos
                .groups
                .get_member(&group_id.to_string(), account.id)
                .await?;
            if joined.is_some() {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

/// Whether the attach that matched nothing lost to a run attaching the same
/// `coach`: two runs racing to give one coachless group its coach both read
/// it coachless, and the loser's conditional write matches no row. The
/// group then holds this coach, active, and the run converges on it.
async fn attached_by_a_racing_run(
    repos: &RepositoryRegistry,
    group_id: &str,
    tenant: TenantId,
    coach: Uuid,
) -> AppResult<bool> {
    Ok(repos
        .groups
        .get_group(group_id, tenant)
        .await?
        .is_some_and(|group| group.is_active && group.coach_user_id == Some(coach)))
}
